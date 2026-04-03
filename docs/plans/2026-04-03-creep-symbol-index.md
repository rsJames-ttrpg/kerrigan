# Creep Symbol Index Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add tree-sitter-based Rust symbol extraction to Creep so drones can find definitions by name and get file outlines via `creep-cli symbols`.

**Architecture:** A new `parser.rs` module uses tree-sitter queries to extract 10 symbol kinds from Rust files. A new `SymbolIndex` (parallel to FileIndex) stores symbols per-file with an inverted name index. Two new gRPC RPCs (SearchSymbols, ListFileSymbols) serve queries. The watcher hooks symbol reparsing into file change events. The CLI gets a `symbols` subcommand.

**Tech Stack:** tree-sitter + tree-sitter-rust (parsing), tonic/prost (gRPC), Rust edition 2024, Buck2 (build)

**Spec:** `docs/specs/2026-04-03-creep-symbol-index-design.md`

---

## File Structure

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/creep/src/parser.rs` | tree-sitter grammar loading, S-expression query, symbol extraction |
| Create | `src/creep/src/symbol_index.rs` | In-memory symbol storage (by_file + by_name), search, scan |
| Modify | `src/creep/Cargo.toml` | Add tree-sitter + tree-sitter-rust deps |
| Modify | `src/creep/proto/creep.proto` | Add SymbolInfo, SearchSymbols, ListFileSymbols |
| Regen  | `src/creep/proto_gen/creep.v1.rs` | Regenerated from updated proto |
| Regen  | `src/creep-cli/proto_gen/creep.v1.rs` | Regenerated from updated proto (with serde) |
| Modify | `src/creep/src/main.rs` | Wire SymbolIndex into startup + process_events |
| Modify | `src/creep/src/service.rs` | Add SymbolIndex field, implement 2 new RPC handlers |
| Modify | `src/creep/src/watcher.rs` | Hook symbol reparse into process_events |
| Modify | `src/creep/src/config.rs` | Add symbol_index + languages config fields |
| Modify | `src/creep/BUCK` | Add tree-sitter deps |
| Modify | `src/creep-cli/src/main.rs` | Add Symbols subcommand |
| Modify | `src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md` | Document symbols commands |

---

### Task 1: Add tree-sitter dependencies

**Files:**
- Modify: `src/creep/Cargo.toml`

- [ ] **Step 1: Add tree-sitter and tree-sitter-rust**

```bash
cd src/creep && cargo add tree-sitter tree-sitter-rust
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/creep && cargo check`
Expected: compiles cleanly. tree-sitter's build.rs compiles C sources via cc crate.

- [ ] **Step 3: Commit**

```bash
git add src/creep/Cargo.toml Cargo.lock
git commit -m "feat(creep): add tree-sitter and tree-sitter-rust dependencies"
```

---

### Task 2: Create parser module — types and Rust query

**Files:**
- Create: `src/creep/src/parser.rs`
- Modify: `src/creep/src/main.rs:1-6` (add `mod parser;`)

- [ ] **Step 1: Write failing tests for symbol extraction**

Create `src/creep/src/parser.rs` with types, a stub `parse_symbols` that returns `todo!()`, and tests:

```rust
/// Kinds of symbols we extract from source files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Static,
    TypeAlias,
    Module,
    Macro,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Const => "const",
            Self::Static => "static",
            Self::TypeAlias => "type_alias",
            Self::Module => "module",
            Self::Macro => "macro",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(Self::Function),
            "struct" => Some(Self::Struct),
            "enum" => Some(Self::Enum),
            "trait" => Some(Self::Trait),
            "impl" => Some(Self::Impl),
            "const" => Some(Self::Const),
            "static" => Some(Self::Static),
            "type_alias" => Some(Self::TypeAlias),
            "module" => Some(Self::Module),
            "macro" => Some(Self::Macro),
            _ => None,
        }
    }
}

/// A symbol extracted from a source file.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub line: u32,
    pub end_line: u32,
    pub parent: Option<String>,
    pub signature: Option<String>,
}

/// Parse symbols from file content given a language identifier.
/// Returns empty vec for unsupported languages.
pub fn parse_symbols(_content: &[u8], _language: &str) -> Vec<Symbol> {
    todo!()
}

/// Returns true if the given language is supported for symbol parsing.
pub fn is_language_supported(language: &str) -> bool {
    matches!(language, "rust")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_function() {
        let src = b"fn hello() -> bool { true }";
        let symbols = parse_symbols(src, "rust");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].line, 0);
        assert!(symbols[0].signature.as_ref().unwrap().contains("fn hello"));
    }

    #[test]
    fn test_parse_rust_struct_and_impl_method() {
        let src = br#"
pub struct Foo {
    x: i32,
}

impl Foo {
    pub fn bar(&self) -> i32 {
        self.x
    }
}
"#;
        let symbols = parse_symbols(src, "rust");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"), "should find struct Foo: {names:?}");
        assert!(names.contains(&"bar"), "should find method bar: {names:?}");

        let foo = symbols.iter().find(|s| s.name == "Foo" && s.kind == SymbolKind::Struct).unwrap();
        assert_eq!(foo.kind, SymbolKind::Struct);

        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Function);
        assert_eq!(bar.parent.as_deref(), Some("Foo"));
    }

    #[test]
    fn test_parse_rust_enum_trait_const_static_type() {
        let src = br#"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
pub const MAX: u32 = 100;
pub static COUNTER: u32 = 0;
type Alias = Vec<String>;
"#;
        let symbols = parse_symbols(src, "rust");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Color"), "missing Color: {names:?}");
        assert!(names.contains(&"Drawable"), "missing Drawable: {names:?}");
        assert!(names.contains(&"MAX"), "missing MAX: {names:?}");
        assert!(names.contains(&"COUNTER"), "missing COUNTER: {names:?}");
        assert!(names.contains(&"Alias"), "missing Alias: {names:?}");

        let color = symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Enum);
        let drawable = symbols.iter().find(|s| s.name == "Drawable").unwrap();
        assert_eq!(drawable.kind, SymbolKind::Trait);
        let max = symbols.iter().find(|s| s.name == "MAX").unwrap();
        assert_eq!(max.kind, SymbolKind::Const);
        let counter = symbols.iter().find(|s| s.name == "COUNTER").unwrap();
        assert_eq!(counter.kind, SymbolKind::Static);
        let alias = symbols.iter().find(|s| s.name == "Alias").unwrap();
        assert_eq!(alias.kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_parse_rust_impl_block_as_symbol() {
        let src = br#"
struct Foo;
impl Foo {
    fn method(&self) {}
}
"#;
        let symbols = parse_symbols(src, "rust");
        let impl_sym = symbols.iter().find(|s| s.kind == SymbolKind::Impl).unwrap();
        assert_eq!(impl_sym.name, "Foo");
    }

    #[test]
    fn test_parse_rust_module() {
        let src = br#"
mod inner {
    fn nested() {}
}
"#;
        let symbols = parse_symbols(src, "rust");
        let inner = symbols.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!(inner.kind, SymbolKind::Module);
        let nested = symbols.iter().find(|s| s.name == "nested").unwrap();
        assert_eq!(nested.parent.as_deref(), Some("inner"));
    }

    #[test]
    fn test_parse_rust_macro() {
        let src = br#"
macro_rules! my_macro {
    () => {};
}
"#;
        let symbols = parse_symbols(src, "rust");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "my_macro");
        assert_eq!(symbols[0].kind, SymbolKind::Macro);
    }

    #[test]
    fn test_parse_unsupported_language() {
        let symbols = parse_symbols(b"def foo(): pass", "python");
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_function_signature_extraction() {
        let src = b"pub fn process(x: i32, y: &str) -> bool { true }";
        let symbols = parse_symbols(src, "rust");
        let sig = symbols[0].signature.as_ref().unwrap();
        assert!(sig.starts_with("fn process("), "sig was: {sig}");
        assert!(sig.contains("-> bool"), "sig was: {sig}");
    }

    #[test]
    fn test_is_language_supported() {
        assert!(is_language_supported("rust"));
        assert!(!is_language_supported("python"));
        assert!(!is_language_supported("unknown"));
    }
}
```

Add `mod parser;` to `src/creep/src/main.rs` after `mod index;`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/creep && cargo test parser`
Expected: FAIL — `todo!()` panics.

- [ ] **Step 3: Implement `parse_symbols` with tree-sitter queries**

Replace the `parse_symbols` stub in `src/creep/src/parser.rs`:

```rust
pub fn parse_symbols(content: &[u8], language: &str) -> Vec<Symbol> {
    match language {
        "rust" => parse_rust_symbols(content),
        _ => Vec::new(),
    }
}

/// Tree-sitter S-expression query for Rust symbol extraction.
const RUST_QUERY: &str = r#"
(function_item name: (identifier) @name) @definition
(struct_item name: (type_identifier) @name) @definition
(enum_item name: (type_identifier) @name) @definition
(trait_item name: (type_identifier) @name) @definition
(impl_item type: (_) @name) @definition
(const_item name: (identifier) @name) @definition
(static_item name: (identifier) @name) @definition
(type_item name: (type_identifier) @name) @definition
(mod_item name: (identifier) @name) @definition
(macro_definition name: (identifier) @name) @definition
"#;

fn parse_rust_symbols(content: &[u8]) -> Vec<Symbol> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("failed to set rust language");

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let query = tree_sitter::Query::new(&tree_sitter_rust::LANGUAGE.into(), RUST_QUERY)
        .expect("invalid rust query");
    let name_idx = query.capture_index_for_name("name").unwrap();
    let def_idx = query.capture_index_for_name("definition").unwrap();

    let mut cursor = tree_sitter::QueryCursor::new();
    let matches: Vec<_> = cursor.matches(&query, tree.root_node(), content).collect();

    let mut symbols = Vec::new();

    for m in &matches {
        let def_node = m.captures.iter().find(|c| c.index == def_idx).unwrap().node;
        let name_node = m.captures.iter().find(|c| c.index == name_idx).unwrap().node;

        let name = match name_node.utf8_text(content) {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        let kind = match def_node.kind() {
            "function_item" => SymbolKind::Function,
            "struct_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "impl_item" => SymbolKind::Impl,
            "const_item" => SymbolKind::Const,
            "static_item" => SymbolKind::Static,
            "type_item" => SymbolKind::TypeAlias,
            "mod_item" => SymbolKind::Module,
            "macro_definition" => SymbolKind::Macro,
            _ => continue,
        };

        let parent = find_parent_scope(def_node, content);

        let signature = if kind == SymbolKind::Function {
            Some(build_function_signature(def_node, content, &name))
        } else {
            None
        };

        symbols.push(Symbol {
            name,
            kind,
            line: def_node.start_position().row as u32,
            end_line: def_node.end_position().row as u32,
            parent,
            signature,
        });
    }

    symbols
}

/// Walk up the tree to find the enclosing impl or mod block.
fn find_parent_scope(node: tree_sitter::Node<'_>, content: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "impl_item" => {
                return parent
                    .child_by_field_name("type")
                    .and_then(|n| n.utf8_text(content).ok())
                    .map(String::from);
            }
            "mod_item" => {
                return parent
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(content).ok())
                    .map(String::from);
            }
            _ => current = parent.parent(),
        }
    }
    None
}

/// Build a short function signature: `fn name(params) -> ReturnType`
fn build_function_signature(
    node: tree_sitter::Node<'_>,
    content: &[u8],
    name: &str,
) -> String {
    let params = node
        .child_by_field_name("parameters")
        .and_then(|n| n.utf8_text(content).ok())
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("return_type")
        .and_then(|n| n.utf8_text(content).ok())
        .map(|r| format!(" -> {r}"))
        .unwrap_or_default();
    format!("fn {name}{params}{ret}")
}
```

Note: The exact tree-sitter API may vary slightly by version. The `tree_sitter_rust::LANGUAGE` constant requires converting `.into()` for `tree_sitter::Language`. If the API uses `tree_sitter_rust::language()` instead, adjust accordingly. Check the docs with `cargo doc -p tree-sitter-rust --open` if needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/creep && cargo test parser`
Expected: all 9 tests PASS.

If any test fails, debug by examining the tree-sitter CST. Add a temporary debug helper:
```rust
fn print_tree(node: tree_sitter::Node<'_>, content: &[u8], indent: usize) {
    let text = node.utf8_text(content).unwrap_or("<err>");
    let preview = if text.len() > 60 { &text[..60] } else { text };
    eprintln!("{:indent$}{} [{}-{}] {:?}", "", node.kind(), node.start_position().row, node.end_position().row, preview, indent = indent);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        print_tree(child, content, indent + 2);
    }
}
```

- [ ] **Step 5: Commit**

```bash
git add src/creep/src/parser.rs src/creep/src/main.rs
git commit -m "feat(creep): add tree-sitter Rust parser with symbol extraction"
```

---

### Task 3: Create SymbolIndex with search and listing

**Files:**
- Create: `src/creep/src/symbol_index.rs`
- Modify: `src/creep/src/main.rs:1-8` (add `mod symbol_index;`)

- [ ] **Step 1: Write failing tests for SymbolIndex**

Create `src/creep/src/symbol_index.rs` with types, stub methods returning `todo!()`, and tests:

```rust
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
        todo!()
    }

    /// Remove all symbols for a file from both indexes.
    pub async fn remove_file(&self, path: &Path) {
        todo!()
    }

    /// Search symbols by name (case-insensitive substring match).
    /// Optionally filter by kind and/or workspace.
    pub async fn search(
        &self,
        query: &str,
        kind: Option<&SymbolKind>,
        workspace: Option<&Path>,
    ) -> Vec<(Symbol, PathBuf)> {
        todo!()
    }

    /// List all symbols in a specific file, ordered by line number.
    pub async fn list_file_symbols(&self, path: &Path) -> Vec<Symbol> {
        todo!()
    }

    /// Parse all supported files in a directory. Returns total symbol count.
    /// Runs synchronously (call from spawn_blocking).
    pub fn scan_workspace(&self, root: &Path) -> anyhow::Result<u64> {
        todo!()
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

    #[tokio::test]
    async fn test_reparse_and_list_file_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub fn hello() {} pub struct World;").unwrap();

        let idx = SymbolIndex::new();
        idx.reparse_file(&file).unwrap();

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
        idx.reparse_file(&file).unwrap();

        let results = idx.search("process", None, None).await;
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_search_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "pub struct FileIndex;").unwrap();

        let idx = SymbolIndex::new();
        idx.reparse_file(&file).unwrap();

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
        idx.reparse_file(&file).unwrap();

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
        idx.reparse_file(&file).unwrap();
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
        idx.reparse_file(&file).unwrap();
        assert_eq!(idx.search("old_fn", None, None).await.len(), 1);

        std::fs::write(&file, "fn new_fn() {}").unwrap();
        idx.reparse_file(&file).unwrap();

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
        let count = idx.scan_workspace(&base).unwrap();
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
        idx.reparse_file(&base_a.join("lib.rs")).unwrap();
        idx.reparse_file(&base_b.join("lib.rs")).unwrap();

        let all = idx.search("shared_name", None, None).await;
        assert_eq!(all.len(), 2);

        let filtered = idx.search("shared_name", None, Some(&base_a)).await;
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].1.starts_with(&base_a));

        let _ = std::fs::remove_dir_all(&base_a);
        let _ = std::fs::remove_dir_all(&base_b);
    }
}
```

Add `mod symbol_index;` to `src/creep/src/main.rs` after `mod parser;`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/creep && cargo test symbol_index`
Expected: FAIL — `todo!()` panics.

- [ ] **Step 3: Implement SymbolIndex methods**

Replace all `todo!()` bodies in `src/creep/src/symbol_index.rs`:

```rust
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

pub async fn list_file_symbols(&self, path: &Path) -> Vec<Symbol> {
    let by_file = self.by_file.read().await;
    let mut symbols = by_file.get(path).cloned().unwrap_or_default();
    symbols.sort_by_key(|s| s.line);
    symbols
}

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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/creep && cargo test symbol_index`
Expected: all 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/creep/src/symbol_index.rs src/creep/src/main.rs
git commit -m "feat(creep): add SymbolIndex with name search and file listing"
```

---

### Task 4: Extend proto with symbol RPCs

**Files:**
- Modify: `src/creep/proto/creep.proto`
- Regen: `src/creep/proto_gen/creep.v1.rs`
- Regen: `src/creep-cli/proto_gen/creep.v1.rs`

- [ ] **Step 1: Add symbol messages and RPCs to proto**

Add inside the `service FileIndex` block in `src/creep/proto/creep.proto`, after the `UnregisterWorkspace` RPC:

```protobuf
  rpc SearchSymbols(SearchSymbolsRequest) returns (SearchSymbolsResponse);
  rpc ListFileSymbols(ListFileSymbolsRequest) returns (ListFileSymbolsResponse);
```

Add after all existing message definitions (after `message UnregisterWorkspaceResponse {}`):

```protobuf
message SymbolInfo {
  string name = 1;
  string kind = 2;
  string file = 3;
  uint32 line = 4;
  uint32 end_line = 5;
  optional string parent = 6;
  optional string signature = 7;
}

message SearchSymbolsRequest {
  string query = 1;
  optional string kind = 2;
  optional string workspace = 3;
}

message SearchSymbolsResponse {
  repeated SymbolInfo symbols = 1;
}

message ListFileSymbolsRequest {
  string path = 1;
}

message ListFileSymbolsResponse {
  repeated SymbolInfo symbols = 1;
}
```

- [ ] **Step 2: Regenerate proto Rust code for creep**

```bash
cd src/creep && cargo build 2>/dev/null
find target/debug/build -path '*/creep-*/out/creep.v1.rs' -exec cp {} proto_gen/creep.v1.rs \;
```

- [ ] **Step 3: Regenerate proto Rust code for creep-cli**

```bash
cd src/creep-cli && cargo build 2>/dev/null
find target/debug/build -path '*/creep-cli-*/out/creep.v1.rs' -exec cp {} proto_gen/creep.v1.rs \;
```

- [ ] **Step 4: Copy updated proto to creep-cli**

```bash
cp src/creep/proto/creep.proto src/creep-cli/proto/creep.proto
```

- [ ] **Step 5: Verify both crates compile**

Run: `cd src/creep && cargo check`
Expected: compiles (new trait methods will be unimplemented — that's Task 5).

Run: `cd src/creep-cli && cargo check`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/creep/proto/creep.proto src/creep/proto_gen/ src/creep-cli/proto/ src/creep-cli/proto_gen/
git commit -m "feat(creep): add SearchSymbols and ListFileSymbols to proto"
```

---

### Task 5: Implement gRPC handlers and wire into server

**Files:**
- Modify: `src/creep/src/service.rs`
- Modify: `src/creep/src/main.rs`

- [ ] **Step 1: Add SymbolIndex to service struct**

In `src/creep/src/service.rs`, add import:

```rust
use crate::symbol_index::SymbolIndex;
```

Add to proto imports (extend the existing `use crate::proto::{...}` block):

```rust
use crate::proto::{
    // ... existing imports ...
    ListFileSymbolsRequest, ListFileSymbolsResponse,
    SearchSymbolsRequest, SearchSymbolsResponse,
    SymbolInfo,
};
```

Change `FileIndexServiceImpl` struct:

```rust
pub struct FileIndexServiceImpl {
    pub index: FileIndex,
    pub symbol_index: SymbolIndex,
    pub watcher: Arc<Mutex<FileWatcher>>,
}
```

Update `new()`:

```rust
pub fn new(index: FileIndex, symbol_index: SymbolIndex, watcher: Arc<Mutex<FileWatcher>>) -> Self {
    Self { index, symbol_index, watcher }
}
```

- [ ] **Step 2: Add conversion helper and RPC implementations**

Add after `to_proto_metadata` function:

```rust
fn to_proto_symbol(sym: &crate::parser::Symbol, file: &std::path::Path) -> SymbolInfo {
    SymbolInfo {
        name: sym.name.clone(),
        kind: sym.kind.as_str().to_string(),
        file: file.to_string_lossy().into_owned(),
        line: sym.line,
        end_line: sym.end_line,
        parent: sym.parent.clone(),
        signature: sym.signature.clone(),
    }
}
```

Add inside the `#[tonic::async_trait] impl FileIndexTrait for FileIndexServiceImpl` block:

```rust
async fn search_symbols(
    &self,
    request: Request<SearchSymbolsRequest>,
) -> Result<Response<SearchSymbolsResponse>, Status> {
    let req = request.into_inner();
    let kind = req.kind.as_deref().and_then(crate::parser::SymbolKind::from_str);
    let workspace = req.workspace.as_deref().map(std::path::PathBuf::from);

    let results = self
        .symbol_index
        .search(&req.query, kind.as_ref(), workspace.as_deref())
        .await;

    let symbols = results
        .iter()
        .map(|(sym, path)| to_proto_symbol(sym, path))
        .collect();

    Ok(Response::new(SearchSymbolsResponse { symbols }))
}

async fn list_file_symbols(
    &self,
    request: Request<ListFileSymbolsRequest>,
) -> Result<Response<ListFileSymbolsResponse>, Status> {
    let req = request.into_inner();
    let path = std::path::PathBuf::from(&req.path);

    let symbols = self
        .symbol_index
        .list_file_symbols(&path)
        .await
        .iter()
        .map(|sym| to_proto_symbol(sym, &path))
        .collect();

    Ok(Response::new(ListFileSymbolsResponse { symbols }))
}
```

- [ ] **Step 3: Update test helper in service.rs**

In the `#[cfg(test)] mod tests` block, update `make_service()`:

```rust
fn make_service() -> FileIndexServiceImpl {
    let index = FileIndex::new();
    let symbol_index = crate::symbol_index::SymbolIndex::new();
    let (watcher, _rx) = FileWatcher::new(index.clone());
    FileIndexServiceImpl::new(index, symbol_index, watcher)
}
```

- [ ] **Step 4: Wire SymbolIndex into main.rs**

In `src/creep/src/main.rs`, after `let index = FileIndex::new();` add:

```rust
let symbol_index = symbol_index::SymbolIndex::new();
```

In the workspace scan loop, after the existing `index.scan_workspace` block, add:

```rust
{
    let si = symbol_index.clone();
    let ws_clone = ws.clone();
    match tokio::task::spawn_blocking(move || si.scan_workspace(&ws_clone)).await {
        Ok(Ok(n)) => tracing::info!("parsed {n} symbols in {}", ws.display()),
        Ok(Err(e)) => tracing::warn!("symbol scan failed for {}: {e}", ws.display()),
        Err(e) => tracing::warn!("symbol scan task panicked for {}: {e}", ws.display()),
    }
}
```

Update the service construction:

```rust
let file_index_svc = FileIndexServiceImpl::new(index, symbol_index, watcher);
```

- [ ] **Step 5: Verify all tests pass**

Run: `cd src/creep && cargo test`
Expected: all tests PASS (parser, symbol_index, service, watcher, config).

- [ ] **Step 6: Commit**

```bash
git add src/creep/src/service.rs src/creep/src/main.rs
git commit -m "feat(creep): wire SymbolIndex into gRPC service with SearchSymbols and ListFileSymbols"
```

---

### Task 6: Hook symbol reparsing into file watcher

**Files:**
- Modify: `src/creep/src/watcher.rs`
- Modify: `src/creep/src/main.rs` (process_events call)

- [ ] **Step 1: Write test for symbol updates via watcher events**

Add to `src/creep/src/watcher.rs` `mod tests`:

```rust
#[tokio::test]
async fn test_process_events_updates_symbol_index() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("hello.rs");
    std::fs::write(&file_path, "fn greeting() {}").unwrap();

    let index = FileIndex::new();
    let symbol_index = crate::symbol_index::SymbolIndex::new();
    let (fw, rx) = FileWatcher::new(index.clone());

    {
        let guard = fw.lock().await;
        guard
            .tx
            .send(WatchEvent::Created(file_path.clone()))
            .await
            .unwrap();
    }

    let handle = tokio::spawn(process_events(
        index.clone(),
        symbol_index.clone(),
        fw.clone(),
        rx,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handle.abort();

    let symbols = symbol_index.list_file_symbols(&file_path).await;
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greeting");
}

#[tokio::test]
async fn test_process_events_removes_symbols_on_delete() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("hello.rs");
    std::fs::write(&file_path, "fn greeting() {}").unwrap();

    let index = FileIndex::new();
    let symbol_index = crate::symbol_index::SymbolIndex::new();
    index.update_file(&file_path).await.unwrap();
    symbol_index.reparse_file(&file_path).unwrap();
    assert_eq!(symbol_index.list_file_symbols(&file_path).await.len(), 1);

    let (fw, rx) = FileWatcher::new(index.clone());
    {
        let guard = fw.lock().await;
        guard
            .tx
            .send(WatchEvent::Removed(file_path.clone()))
            .await
            .unwrap();
    }

    let handle = tokio::spawn(process_events(
        index.clone(),
        symbol_index.clone(),
        fw.clone(),
        rx,
    ));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handle.abort();

    assert!(symbol_index.list_file_symbols(&file_path).await.is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/creep && cargo test test_process_events_updates_symbol`
Expected: FAIL — `process_events` signature doesn't accept SymbolIndex yet.

- [ ] **Step 3: Update `process_events` to accept and use SymbolIndex**

Change the function signature in `src/creep/src/watcher.rs`:

```rust
pub async fn process_events(
    index: FileIndex,
    symbol_index: crate::symbol_index::SymbolIndex,
    watcher: Arc<Mutex<FileWatcher>>,
    mut event_rx: mpsc::Receiver<WatchEvent>,
) {
```

In the `Created | Modified` arm, add symbol reparse after `index.update_file`:

```rust
WatchEvent::Created(p) | WatchEvent::Modified(p) => {
    if p.is_file() {
        if let Err(e) = index.update_file(&p).await {
            tracing::warn!("failed to index {}: {}", p.display(), e);
        } else {
            tracing::debug!("indexed {}", p.display());
        }
        let si = symbol_index.clone();
        let p2 = p.clone();
        if let Err(e) =
            tokio::task::spawn_blocking(move || si.reparse_file(&p2)).await
        {
            tracing::warn!("symbol reparse failed for {}: {e}", p.display());
        }
    }
}
```

In the `Removed` arm, add symbol removal:

```rust
WatchEvent::Removed(p) => {
    index.remove_file(&p).await;
    symbol_index.remove_file(&p).await;
    tracing::debug!("removed {} from index", p.display());
}
```

- [ ] **Step 4: Fix existing watcher tests to pass SymbolIndex**

Update `test_process_events_updates_index` and `test_process_events_removes_from_index` — add `let symbol_index = crate::symbol_index::SymbolIndex::new();` and pass it to `process_events`:

```rust
let handle = tokio::spawn(process_events(index_clone, symbol_index, fw_clone, rx));
```

- [ ] **Step 5: Update main.rs to pass symbol_index to process_events**

In `src/creep/src/main.rs`, update the spawn call:

```rust
tokio::spawn(process_events(index.clone(), symbol_index.clone(), watcher.clone(), event_rx));
```

- [ ] **Step 6: Run all tests**

Run: `cd src/creep && cargo test`
Expected: all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/creep/src/watcher.rs src/creep/src/main.rs
git commit -m "feat(creep): hook symbol reparsing into file watcher events"
```

---

### Task 7: Add config fields for symbol index

**Files:**
- Modify: `src/creep/src/config.rs`
- Modify: `src/creep/src/main.rs` (gate scanning on config)

- [ ] **Step 1: Write failing tests for new config fields**

Add to `src/creep/src/config.rs` `mod tests`:

```rust
#[test]
fn test_symbol_index_config() {
    let toml_str = r#"
[creep]
grpc_port = 9090
symbol_index = false
languages = ["rust"]
"#;
    let config: Config = toml::from_str(toml_str).unwrap();
    assert!(!config.creep.symbol_index);
    assert_eq!(config.creep.languages, vec!["rust"]);
}

#[test]
fn test_symbol_index_defaults() {
    let config: Config = toml::from_str("[creep]").unwrap();
    assert!(config.creep.symbol_index);
    assert_eq!(config.creep.languages, vec!["rust"]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/creep && cargo test config`
Expected: FAIL — fields don't exist on CreepConfig.

- [ ] **Step 3: Add fields to CreepConfig**

In `src/creep/src/config.rs`, add default functions:

```rust
fn default_symbol_index() -> bool {
    true
}

fn default_languages() -> Vec<String> {
    vec!["rust".to_string()]
}
```

Add fields to `CreepConfig`:

```rust
#[derive(Debug, Deserialize)]
pub struct CreepConfig {
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default = "default_symbol_index")]
    pub symbol_index: bool,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
}
```

Update `impl Default for CreepConfig`:

```rust
impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            grpc_port: default_grpc_port(),
            workspaces: Vec::new(),
            symbol_index: default_symbol_index(),
            languages: default_languages(),
        }
    }
}
```

- [ ] **Step 4: Gate symbol scanning on config in main.rs**

In `src/creep/src/main.rs`, wrap the symbol scan in the workspace loop with a config check:

```rust
if config.creep.symbol_index {
    let si = symbol_index.clone();
    let ws_clone = ws.clone();
    match tokio::task::spawn_blocking(move || si.scan_workspace(&ws_clone)).await {
        Ok(Ok(n)) => tracing::info!("parsed {n} symbols in {}", ws.display()),
        Ok(Err(e)) => tracing::warn!("symbol scan failed for {}: {e}", ws.display()),
        Err(e) => tracing::warn!("symbol scan task panicked for {}: {e}", ws.display()),
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd src/creep && cargo test`
Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/creep/src/config.rs src/creep/src/main.rs
git commit -m "feat(creep): add symbol_index and languages config options"
```

---

### Task 8: Add `symbols` subcommand to creep-cli

**Files:**
- Modify: `src/creep-cli/src/main.rs`

- [ ] **Step 1: Add Symbols variant to Commands enum**

Add to the `Commands` enum in `src/creep-cli/src/main.rs`:

```rust
/// Search for symbols by name, or list symbols in a file
Symbols {
    /// Symbol name to search for (substring match, case-insensitive)
    query: Option<String>,
    /// List all symbols in this file instead of searching by name
    #[arg(long)]
    file: Option<String>,
    /// Filter by symbol kind (function, struct, enum, trait, impl, const, static, type_alias, module, macro)
    #[arg(long)]
    kind: Option<String>,
    /// Filter by workspace path
    #[arg(long)]
    workspace: Option<String>,
},
```

- [ ] **Step 2: Add match arm in main**

Add to the `match cli.command` block:

```rust
Commands::Symbols {
    query,
    file,
    kind,
    workspace,
} => {
    if let Some(file_path) = file {
        cmd_list_file_symbols(&mut client, &file_path, cli.json).await
    } else {
        cmd_search_symbols(
            &mut client,
            query.as_deref().unwrap_or(""),
            kind,
            workspace,
            cli.json,
        )
        .await
    }
}
```

- [ ] **Step 3: Implement handler functions**

Add after the existing `cmd_unregister` function:

```rust
async fn cmd_search_symbols(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    query: &str,
    kind: Option<String>,
    workspace: Option<String>,
    json: bool,
) -> Result<()> {
    let response = client
        .search_symbols(proto::SearchSymbolsRequest {
            query: query.to_string(),
            kind,
            workspace,
        })
        .await
        .context("search_symbols RPC failed")?;

    let symbols = response.into_inner().symbols;
    if json {
        print_json(&symbols)?;
    } else {
        for s in &symbols {
            let parent_str = match s.parent.as_deref() {
                Some(p) if !p.is_empty() => format!(" ({p})"),
                _ => String::new(),
            };
            let display_name = s.signature.as_deref().unwrap_or(&s.name);
            println!(
                "{:<10} {}{:<40} {}:{}",
                s.kind,
                display_name,
                parent_str,
                s.file,
                s.line + 1,
            );
        }
        if symbols.is_empty() {
            eprintln!("no symbols found matching '{query}'");
        }
    }
    Ok(())
}

async fn cmd_list_file_symbols(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .list_file_symbols(proto::ListFileSymbolsRequest {
            path: path.to_string(),
        })
        .await
        .context("list_file_symbols RPC failed")?;

    let symbols = response.into_inner().symbols;
    if json {
        print_json(&symbols)?;
    } else {
        for s in &symbols {
            let parent_str = match s.parent.as_deref() {
                Some(p) if !p.is_empty() => format!(" ({p})"),
                _ => String::new(),
            };
            let display_name = s.signature.as_deref().unwrap_or(&s.name);
            println!(
                "  {:>4}  {:<10} {}{}",
                s.line + 1,
                s.kind,
                display_name,
                parent_str,
            );
        }
        if symbols.is_empty() {
            eprintln!("no symbols found in '{path}'");
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src/creep-cli && cargo check`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src/creep-cli/src/main.rs
git commit -m "feat(creep-cli): add symbols subcommand for search and file listing"
```

---

### Task 9: Update BUCK files and buckify

**Files:**
- Modify: `src/creep/BUCK`
- Regen: `third-party/BUCK`
- Maybe create: `third-party/fixups/tree-sitter/fixups.toml`, `third-party/fixups/tree-sitter-rust/fixups.toml`

- [ ] **Step 1: Run buckify**

```bash
./tools/buckify.sh
```

- [ ] **Step 2: Add tree-sitter deps to creep BUCK**

Add to `CREEP_DEPS` list in `src/creep/BUCK`:

```python
    "//third-party:tree-sitter",
    "//third-party:tree-sitter-rust",
```

- [ ] **Step 3: Attempt Buck2 build**

```bash
buck2 build root//src/creep:creep
```

If it fails on tree-sitter build scripts, create fixups. Both tree-sitter and tree-sitter-rust compile C code via `cc` crate in their build scripts.

Create `third-party/fixups/tree-sitter/fixups.toml`:
```toml
[buildscript]
run = true
```

Create `third-party/fixups/tree-sitter-rust/fixups.toml`:
```toml
[buildscript]
run = true
```

Then re-run:
```bash
./tools/buckify.sh
buck2 build root//src/creep:creep
```

If `cc` crate needs additional fixups (e.g. `rustc_link_lib`, `rustc_link_search`), add them following the pattern in `third-party/fixups/blake3/fixups.toml` or `third-party/fixups/ring/fixups.toml`.

- [ ] **Step 4: Build both creep and creep-cli**

```bash
buck2 build root//src/creep:creep root//src/creep-cli:creep-cli
```

Expected: both build successfully.

- [ ] **Step 5: Run Buck2 tests**

```bash
buck2 test root//src/creep:creep-test
```

Expected: all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/creep/BUCK third-party/ Cargo.lock
git commit -m "build: add tree-sitter to Buck2 build with fixups"
```

---

### Task 10: Update drone skill documentation

**Files:**
- Modify: `src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md`

- [ ] **Step 1: Add symbol commands to SKILL.md**

Add a new section after "### Get metadata for a specific file":

```markdown
### Search for symbols by name

```bash
creep-cli symbols "process"                         # find symbols containing "process"
creep-cli symbols "Config" --kind struct             # find structs named "Config"
creep-cli symbols --file /path/to/file.rs            # list all symbols in a file
creep-cli symbols --file /path/to/file.rs --json     # JSON output for parsing
creep-cli symbols "handler" --workspace /path/repo   # search within workspace
```

Symbol kinds: function, struct, enum, trait, impl, const, static, type_alias, module, macro.

Output includes: name, kind, file path, line number, enclosing scope (parent), and signature (for functions).
```

Add to the "## When to Use" section:

```markdown
- Finding function/struct/trait definitions by name (faster and more precise than Grep)
- Getting a structural overview of a file (all functions, structs, traits defined in it)
- Understanding what's implemented on a type (search for impl blocks)
```

Add to the "## When NOT to Use" section:

```markdown
- Finding call sites or references (use Grep — symbol search finds definitions only)
```

- [ ] **Step 2: Commit**

```bash
git add src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md
git commit -m "docs: add symbol search commands to creep-discovery skill"
```

---

### Task 11: Update roadmap

**Files:**
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Replace item #11 in Phase 5**

In `docs/ROADMAP.md`, replace:

```
**11. Creep v2** — tree-sitter AST parsing, symbol index
```

With:

```
**11a. Symbol Index** `[done]` — tree-sitter Rust parsing, SearchSymbols + ListFileSymbols RPCs, creep-cli symbols
**11b. Context Extraction** — given file:line, return enclosing scope chain + locals in scope
**11c. Relationship Graph** — callers, callees, import graph, dependent graph (tree-sitter, name-resolved)
**11d. Additional Languages** — tree-sitter grammars for TypeScript, Python
```

- [ ] **Step 2: Commit**

```bash
git add docs/ROADMAP.md
git commit -m "docs: expand Phase 5 roadmap with Creep sub-phases"
```

---

## Self-Review

**Spec coverage:**
- Symbol data model (10 kinds, parent, signature) — Task 2
- SymbolIndex (by_file + by_name) — Task 3
- gRPC RPCs (SearchSymbols, ListFileSymbols) — Task 4, 5
- tree-sitter queries — Task 2
- Incremental updates via watcher — Task 6
- Config (symbol_index, languages) — Task 7
- CLI (symbols subcommand) — Task 8
- Skill update — Task 10
- Buck2 build — Task 9
- Roadmap update — Task 11
- Resource estimates — covered by design (no persistence, reparse on demand)

**Placeholder scan:** No TBDs, TODOs, or vague steps. All code blocks are complete.

**Type consistency:** `Symbol`, `SymbolKind`, `SymbolRef`, `SymbolIndex` used consistently across Tasks 2-8. Proto `SymbolInfo` maps to `Symbol` via `to_proto_symbol` in Task 5. `parse_symbols` signature consistent between Task 2 (definition) and Task 3 (usage). `process_events` signature updated in Task 6, callers updated in Tasks 6 (tests) and 6 (main.rs).
