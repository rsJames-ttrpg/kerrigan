use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::index::detect_file_type;
use crate::parser::{Symbol, SymbolKind, is_language_supported, parse_symbols};

/// Reference to a symbol's location, stored in the inverted name index.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    pub file: PathBuf,
    pub line: u32,
    pub kind: SymbolKind,
}

/// In-memory symbol index with per-file storage and inverted name lookup.
#[derive(Clone)]
pub struct SymbolIndex {
    by_file: Arc<RwLock<HashMap<PathBuf, Vec<Symbol>>>>,
    by_name: Arc<RwLock<HashMap<String, Vec<SymbolRef>>>>,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self {
            by_file: Arc::new(RwLock::new(HashMap::new())),
            by_name: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Parse a file and update both indexes. Skips unsupported languages.
    /// Runs synchronously — call from spawn_blocking or sync context.
    pub fn reparse_file(&self, path: &Path) -> anyhow::Result<()> {
        let language = detect_file_type(path);
        if !is_language_supported(&language) {
            return Ok(());
        }

        let content = std::fs::read(path)?;
        let symbols = parse_symbols(&content, &language);

        let mut by_file = self.by_file.blocking_write();
        let mut by_name = self.by_name.blocking_write();

        // Remove old name index entries for this file.
        if let Some(old_symbols) = by_file.remove(path) {
            for sym in &old_symbols {
                if let Some(refs) = by_name.get_mut(&sym.name) {
                    refs.retain(|r| r.file != path);
                    if refs.is_empty() {
                        by_name.remove(&sym.name);
                    }
                }
            }
        }

        // Insert new name index entries.
        for sym in &symbols {
            by_name
                .entry(sym.name.clone())
                .or_default()
                .push(SymbolRef {
                    file: path.to_path_buf(),
                    line: sym.line,
                    kind: sym.kind.clone(),
                });
        }

        by_file.insert(path.to_path_buf(), symbols);
        Ok(())
    }

    /// Remove all symbols for a file from both indexes.
    pub async fn remove_file(&self, path: &Path) {
        let mut by_file = self.by_file.write().await;
        let mut by_name = self.by_name.write().await;

        if let Some(old_symbols) = by_file.remove(path) {
            for sym in &old_symbols {
                if let Some(refs) = by_name.get_mut(&sym.name) {
                    refs.retain(|r| r.file != path);
                    if refs.is_empty() {
                        by_name.remove(&sym.name);
                    }
                }
            }
        }
    }

    /// Search symbols by name (case-insensitive substring match).
    /// Optionally filter by kind and/or workspace.
    pub async fn search(
        &self,
        query: &str,
        kind: Option<&SymbolKind>,
        workspace: Option<&Path>,
    ) -> Vec<(Symbol, PathBuf)> {
        let by_file = self.by_file.read().await;
        let query_lower = query.to_lowercase();

        let mut results = Vec::new();
        for (path, symbols) in by_file.iter() {
            if let Some(ws) = workspace {
                if !path.starts_with(ws) {
                    continue;
                }
            }
            for sym in symbols {
                let name_matches =
                    query.is_empty() || sym.name.to_lowercase().contains(&query_lower);
                let kind_matches = kind.is_none() || kind == Some(&sym.kind);
                if name_matches && kind_matches {
                    results.push((sym.clone(), path.clone()));
                }
            }
        }
        results
    }

    /// List all symbols in a specific file, ordered by line number.
    pub async fn list_file_symbols(&self, path: &Path) -> Vec<Symbol> {
        let by_file = self.by_file.read().await;
        let mut symbols = by_file.get(path).cloned().unwrap_or_default();
        symbols.sort_by_key(|s| s.line);
        symbols
    }

    /// Parse all supported files in a directory. Returns total symbol count.
    /// Runs synchronously (call from spawn_blocking).
    pub fn scan_workspace(&self, root: &Path) -> anyhow::Result<u64> {
        let mut count = 0u64;
        for entry in ignore::WalkBuilder::new(root).require_git(false).build() {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && is_language_supported(&detect_file_type(path)) {
                if let Err(e) = self.reparse_file(path) {
                    tracing::warn!("failed to parse symbols in {}: {e}", path.display());
                    continue;
                }
                let by_file = self.by_file.blocking_read();
                if let Some(syms) = by_file.get(path) {
                    count += syms.len() as u64;
                }
            }
        }
        Ok(count)
    }
}

impl Default for SymbolIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn blocking_reparse(idx: &SymbolIndex, path: &Path) {
        let idx = idx.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || idx.reparse_file(&path).unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_reparse_and_list_file_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub fn hello() {} pub struct World;").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;

        let symbols = idx.list_file_symbols(&file).await;
        assert_eq!(symbols.len(), 2);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"World"));
    }

    #[tokio::test]
    async fn test_search_by_name_substring() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(
            &file,
            "fn process_events() {} fn process_files() {} fn unrelated() {}",
        )
        .unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;

        let results = idx.search("process", None, None).await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub struct FileIndex;").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;

        let results = idx.search("fileindex", None, None).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "FileIndex");
    }

    #[tokio::test]
    async fn test_search_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub fn foo() {} pub struct Bar;").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;

        let results = idx.search("", Some(&SymbolKind::Struct), None).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.name, "Bar");
    }

    #[tokio::test]
    async fn test_remove_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub fn hello() {}").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;
        assert_eq!(idx.list_file_symbols(&file).await.len(), 1);

        idx.remove_file(&file).await;
        assert_eq!(idx.list_file_symbols(&file).await.len(), 0);
        assert!(idx.search("hello", None, None).await.is_empty());
    }

    #[tokio::test]
    async fn test_reparse_replaces_old_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "fn old_fn() {}").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &file).await;
        assert_eq!(idx.search("old_fn", None, None).await.len(), 1);

        std::fs::write(&file, "fn new_fn() {}").unwrap();
        blocking_reparse(&idx, &file).await;

        assert!(idx.search("old_fn", None, None).await.is_empty());
        assert_eq!(idx.search("new_fn", None, None).await.len(), 1);
    }

    #[tokio::test]
    async fn test_scan_workspace() {
        let base = std::env::temp_dir().join("creep_sym_scan_test");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.rs"), "fn alpha() {}").unwrap();
        std::fs::write(base.join("b.rs"), "fn beta() {} struct Gamma;").unwrap();
        std::fs::write(base.join("c.py"), "def delta(): pass").unwrap();

        let idx = SymbolIndex::new();
        let idx2 = idx.clone();
        let base2 = base.clone();
        let count = tokio::task::spawn_blocking(move || idx2.scan_workspace(&base2).unwrap())
            .await
            .unwrap();
        assert_eq!(count, 3); // alpha, beta, Gamma (c.py unsupported)

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_search_with_workspace_filter() {
        let base_a = std::env::temp_dir().join("creep_sym_ws_a");
        let base_b = std::env::temp_dir().join("creep_sym_ws_b");
        let _ = std::fs::remove_dir_all(&base_a);
        let _ = std::fs::remove_dir_all(&base_b);
        std::fs::create_dir_all(&base_a).unwrap();
        std::fs::create_dir_all(&base_b).unwrap();
        std::fs::write(base_a.join("lib.rs"), "fn shared_name() {}").unwrap();
        std::fs::write(base_b.join("lib.rs"), "fn shared_name() {}").unwrap();

        let idx = SymbolIndex::new();
        blocking_reparse(&idx, &base_a.join("lib.rs")).await;
        blocking_reparse(&idx, &base_b.join("lib.rs")).await;

        let all = idx.search("shared_name", None, None).await;
        assert_eq!(all.len(), 2);

        let filtered = idx.search("shared_name", None, Some(&base_a)).await;
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].1.starts_with(&base_a));

        let _ = std::fs::remove_dir_all(&base_a);
        let _ = std::fs::remove_dir_all(&base_b);
    }
}
