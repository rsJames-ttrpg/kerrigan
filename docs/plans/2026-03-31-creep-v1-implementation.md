# Creep v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Creep, the persistent file-indexing gRPC sidecar that drones query for fast file lookups.

**Architecture:** Rust binary using tonic for gRPC, notify for filesystem watching, ignore for .gitignore support, blake3 for content hashing. In-memory file index behind `Arc<RwLock<HashMap>>`. Proto codegen via tonic-build in build.rs.

**Tech Stack:** Rust (edition 2024), tonic, prost, tonic-build, tonic-health, notify, ignore, blake3, tokio, serde, toml, tracing, glob-match

---

## File Structure

```
src/creep/
  Cargo.toml
  BUCK
  build.rs                 # tonic-build proto codegen
  proto/
    creep.proto            # FileIndex service definition
  src/
    main.rs                # Entry: load config, start watcher + gRPC server
    config.rs              # TOML config parsing
    index.rs               # File index: scan, update, search, metadata
    watcher.rs             # notify file watcher -> index updates
    service.rs             # tonic FileIndex gRPC service impl
    lib.rs                 # Module declarations for test access
```

---

### Task 1: Crate scaffolding and proto codegen

**Files:**
- Create: `src/creep/Cargo.toml`
- Create: `src/creep/BUCK`
- Create: `src/creep/build.rs`
- Create: `src/creep/proto/creep.proto`
- Create: `src/creep/src/lib.rs`
- Create: `src/creep/src/main.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create Cargo.toml**

Create `src/creep/Cargo.toml`:

```toml
[package]
name = "creep"
version = "0.1.0"
edition = "2024"

[dependencies]
tonic = "0.13"
tonic-health = "0.13"
prost = "0.13"
notify = "8"
ignore = "0.4"
blake3 = "1"
glob-match = "0.2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "signal", "time"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"

[build-dependencies]
tonic-build = "0.13"
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`:
```toml
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base", "src/creep"]
```

- [ ] **Step 3: Create proto file**

Create `src/creep/proto/creep.proto`:

```protobuf
syntax = "proto3";
package creep.v1;

service FileIndex {
  rpc SearchFiles(SearchFilesRequest) returns (SearchFilesResponse);
  rpc GetFileMetadata(GetFileMetadataRequest) returns (GetFileMetadataResponse);
  rpc RegisterWorkspace(RegisterWorkspaceRequest) returns (RegisterWorkspaceResponse);
  rpc UnregisterWorkspace(UnregisterWorkspaceRequest) returns (UnregisterWorkspaceResponse);
}

message SearchFilesRequest {
  string pattern = 1;
  optional string workspace = 2;
  optional string file_type = 3;
}

message SearchFilesResponse {
  repeated FileMetadata files = 1;
}

message GetFileMetadataRequest {
  string path = 1;
}

message GetFileMetadataResponse {
  optional FileMetadata file = 1;
}

message FileMetadata {
  string path = 1;
  uint64 size = 2;
  int64 modified_at = 3;
  string file_type = 4;
  string content_hash = 5;
}

message RegisterWorkspaceRequest {
  string path = 1;
}

message RegisterWorkspaceResponse {
  uint64 files_indexed = 1;
}

message UnregisterWorkspaceRequest {
  string path = 1;
}

message UnregisterWorkspaceResponse {}
```

- [ ] **Step 4: Create build.rs**

Create `src/creep/build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/creep.proto")?;
    Ok(())
}
```

- [ ] **Step 5: Create lib.rs and main.rs placeholders**

Create `src/creep/src/lib.rs`:
```rust
pub mod config;
pub mod index;
pub mod service;
pub mod watcher;

pub mod proto {
    tonic::include_proto!("creep.v1");
}
```

Create `src/creep/src/main.rs`:
```rust
fn main() {
    println!("creep placeholder");
}
```

- [ ] **Step 6: Create BUCK file**

Create `src/creep/BUCK`:

```python
CREEP_SRCS = glob(["src/**/*.rs", "proto/**/*.proto", "build.rs"])

CREEP_DEPS = [
    "//third-party:anyhow",
    "//third-party:blake3",
    "//third-party:glob-match",
    "//third-party:ignore",
    "//third-party:notify",
    "//third-party:prost",
    "//third-party:serde",
    "//third-party:tokio",
    "//third-party:toml",
    "//third-party:tonic",
    "//third-party:tonic-health",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
]

rust_binary(
    name = "creep",
    srcs = CREEP_SRCS,
    crate_root = "src/main.rs",
    deps = CREEP_DEPS,
    env = {"CARGO_MANIFEST_DIR": "."},
    visibility = ["PUBLIC"],
)

rust_test(
    name = "creep-test",
    srcs = CREEP_SRCS,
    crate_root = "src/lib.rs",
    deps = CREEP_DEPS + [
        "//third-party:tempfile",
    ],
    env = {"CARGO_MANIFEST_DIR": "."},
    visibility = ["PUBLIC"],
)
```

Note: `CARGO_MANIFEST_DIR` env is needed so `tonic-build` in `build.rs` can find the proto files relative to the crate root. The build script needs the `tonic-build` dep too — add fixup if needed.

- [ ] **Step 7: Run buckify and verify**

Run: `cd /home/jackm/repos/kerrigan && ./tools/buckify.sh`
Run: `cargo check -p creep`

Note: This will pull in tonic, prost, notify, ignore, blake3 and their transitive deps. May need fixups for crates with build scripts (e.g., `ring` if pulled by tonic's TLS, `blake3` for SIMD detection). Check existing fixups for patterns.

If Buck2 build fails on proto codegen, create a fixup for the creep crate itself:
```toml
# third-party/fixups/creep/fixups.toml — only if needed
[buildscript]
run = true
```

Actually, since creep is a first-party crate (not third-party), the build.rs runs via Buck2's native build script support in the `rust_binary` rule. The `env = {"CARGO_MANIFEST_DIR": "."}` ensures proto paths resolve correctly.

- [ ] **Step 8: Commit**

```bash
git add src/creep/ Cargo.toml Cargo.lock
git commit -m "feat(creep): scaffold creep crate with proto codegen"
```

---

### Task 2: Configuration

**Files:**
- Create: `src/creep/src/config.rs`
- Modify: `src/creep/src/main.rs`

- [ ] **Step 1: Implement config**

Create `src/creep/src/config.rs`:

```rust
use serde::Deserialize;

fn default_grpc_port() -> u16 {
    9090
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub creep: CreepConfig,
}

#[derive(Debug, Deserialize)]
pub struct CreepConfig {
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default)]
    pub workspaces: Vec<String>,
}

impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            grpc_port: default_grpc_port(),
            workspaces: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let config: Config = toml::from_str("[creep]").unwrap();
        assert_eq!(config.creep.grpc_port, 9090);
        assert!(config.creep.workspaces.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[creep]
grpc_port = 8080
workspaces = ["/home/user/repo1", "/home/user/repo2"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.creep.grpc_port, 8080);
        assert_eq!(config.creep.workspaces.len(), 2);
    }

    #[test]
    fn test_empty_config_uses_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.creep.grpc_port, 9090);
    }
}
```

- [ ] **Step 2: Update main.rs**

Replace `src/creep/src/main.rs`:

```rust
use std::path::PathBuf;

use tracing_subscriber::EnvFilter;

mod config;
mod index;
mod service;
mod watcher;

mod proto {
    tonic::include_proto!("creep.v1");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("hatchery.toml"));

    let config = config::Config::load(&config_path)?;

    tracing::info!(port = config.creep.grpc_port, "creep starting");

    // Index, watcher, and gRPC server will be started here in later tasks.
    tokio::signal::ctrl_c().await?;
    tracing::info!("creep shutting down");

    Ok(())
}
```

- [ ] **Step 3: Run tests**

Run: `cd src/creep && cargo test config::tests`
Expected: ALL PASS (3 tests)

- [ ] **Step 4: Commit**

```bash
git add src/creep/src/
git commit -m "feat(creep): add configuration with TOML parsing"
```

---

### Task 3: File index

**Files:**
- Create: `src/creep/src/index.rs`

- [ ] **Step 1: Implement the file index**

Create `src/creep/src/index.rs`:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

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
    files: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
}

impl FileIndex {
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Scan a directory and index all files, respecting .gitignore.
    pub async fn scan_workspace(&self, workspace: &Path) -> anyhow::Result<u64> {
        let workspace = workspace.to_path_buf();
        let entries = tokio::task::spawn_blocking(move || scan_directory(&workspace))
            .await??;

        let count = entries.len() as u64;
        let mut index = self.files.write().await;
        for entry in entries {
            index.insert(entry.path.clone(), entry);
        }

        Ok(count)
    }

    /// Update or insert a single file's metadata.
    pub async fn update_file(&self, path: &Path) -> anyhow::Result<()> {
        let path = path.to_path_buf();
        let metadata = tokio::task::spawn_blocking(move || index_file(&path))
            .await??;

        self.files.write().await.insert(metadata.path.clone(), metadata);
        Ok(())
    }

    /// Remove a file from the index.
    pub async fn remove_file(&self, path: &Path) {
        self.files.write().await.remove(path);
    }

    /// Search files by glob pattern, optionally filtered by workspace and file type.
    pub async fn search(
        &self,
        pattern: &str,
        workspace: Option<&str>,
        file_type: Option<&str>,
    ) -> Vec<FileMetadata> {
        let index = self.files.read().await;
        index
            .values()
            .filter(|f| {
                if let Some(ws) = workspace {
                    if !f.path.starts_with(ws) {
                        return false;
                    }
                }
                if let Some(ft) = file_type {
                    if f.file_type != ft {
                        return false;
                    }
                }
                glob_match::glob_match(pattern, &f.path.to_string_lossy())
            })
            .cloned()
            .collect()
    }

    /// Get metadata for a specific file path.
    pub async fn get(&self, path: &Path) -> Option<FileMetadata> {
        self.files.read().await.get(path).cloned()
    }

    /// Number of indexed files.
    pub async fn len(&self) -> usize {
        self.files.read().await.len()
    }
}

/// Scan a directory using the ignore crate (respects .gitignore).
fn scan_directory(root: &Path) -> anyhow::Result<Vec<FileMetadata>> {
    let mut entries = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for result in walker {
        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "error walking directory");
                continue;
            }
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        match index_file(entry.path()) {
            Ok(meta) => entries.push(meta),
            Err(e) => {
                tracing::debug!(path = %entry.path().display(), error = %e, "failed to index file");
            }
        }
    }

    Ok(entries)
}

/// Index a single file: read metadata and compute content hash.
fn index_file(path: &Path) -> anyhow::Result<FileMetadata> {
    let fs_meta = std::fs::metadata(path)?;
    let size = fs_meta.len();
    let modified_at = fs_meta
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let file_type = detect_file_type(path);

    let content = std::fs::read(path)?;
    let content_hash = blake3::hash(&content).to_hex().to_string();

    Ok(FileMetadata {
        path: path.to_path_buf(),
        size,
        modified_at,
        file_type,
        content_hash,
    })
}

fn detect_file_type(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("toml") => "toml",
        Some("json") => "json",
        Some("md") => "markdown",
        Some("ts") => "typescript",
        Some("js") => "javascript",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("hpp") | Some("cc") => "cpp",
        Some("yaml") | Some("yml") => "yaml",
        Some("sh") | Some("bash") | Some("zsh") => "shell",
        Some("sql") => "sql",
        Some("proto") => "protobuf",
        Some("bzl") => "starlark",
        _ => "unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_file_type() {
        assert_eq!(detect_file_type(Path::new("foo.rs")), "rust");
        assert_eq!(detect_file_type(Path::new("bar.py")), "python");
        assert_eq!(detect_file_type(Path::new("baz.toml")), "toml");
        assert_eq!(detect_file_type(Path::new("qux.unknown")), "unknown");
        assert_eq!(detect_file_type(Path::new("no_ext")), "unknown");
    }

    #[test]
    fn test_index_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() {}").unwrap();

        let meta = index_file(&file).unwrap();
        assert_eq!(meta.path, file);
        assert_eq!(meta.size, 12);
        assert_eq!(meta.file_type, "rust");
        assert!(!meta.content_hash.is_empty());
    }

    #[test]
    fn test_scan_directory_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();

        // Create a .gitignore
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();

        // Create files
        fs::write(dir.path().join("included.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("ignored.txt"), "should be ignored").unwrap();

        let entries = scan_directory(dir.path()).unwrap();
        let paths: Vec<_> = entries.iter().map(|e| e.path.file_name().unwrap().to_str().unwrap()).collect();

        assert!(paths.contains(&"included.rs"));
        assert!(!paths.contains(&"ignored.txt"));
        // .gitignore itself may or may not be included depending on ignore crate defaults
    }

    #[tokio::test]
    async fn test_file_index_scan_and_search() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("lib.rs"), "pub mod foo;").unwrap();
        fs::write(dir.path().join("readme.md"), "# Hello").unwrap();

        let index = FileIndex::new();
        let count = index.scan_workspace(dir.path()).await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(index.len().await, 3);

        // Search by glob
        let results = index.search("**/*.rs", None, None).await;
        assert_eq!(results.len(), 2);

        // Search by file type
        let results = index.search("**/*", None, Some("rust")).await;
        assert_eq!(results.len(), 2);

        // Search by file type — markdown
        let results = index.search("**/*", None, Some("markdown")).await;
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_file_index_update_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        fs::write(&file, "fn main() {}").unwrap();

        let index = FileIndex::new();

        // Update (insert)
        index.update_file(&file).await.unwrap();
        assert_eq!(index.len().await, 1);

        let meta = index.get(&file).await.unwrap();
        assert_eq!(meta.file_type, "rust");

        // Remove
        index.remove_file(&file).await;
        assert_eq!(index.len().await, 0);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/creep && cargo test index::tests`
Expected: ALL PASS (5 tests)

- [ ] **Step 3: Commit**

```bash
git add src/creep/src/index.rs
git commit -m "feat(creep): add file index with gitignore support and blake3 hashing"
```

---

### Task 4: File watcher

**Files:**
- Create: `src/creep/src/watcher.rs`

- [ ] **Step 1: Implement the watcher**

Create `src/creep/src/watcher.rs`:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::index::FileIndex;

/// Manages filesystem watchers for registered workspaces.
pub struct FileWatcher {
    index: FileIndex,
    watchers: HashMap<PathBuf, RecommendedWatcher>,
    event_tx: mpsc::Sender<WatchEvent>,
    gitignore_matchers: HashMap<PathBuf, Arc<ignore::gitignore::Gitignore>>,
}

enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl FileWatcher {
    pub fn new(index: FileIndex) -> (Self, mpsc::Receiver<WatchEvent>) {
        let (event_tx, event_rx) = mpsc::channel(256);
        (
            Self {
                index,
                watchers: HashMap::new(),
                event_tx,
                gitignore_matchers: HashMap::new(),
            },
            event_rx,
        )
    }

    /// Start watching a workspace directory.
    pub fn watch(&mut self, workspace: &Path) -> anyhow::Result<()> {
        if self.watchers.contains_key(workspace) {
            return Ok(()); // Already watching
        }

        // Build gitignore matcher for this workspace
        let mut builder = ignore::gitignore::GitignoreBuilder::new(workspace);
        let gitignore_path = workspace.join(".gitignore");
        if gitignore_path.exists() {
            builder.add(&gitignore_path);
        }
        let matcher = Arc::new(builder.build()?);
        self.gitignore_matchers
            .insert(workspace.to_path_buf(), matcher);

        let tx = self.event_tx.clone();
        let workspace_path = workspace.to_path_buf();
        let mut watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                let event = match res {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(error = %e, "file watcher error");
                        return;
                    }
                };

                for path in event.paths {
                    // Skip directories
                    if path.is_dir() {
                        continue;
                    }

                    let watch_event = match event.kind {
                        EventKind::Create(_) => WatchEvent::Created(path),
                        EventKind::Modify(_) => WatchEvent::Modified(path),
                        EventKind::Remove(_) => WatchEvent::Removed(path),
                        _ => continue,
                    };

                    if tx.blocking_send(watch_event).is_err() {
                        return; // Receiver dropped
                    }
                }
            })?;

        watcher.watch(&workspace_path, RecursiveMode::Recursive)?;
        self.watchers.insert(workspace_path, watcher);

        Ok(())
    }

    /// Stop watching a workspace.
    pub fn unwatch(&mut self, workspace: &Path) {
        self.watchers.remove(workspace);
        self.gitignore_matchers.remove(workspace);
    }

    /// Check if a path should be ignored based on the workspace's .gitignore.
    pub fn is_ignored(&self, path: &Path) -> bool {
        for (workspace, matcher) in &self.gitignore_matchers {
            if path.starts_with(workspace) {
                let relative = path.strip_prefix(workspace).unwrap_or(path);
                return matcher
                    .matched_path_or_any_parents(relative, path.is_dir())
                    .is_ignore();
            }
        }
        false
    }
}

/// Process watch events and update the index. Runs as a tokio task.
pub async fn process_events(
    index: FileIndex,
    watcher: &FileWatcher,
    mut event_rx: mpsc::Receiver<WatchEvent>,
) {
    while let Some(event) = event_rx.recv().await {
        match event {
            WatchEvent::Created(path) | WatchEvent::Modified(path) => {
                if watcher.is_ignored(&path) {
                    continue;
                }
                if let Err(e) = index.update_file(&path).await {
                    tracing::debug!(path = %path.display(), error = %e, "failed to index file");
                }
            }
            WatchEvent::Removed(path) => {
                index.remove_file(&path).await;
            }
        }
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cd src/creep && cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/creep/src/watcher.rs
git commit -m "feat(creep): add file watcher with gitignore filtering"
```

---

### Task 5: gRPC service implementation

**Files:**
- Create: `src/creep/src/service.rs`

- [ ] **Step 1: Implement the FileIndex gRPC service**

Create `src/creep/src/service.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

use crate::index::FileIndex;
use crate::proto::file_index_server::FileIndex as FileIndexService;
use crate::proto::{
    FileMetadata as ProtoFileMetadata, GetFileMetadataRequest, GetFileMetadataResponse,
    RegisterWorkspaceRequest, RegisterWorkspaceResponse, SearchFilesRequest, SearchFilesResponse,
    UnregisterWorkspaceRequest, UnregisterWorkspaceResponse,
};
use crate::watcher::FileWatcher;

pub struct FileIndexServiceImpl {
    index: FileIndex,
    watcher: Arc<Mutex<FileWatcher>>,
}

impl FileIndexServiceImpl {
    pub fn new(index: FileIndex, watcher: Arc<Mutex<FileWatcher>>) -> Self {
        Self { index, watcher }
    }
}

fn to_proto_metadata(m: &crate::index::FileMetadata) -> ProtoFileMetadata {
    ProtoFileMetadata {
        path: m.path.to_string_lossy().to_string(),
        size: m.size,
        modified_at: m.modified_at,
        file_type: m.file_type.clone(),
        content_hash: m.content_hash.clone(),
    }
}

#[tonic::async_trait]
impl FileIndexService for FileIndexServiceImpl {
    async fn search_files(
        &self,
        request: Request<SearchFilesRequest>,
    ) -> Result<Response<SearchFilesResponse>, Status> {
        let req = request.into_inner();
        let results = self
            .index
            .search(&req.pattern, req.workspace.as_deref(), req.file_type.as_deref())
            .await;
        let files = results.iter().map(to_proto_metadata).collect();
        Ok(Response::new(SearchFilesResponse { files }))
    }

    async fn get_file_metadata(
        &self,
        request: Request<GetFileMetadataRequest>,
    ) -> Result<Response<GetFileMetadataResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);
        let file = self.index.get(&path).await.map(|m| to_proto_metadata(&m));
        Ok(Response::new(GetFileMetadataResponse { file }))
    }

    async fn register_workspace(
        &self,
        request: Request<RegisterWorkspaceRequest>,
    ) -> Result<Response<RegisterWorkspaceResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);

        if !path.is_dir() {
            return Err(Status::invalid_argument(format!(
                "path is not a directory: {}",
                path.display()
            )));
        }

        // Start watching
        {
            let mut watcher = self.watcher.lock().await;
            watcher
                .watch(&path)
                .map_err(|e| Status::internal(format!("failed to watch: {e}")))?;
        }

        // Scan and index
        let files_indexed = self
            .index
            .scan_workspace(&path)
            .await
            .map_err(|e| Status::internal(format!("failed to scan: {e}")))?;

        tracing::info!(path = %path.display(), files_indexed, "workspace registered");

        Ok(Response::new(RegisterWorkspaceResponse { files_indexed }))
    }

    async fn unregister_workspace(
        &self,
        request: Request<UnregisterWorkspaceRequest>,
    ) -> Result<Response<UnregisterWorkspaceResponse>, Status> {
        let req = request.into_inner();
        let path = PathBuf::from(&req.path);

        {
            let mut watcher = self.watcher.lock().await;
            watcher.unwatch(&path);
        }

        tracing::info!(path = %path.display(), "workspace unregistered");

        Ok(Response::new(UnregisterWorkspaceResponse {}))
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cd src/creep && cargo check`
Expected: compiles (may need to adjust proto import paths based on what tonic-build generates)

- [ ] **Step 3: Commit**

```bash
git add src/creep/src/service.rs
git commit -m "feat(creep): add FileIndex gRPC service implementation"
```

---

### Task 6: Full main.rs integration

**Files:**
- Modify: `src/creep/src/main.rs`

- [ ] **Step 1: Wire everything together**

Replace `src/creep/src/main.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::transport::Server;
use tonic_health::server::health_reporter;
use tracing_subscriber::EnvFilter;

mod config;
mod index;
mod service;
mod watcher;

mod proto {
    tonic::include_proto!("creep.v1");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("hatchery.toml"));

    let config = config::Config::load(&config_path)?;

    tracing::info!(port = config.creep.grpc_port, "creep starting");

    // Create index
    let file_index = index::FileIndex::new();

    // Create watcher
    let (mut file_watcher, event_rx) = watcher::FileWatcher::new(file_index.clone());

    // Index configured workspaces
    for workspace in &config.creep.workspaces {
        let path = PathBuf::from(workspace);
        if path.is_dir() {
            match file_index.scan_workspace(&path).await {
                Ok(count) => {
                    tracing::info!(path = %path.display(), count, "indexed workspace");
                    if let Err(e) = file_watcher.watch(&path) {
                        tracing::warn!(path = %path.display(), error = %e, "failed to watch workspace");
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to index workspace");
                }
            }
        } else {
            tracing::warn!(path = %path.display(), "configured workspace is not a directory, skipping");
        }
    }

    let watcher = Arc::new(Mutex::new(file_watcher));

    // Start event processor
    let event_index = file_index.clone();
    let event_watcher = watcher.clone();
    tokio::spawn(async move {
        let w = event_watcher.lock().await;
        watcher::process_events(event_index, &w, event_rx).await;
    });

    // Health service
    let (mut health_reporter, health_service) = health_reporter();
    health_reporter
        .set_serving::<proto::file_index_server::FileIndexServer<service::FileIndexServiceImpl>>()
        .await;

    // gRPC service
    let file_index_service =
        service::FileIndexServiceImpl::new(file_index.clone(), watcher.clone());

    let addr = format!("0.0.0.0:{}", config.creep.grpc_port).parse()?;
    tracing::info!(%addr, "gRPC server listening");

    Server::builder()
        .add_service(health_service)
        .add_service(proto::file_index_server::FileIndexServer::new(
            file_index_service,
        ))
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.unwrap();
            tracing::info!("creep shutting down");
        })
        .await?;

    Ok(())
}
```

- [ ] **Step 2: Verify build**

Run: `cd src/creep && cargo check`
Expected: compiles

- [ ] **Step 3: Run all tests**

Run: `cd src/creep && cargo test`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/creep/src/main.rs
git commit -m "feat(creep): integrate index, watcher, and gRPC server in main.rs"
```

---

### Task 7: Build verification

**Files:**
- None modified — verification only

- [ ] **Step 1: Run all tests**

Run: `cd src/creep && cargo test`
Expected: ALL PASS

- [ ] **Step 2: Build with Buck2**

Run: `buck2 build root//src/creep:creep`
Expected: BUILD SUCCEEDED

Note: If Buck2 fails on proto codegen, the build.rs needs access to `protoc` and the proto files. The `env = {"CARGO_MANIFEST_DIR": "."}` in BUCK should handle this. If not, may need to add a fixup or adjust the build.rs path resolution.

- [ ] **Step 3: Build entire project**

Run: `buck2 build root//...`
Expected: BUILD SUCCEEDED

- [ ] **Step 4: Run clippy**

Run: `cd src/creep && cargo clippy -- -D warnings`
Expected: No warnings

- [ ] **Step 5: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: All hooks pass

- [ ] **Step 6: Commit any formatting fixes**

```bash
git add -u
git commit -m "style: apply cargo fmt"
```
