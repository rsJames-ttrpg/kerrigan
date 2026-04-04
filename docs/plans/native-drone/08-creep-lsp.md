# Plan 08: Creep LSP Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add LSP server management to Creep. Creep spawns language servers, collects diagnostics passively via its file watcher, and exposes diagnostics + symbol lookups via gRPC. creep-cli gets new commands (`diagnostics`, `definition`, `references`).

**Architecture:** `LspManager` inside Creep manages per-workspace language server processes. Hooks into the existing file watcher for document sync. New `LspService` gRPC service alongside existing `FileIndex`. creep-cli adds CLI commands that call the new gRPC methods.

**Tech Stack:** tonic (gRPC), tokio (async I/O, process management), lsp-types (LSP protocol types), serde_json (JSON-RPC)

**Spec:** `docs/specs/2026-04-04-creep-lsp-integration-design.md`

**Reference:** `rust/crates/lsp/` in Claw Code repo (LSP JSON-RPC client patterns)

**Independent:** This plan has no dependency on the native drone plans and can execute in parallel.

---

### Task 1: LSP JSON-RPC client

**Files:**
- Create: `src/creep/src/lsp/mod.rs`
- Create: `src/creep/src/lsp/jsonrpc.rs`
- Create: `src/creep/src/lsp/types.rs`
- Modify: `src/creep/Cargo.toml`

- [ ] **Step 1: Add dependencies**

In `src/creep/Cargo.toml`:
```toml
lsp-types = "0.97"
```

Run: `./tools/buckify.sh`

- [ ] **Step 2: Implement JSON-RPC message types with tests**

Create `src/creep/src/lsp/jsonrpc.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: i64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: i64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<i64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcNotification {
    pub method: String,
    pub params: Option<serde_json::Value>,
}

/// Encode a JSON-RPC message with Content-Length framing
pub fn encode_message(msg: &impl Serialize) -> Vec<u8> {
    let body = serde_json::to_string(msg).unwrap();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

/// Parse Content-Length header and extract JSON body from a buffered reader
pub fn decode_header(header: &str) -> Option<usize> {
    header
        .strip_prefix("Content-Length: ")
        .and_then(|s| s.trim().parse().ok())
}
```

Tests: encode/decode roundtrip, header parsing, notification deserialization.

- [ ] **Step 3: Implement LspClient with stdio transport**

Create `src/creep/src/lsp/client.rs`:

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{oneshot, Mutex};

pub struct LspClient {
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<i64, oneshot::Sender<serde_json::Value>>>,
    next_id: AtomicI64,
    diagnostics: Arc<DiagnosticsCache>,
    _reader_task: tokio::task::JoinHandle<()>,
    _child: Child,
}
```

Implement:
- `connect(config: &LspServerConfig)` — spawn process, start reader task
- `request(method, params)` — send request, wait for response via oneshot
- `notify(method, params)` — send notification (no response expected)
- Reader task: parse frames, route responses to pending map, handle `textDocument/publishDiagnostics` notifications

- [ ] **Step 4: Implement initialize handshake**

```rust
impl LspClient {
    pub async fn initialize(&self, workspace_root: &str) -> anyhow::Result<()> {
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": format!("file://{workspace_root}"),
            "capabilities": {
                "textDocument": {
                    "synchronization": { "dynamicRegistration": false },
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false }
                }
            }
        });
        self.request("initialize", Some(params)).await?;
        self.notify("initialized", Some(serde_json::json!({}))).await?;
        Ok(())
    }
}
```

- [ ] **Step 5: Run tests, commit**

```bash
git add src/creep/ Cargo.lock third-party/BUCK
git commit -m "add LSP JSON-RPC client with stdio transport"
```

---

### Task 2: Diagnostics cache and document sync

**Files:**
- Create: `src/creep/src/lsp/diagnostics.rs`
- Create: `src/creep/src/lsp/document.rs`

- [ ] **Step 1: Implement DiagnosticsCache**

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Info = 3,
    Hint = 4,
}

pub struct DiagnosticsCache {
    entries: RwLock<BTreeMap<PathBuf, Vec<LspDiagnostic>>>,
}

impl DiagnosticsCache {
    pub fn new() -> Self {
        Self { entries: RwLock::new(BTreeMap::new()) }
    }

    pub fn update(&self, file: PathBuf, diagnostics: Vec<LspDiagnostic>) {
        let mut entries = self.entries.write().unwrap();
        if diagnostics.is_empty() {
            entries.remove(&file);
        } else {
            entries.insert(file, diagnostics);
        }
    }

    pub fn get_all(&self, min_severity: DiagnosticSeverity) -> Vec<LspDiagnostic> {
        let entries = self.entries.read().unwrap();
        entries
            .values()
            .flat_map(|v| v.iter())
            .filter(|d| d.severity <= min_severity)
            .cloned()
            .collect()
    }

    pub fn get_file(&self, file: &PathBuf) -> Vec<LspDiagnostic> {
        let entries = self.entries.read().unwrap();
        entries.get(file).cloned().unwrap_or_default()
    }
}
```

Tests: update and retrieve, empty diagnostics clears entry, severity filtering.

- [ ] **Step 2: Implement document sync methods on LspClient**

```rust
impl LspClient {
    pub async fn open_document(&self, path: &str, content: &str, language_id: &str) -> anyhow::Result<()> {
        self.notify("textDocument/didOpen", Some(serde_json::json!({
            "textDocument": {
                "uri": format!("file://{path}"),
                "languageId": language_id,
                "version": 1,
                "text": content
            }
        }))).await
    }

    pub async fn change_document(&self, path: &str, content: &str, version: i32) -> anyhow::Result<()> {
        self.notify("textDocument/didChange", Some(serde_json::json!({
            "textDocument": { "uri": format!("file://{path}"), "version": version },
            "contentChanges": [{ "text": content }]
        }))).await
    }

    pub async fn close_document(&self, path: &str) -> anyhow::Result<()> {
        self.notify("textDocument/didClose", Some(serde_json::json!({
            "textDocument": { "uri": format!("file://{path}") }
        }))).await
    }
}
```

- [ ] **Step 3: Run tests, commit**

```bash
git add src/creep/
git commit -m "add diagnostics cache and document sync for LSP"
```

---

### Task 3: Symbol lookup methods

**Files:**
- Modify: `src/creep/src/lsp/client.rs`

- [ ] **Step 1: Implement goto_definition and find_references**

```rust
#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub file: PathBuf,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl LspClient {
    pub async fn goto_definition(&self, file: &str, line: u32, column: u32) -> anyhow::Result<Vec<SymbolLocation>> {
        let result = self.request("textDocument/definition", Some(serde_json::json!({
            "textDocument": { "uri": format!("file://{file}") },
            "position": { "line": line, "character": column }
        }))).await?;
        parse_locations(result)
    }

    pub async fn find_references(&self, file: &str, line: u32, column: u32, include_declaration: bool) -> anyhow::Result<Vec<SymbolLocation>> {
        let result = self.request("textDocument/references", Some(serde_json::json!({
            "textDocument": { "uri": format!("file://{file}") },
            "position": { "line": line, "character": column },
            "context": { "includeDeclaration": include_declaration }
        }))).await?;
        parse_locations(result)
    }
}

fn parse_locations(value: serde_json::Value) -> anyhow::Result<Vec<SymbolLocation>> {
    // Null = no results
    if value.is_null() {
        return Ok(vec![]);
    }

    // Single Location: { uri, range }
    if value.is_object() && value.get("uri").is_some() {
        return Ok(vec![parse_single_location(&value)?]);
    }

    // Array of Location or LocationLink
    if let Some(arr) = value.as_array() {
        let mut locations = Vec::new();
        for item in arr {
            if item.get("targetUri").is_some() {
                // LocationLink: { targetUri, targetRange }
                let uri = item["targetUri"].as_str().unwrap_or_default();
                let range = &item["targetRange"];
                locations.push(SymbolLocation {
                    file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
                    start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
                    start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
                    end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
                    end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
                });
            } else {
                // Standard Location: { uri, range }
                locations.push(parse_single_location(item)?);
            }
        }
        return Ok(locations);
    }

    Ok(vec![])
}

fn parse_single_location(value: &serde_json::Value) -> anyhow::Result<SymbolLocation> {
    let uri = value["uri"].as_str().unwrap_or_default();
    let range = &value["range"];
    Ok(SymbolLocation {
        file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
        start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
        start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
        end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
        end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
    })
}
```

Tests: parse each location response shape.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/creep/
git commit -m "add goto_definition and find_references to LSP client"
```

---

### Task 4: LSP Manager with lifecycle management

**Files:**
- Create: `src/creep/src/lsp/manager.rs`

- [ ] **Step 1: Implement LspManager**

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct LspServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub extensions: Vec<String>,
    pub language_id: String,
}

pub struct LspManager {
    configs: Vec<LspServerConfig>,
    clients: HashMap<String, LspClient>,       // key: "{server_name}:{workspace_path}"
    extension_map: HashMap<String, String>,    // extension → server name
}

impl LspManager {
    pub fn new(configs: Vec<LspServerConfig>) -> Self {
        let mut extension_map = HashMap::new();
        for config in &configs {
            for ext in &config.extensions {
                extension_map.insert(ext.clone(), config.name.clone());
            }
        }
        Self { configs, clients: HashMap::new(), extension_map }
    }

    /// Start language server for a workspace if not already running
    pub async fn ensure_server(&mut self, workspace: &Path, file_ext: &str) -> anyhow::Result<Option<&LspClient>> {
        let server_name = match self.extension_map.get(file_ext) {
            Some(name) => name.clone(),
            None => return Ok(None),
        };

        let key = format!("{server_name}:{}", workspace.display());
        if !self.clients.contains_key(&key) {
            let config = self.configs.iter().find(|c| c.name == server_name).unwrap();
            let client = LspClient::connect(config, workspace).await?;
            client.initialize(&workspace.to_string_lossy()).await?;
            self.clients.insert(key.clone(), client);
        }

        Ok(self.clients.get(&key))
    }

    /// Shutdown all language servers
    pub async fn shutdown_all(&mut self) {
        for (_, client) in self.clients.drain() {
            let _ = client.shutdown().await;
        }
    }

    /// Get diagnostics across all active servers for a workspace
    pub fn diagnostics(&self, workspace: &Path, min_severity: DiagnosticSeverity) -> Vec<LspDiagnostic> {
        self.clients
            .iter()
            .filter(|(k, _)| k.ends_with(&format!(":{}", workspace.display())))
            .flat_map(|(_, client)| client.diagnostics.get_all(min_severity))
            .collect()
    }
}
```

- [ ] **Step 2: Hook into Creep's file watcher for document sync**

When Creep's existing file watcher (notify) detects changes in a registered workspace, forward them to the LSP manager:

```rust
// In the existing watcher event handler:
match event.kind {
    notify::EventKind::Create(_) => {
        let content = tokio::fs::read_to_string(&path).await?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if let Some(client) = lsp_manager.ensure_server(workspace, ext).await? {
            client.open_document(&path_str, &content, &language_id).await?;
        }
    }
    notify::EventKind::Modify(_) => {
        // didChange with full content
    }
    notify::EventKind::Remove(_) => {
        // didClose
    }
}
```

Tests: verify document open/change/close are called for matching extensions.

- [ ] **Step 3: Run tests, commit**

```bash
git add src/creep/
git commit -m "add LSP manager with lifecycle and file watcher integration"
```

---

### Task 5: gRPC service

**Files:**
- Modify: `src/creep/proto/creep.proto`
- Modify: `src/creep/src/main.rs` (or server setup)

- [ ] **Step 1: Add LspService to proto**

In `src/creep/proto/creep.proto`, add:

```protobuf
service LspService {
    rpc GetDiagnostics(GetDiagnosticsRequest) returns (GetDiagnosticsResponse);
    rpc GetFileDiagnostics(GetFileDiagnosticsRequest) returns (GetFileDiagnosticsResponse);
    rpc GotoDefinition(GotoDefinitionRequest) returns (GotoDefinitionResponse);
    rpc FindReferences(FindReferencesRequest) returns (FindReferencesResponse);
}

message GetDiagnosticsRequest {
    string workspace_path = 1;
    int32 min_severity = 2;
    uint32 max_results = 3;
}

message GetDiagnosticsResponse {
    repeated Diagnostic diagnostics = 1;
    uint32 total_count = 2;
}

message Diagnostic {
    string file_path = 1;
    uint32 line = 2;
    uint32 column = 3;
    string severity = 4;
    string message = 5;
    string source = 6;
}

message GetFileDiagnosticsRequest {
    string workspace_path = 1;
    string file_path = 2;
}

message GetFileDiagnosticsResponse {
    repeated Diagnostic diagnostics = 1;
}

message GotoDefinitionRequest {
    string file_path = 1;
    uint32 line = 2;
    uint32 column = 3;
}

message GotoDefinitionResponse {
    repeated SymbolLocation locations = 1;
}

message FindReferencesRequest {
    string file_path = 1;
    uint32 line = 2;
    uint32 column = 3;
    bool include_declaration = 4;
}

message FindReferencesResponse {
    repeated SymbolLocation locations = 1;
}

message SymbolLocation {
    string file_path = 1;
    uint32 start_line = 2;
    uint32 start_column = 3;
    uint32 end_line = 4;
    uint32 end_column = 5;
}
```

- [ ] **Step 2: Regenerate proto and implement service**

Run: `cd src/creep && cargo build` (triggers tonic-build)

Implement the gRPC service handler that delegates to `LspManager`.

- [ ] **Step 3: Register service in Creep's tonic server**

Add `LspServiceServer` alongside existing `FileIndexServer` in the tonic router.

- [ ] **Step 4: Run tests, buckify, verify build**

Run: `cd src/creep && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/creep:creep`

- [ ] **Step 5: Commit**

```bash
git add src/creep/ Cargo.lock third-party/BUCK
git commit -m "add LspService gRPC endpoints for diagnostics and symbol lookup"
```

---

### Task 6: creep-cli commands

**Files:**
- Modify: `src/creep-cli/src/main.rs`

- [ ] **Step 1: Add diagnostics command**

```rust
#[derive(clap::Subcommand)]
enum Commands {
    // ... existing commands
    Diagnostics {
        workspace: String,
        #[arg(long)]
        file: Option<String>,
        #[arg(long, default_value = "warning")]
        severity: String,
        #[arg(long)]
        json: bool,
    },
    Definition {
        /// file:line:column
        location: String,
    },
    References {
        /// file:line:column
        location: String,
        #[arg(long)]
        include_declaration: bool,
    },
}
```

Implement each command:
- `diagnostics` — calls `GetDiagnostics` or `GetFileDiagnostics`, formats as markdown or JSON
- `definition` — parses `file:line:column`, calls `GotoDefinition`, formats locations
- `references` — parses `file:line:column`, calls `FindReferences`, formats locations

Markdown output format for diagnostics:
```markdown
## Workspace Diagnostics (3 errors, 2 warnings)

### Errors
- `src/api/auth.rs:42:5` — cannot find value `token` in this scope (rustc)

### Warnings
- `src/main.rs:8:5` — unused import `std::io::Read` (rustc)
```

- [ ] **Step 2: Run tests, buckify, verify build**

Run: `cd src/creep-cli && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/creep-cli:creep-cli`

- [ ] **Step 3: Commit**

```bash
git add src/creep-cli/ Cargo.lock third-party/BUCK
git commit -m "add diagnostics, definition, references commands to creep-cli"
```

---

### Task 7: Configuration in hatchery.toml

**Files:**
- Modify: `src/queen/src/config.rs` (or wherever hatchery.toml is parsed)
- Modify: `hatchery.toml`

- [ ] **Step 1: Add LSP config section**

```toml
[creep.lsp.rust]
command = "rust-analyzer"
args = []
extensions = [".rs"]
language_id = "rust"

# [creep.lsp.typescript]
# command = "typescript-language-server"
# args = ["--stdio"]
# extensions = [".ts", ".tsx", ".js", ".jsx"]
# language_id = "typescript"
```

Parse this into `Vec<LspServerConfig>` and pass to Creep's `LspManager` at startup.

- [ ] **Step 2: Verify build, commit**

```bash
git add src/queen/ hatchery.toml
git commit -m "add LSP server configuration to hatchery.toml"
```
