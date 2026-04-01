use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub size: u64,
    pub modified_at: i64,
    pub file_type: String,
    pub content_hash: String,
}

#[derive(Clone)]
pub struct FileIndex {
    inner: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
}

impl FileIndex {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn scan_workspace(&self, path: impl AsRef<Path>) -> anyhow::Result<u64> {
        let workspace = path.as_ref().to_path_buf();
        let workspace_for_scan = workspace.clone();
        let files =
            tokio::task::spawn_blocking(move || scan_directory(&workspace_for_scan)).await??;
        let count = files.len() as u64;
        let mut map = self.inner.write().await;
        // Remove stale entries from this workspace before inserting fresh ones.
        map.retain(|k, _| !k.starts_with(&workspace));
        for meta in files {
            map.insert(meta.path.clone(), meta);
        }
        Ok(count)
    }

    pub async fn update_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref().to_path_buf();
        let meta = tokio::task::spawn_blocking(move || index_file(&path)).await??;
        let mut map = self.inner.write().await;
        map.insert(meta.path.clone(), meta);
        Ok(())
    }

    pub async fn remove_file(&self, path: impl AsRef<Path>) {
        let mut map = self.inner.write().await;
        map.remove(path.as_ref());
    }

    pub async fn search(
        &self,
        pattern: &str,
        workspace: Option<&Path>,
        file_type: Option<&str>,
    ) -> Vec<FileMetadata> {
        let map = self.inner.read().await;
        map.values()
            .filter(|meta| {
                let path_str = meta.path.to_string_lossy();
                // Trim leading separator so "**/*.rs" matches absolute paths like "/foo/bar.rs"
                let path_for_match = path_str.trim_start_matches('/');
                let matches_pattern = glob_match::glob_match(pattern, path_for_match);
                let matches_workspace = workspace
                    .map(|ws| meta.path.starts_with(ws))
                    .unwrap_or(true);
                let matches_type = file_type.map(|ft| meta.file_type == ft).unwrap_or(true);
                matches_pattern && matches_workspace && matches_type
            })
            .cloned()
            .collect()
    }

    pub async fn get(&self, path: impl AsRef<Path>) -> Option<FileMetadata> {
        let map = self.inner.read().await;
        map.get(path.as_ref()).cloned()
    }

    #[allow(dead_code)]
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }
}

impl Default for FileIndex {
    fn default() -> Self {
        Self::new()
    }
}

pub fn scan_directory(root: &Path) -> anyhow::Result<Vec<FileMetadata>> {
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root).require_git(false).build() {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            match index_file(path) {
                Ok(meta) => files.push(meta),
                Err(e) => {
                    tracing::warn!("Failed to index {}: {}", path.display(), e);
                }
            }
        }
    }
    Ok(files)
}

pub fn index_file(path: &Path) -> anyhow::Result<FileMetadata> {
    let metadata = std::fs::metadata(path)?;
    let size = metadata.len();
    let modified_at = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    let file_type = detect_file_type(path);
    let file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&file)?;
    let hash = hasher.finalize();
    let content_hash = hash.to_hex().to_string();
    Ok(FileMetadata {
        path: path.to_path_buf(),
        size,
        modified_at,
        file_type,
        content_hash,
    })
}

pub fn detect_file_type(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("jsx") => "javascript",
        Some("tsx") => "typescript",
        Some("go") => "go",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") => "cpp",
        Some("java") => "java",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("kt") | Some("kts") => "kotlin",
        Some("toml") => "toml",
        Some("yaml") | Some("yml") => "yaml",
        Some("json") => "json",
        Some("md") | Some("markdown") => "markdown",
        Some("sh") | Some("bash") => "shell",
        Some("html") | Some("htm") => "html",
        Some("css") => "css",
        Some("proto") => "protobuf",
        Some("sql") => "sql",
        Some("txt") => "text",
        _ => "unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn test_detect_file_type() {
        assert_eq!(detect_file_type(Path::new("foo.rs")), "rust");
        assert_eq!(detect_file_type(Path::new("foo.py")), "python");
        assert_eq!(detect_file_type(Path::new("foo.js")), "javascript");
        assert_eq!(detect_file_type(Path::new("foo.ts")), "typescript");
        assert_eq!(detect_file_type(Path::new("foo.go")), "go");
        assert_eq!(detect_file_type(Path::new("foo.c")), "c");
        assert_eq!(detect_file_type(Path::new("foo.cpp")), "cpp");
        assert_eq!(detect_file_type(Path::new("foo.toml")), "toml");
        assert_eq!(detect_file_type(Path::new("foo.json")), "json");
        assert_eq!(detect_file_type(Path::new("foo.md")), "markdown");
        assert_eq!(detect_file_type(Path::new("foo.proto")), "protobuf");
        assert_eq!(detect_file_type(Path::new("foo.xyz")), "unknown");
        assert_eq!(detect_file_type(Path::new("no_extension")), "unknown");
    }

    #[test]
    fn test_index_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("hello.rs");
        let content = b"fn main() {}";
        std::fs::write(&file_path, content).unwrap();

        let meta = index_file(&file_path).unwrap();

        assert_eq!(meta.path, file_path);
        assert_eq!(meta.size, content.len() as u64);
        assert_eq!(meta.file_type, "rust");
        assert!(!meta.content_hash.is_empty());
        assert!(meta.modified_at > 0);

        // Verify hash matches blake3
        let expected = blake3::hash(content).to_hex().to_string();
        assert_eq!(meta.content_hash, expected);
    }

    #[test]
    fn test_scan_directory_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();

        // Create .gitignore that ignores *.log files
        let gitignore_path = dir.path().join(".gitignore");
        let mut f = std::fs::File::create(&gitignore_path).unwrap();
        writeln!(f, "*.log").unwrap();
        drop(f);

        // Create a tracked file and an ignored file
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("debug.log"), "log output").unwrap();

        let files = scan_directory(dir.path()).unwrap();
        let paths: Vec<_> = files
            .iter()
            .map(|m| m.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(
            paths.contains(&"main.rs".to_string()),
            "main.rs should be indexed"
        );
        assert!(
            !paths.contains(&"debug.log".to_string()),
            "debug.log should be excluded by .gitignore"
        );
    }

    #[tokio::test]
    async fn test_file_index_scan_and_search() {
        // tempfile creates dirs under /tmp/.tmpXXX (hidden dir) which breaks glob_match
        // Use a non-hidden path under /tmp directly
        let base = std::env::temp_dir().join("creep_test_idx_scan");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        std::fs::write(base.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(base.join("lib.rs"), "pub fn foo() {}").unwrap();
        std::fs::write(base.join("config.toml"), "[section]").unwrap();

        let index = FileIndex::new();
        let count = index.scan_workspace(&base).await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(index.len().await, 3);

        // Search by glob pattern matching .rs files
        let rs_files = index.search("**/*.rs", None, None).await;
        assert_eq!(rs_files.len(), 2);

        // Search by file_type
        let toml_files = index.search("**/*", None, Some("toml")).await;
        assert_eq!(toml_files.len(), 1);
        assert_eq!(toml_files[0].file_type, "toml");

        // Search with workspace filter
        let ws_files = index.search("**/*", Some(&base), None).await;
        assert_eq!(ws_files.len(), 3);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn test_file_index_update_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("foo.rs");
        std::fs::write(&file_path, "fn foo() {}").unwrap();

        let index = FileIndex::new();
        index.update_file(&file_path).await.unwrap();
        assert_eq!(index.len().await, 1);

        let meta = index.get(&file_path).await.unwrap();
        assert_eq!(meta.file_type, "rust");

        // Update the file content
        std::fs::write(&file_path, "fn foo() { println!(\"updated\"); }").unwrap();
        index.update_file(&file_path).await.unwrap();
        assert_eq!(index.len().await, 1);

        let updated_meta = index.get(&file_path).await.unwrap();
        assert_ne!(updated_meta.content_hash, meta.content_hash);

        // Remove the file
        index.remove_file(&file_path).await;
        assert_eq!(index.len().await, 0);
        assert!(index.get(&file_path).await.is_none());
    }
}
