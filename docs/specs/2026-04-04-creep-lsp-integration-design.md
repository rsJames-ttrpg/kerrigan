# Creep LSP Integration

**Date:** 2026-04-04
**Status:** Draft

## Problem

LLM coding agents waste context and iterations discovering what's broken. A typical cycle is: edit file → run build → parse 200 lines of compiler output → find the 3 relevant errors → fix → repeat. The compiler already knows what's wrong — the agent just can't see it.

The [Claw Code](https://github.com/rsJames-ttrpg/claudecode) project solves this with an in-process LSP client that spawns language servers, collects diagnostics, and injects them into the system prompt. This works but has drawbacks in the kerrigan architecture: language servers are expensive to start (rust-analyzer takes 30-60s on a cold workspace), each drone would need its own instance, and the lifecycle is tied to the drone process.

Creep is already the persistent file-indexing sidecar that outlives individual drone runs. Moving LSP management into Creep makes language server state persistent, shared, and decoupled from drone lifecycles.

## Solution

Add LSP server management to Creep. Creep spawns and manages language server processes, collects diagnostics passively, and exposes diagnostics + symbol lookups via its existing gRPC API. Drones access this through the creep-cli external tool — no LSP code in the runtime or drone crates.

## Architecture

```
┌──────────────────────────────────────────────────┐
│                    Creep                         │
│                                                  │
│  ┌──────────────┐  ┌──────────────────────────┐  │
│  │ File Index   │  │ LSP Manager              │  │
│  │ (existing)   │  │                          │  │
│  │ - glob       │  │ ┌────────────────────┐   │  │
│  │ - metadata   │  │ │ rust-analyzer      │   │  │
│  │ - blake3     │  │ │ (spawned on demand)│   │  │
│  │ - watcher    │  │ └────────────────────┘   │  │
│  │              │  │ ┌────────────────────┐   │  │
│  │              │  │ │ typescript-ls      │   │  │
│  │              │  │ │ (spawned on demand)│   │  │
│  │              │  │ └────────────────────┘   │  │
│  │              │  │                          │  │
│  │              │  │ Diagnostics cache        │  │
│  │              │  │ Symbol lookups           │  │
│  └──────────────┘  └──────────────────────────┘  │
│                                                  │
│  gRPC API (FileIndex + LspService)               │
└───────────────────┬──────────────────────────────┘
                    │
          ┌─────────▼─────────┐
          │    creep-cli       │
          │  (external tool)   │
          │                    │
          │  creep diagnostics │
          │  creep definition  │
          │  creep references  │
          └────────────────────┘
```

## LSP Manager

### Server Configuration

Configured in `hatchery.toml` alongside existing Creep config:

```toml
[creep.lsp.rust]
command = "rust-analyzer"
args = []
extensions = [".rs"]
language_id = "rust"
# initialization_options = { ... }    # optional, passed to server

[creep.lsp.typescript]
command = "typescript-language-server"
args = ["--stdio"]
extensions = [".ts", ".tsx", ".js", ".jsx"]
language_id = "typescript"
```

### Lifecycle

1. **Lazy startup** — language servers are not started at Creep boot. They start when a workspace is registered (`creep register <path>`) that contains files matching configured extensions.
2. **Per-workspace** — each registered workspace gets its own language server instance (language servers are workspace-scoped). Multiple drones working on the same workspace share the server.
3. **Warm across runs** — when a drone's job completes and the workspace is unregistered, the language server is not immediately killed. It stays alive for a configurable grace period (`lsp_idle_timeout`, default 5 minutes) in case another drone picks up a follow-on job on the same repo.
4. **Explicit shutdown** — `creep unregister <path>` starts the grace timer. If no new registration arrives, the server is shut down cleanly (LSP `shutdown` + `exit`).
5. **Crash recovery** — if a language server process exits unexpectedly, Creep restarts it on next request (not eagerly, to avoid restart loops).

### Document Sync

Creep already watches registered workspaces for file changes via `notify`. The LSP manager hooks into this:

- **File created** → `textDocument/didOpen` (read content, send to server)
- **File modified** → `textDocument/didChange` (full content sync, not incremental — simpler and language servers handle it fine)
- **File deleted** → `textDocument/didClose`
- **File saved** → `textDocument/didSave`

This means the language server's view stays in sync with disk without any drone involvement. When a drone edits a file via the `edit_file` tool, Creep's watcher picks up the change and forwards it to the language server. Diagnostics update automatically.

### Diagnostics Collection

Language servers push `textDocument/publishDiagnostics` notifications asynchronously. Creep stores them in a cache keyed by file path:

```rust
pub struct DiagnosticsCache {
    /// file path → diagnostics, updated on each publishDiagnostics notification
    entries: RwLock<BTreeMap<PathBuf, Vec<LspDiagnostic>>>,
}

pub struct LspDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,   // Error, Warning, Info, Hint
    pub message: String,
    pub source: Option<String>,          // e.g., "rustc", "clippy"
}

pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}
```

Empty diagnostic lists clear the file entry (server resolved all issues).

## gRPC API

Extend `creep.proto` with an `LspService`:

```protobuf
service LspService {
    // Get all diagnostics for a workspace, optionally filtered by severity
    rpc GetDiagnostics(GetDiagnosticsRequest) returns (GetDiagnosticsResponse);

    // Get diagnostics for a specific file
    rpc GetFileDiagnostics(GetFileDiagnosticsRequest) returns (GetFileDiagnosticsResponse);

    // Go-to-definition at a position
    rpc GotoDefinition(GotoDefinitionRequest) returns (GotoDefinitionResponse);

    // Find all references to a symbol at a position
    rpc FindReferences(FindReferencesRequest) returns (FindReferencesResponse);
}

message GetDiagnosticsRequest {
    string workspace_path = 1;
    DiagnosticSeverityFilter min_severity = 2;  // default: WARNING
    uint32 max_results = 3;                      // default: 50
}

message GetDiagnosticsResponse {
    repeated Diagnostic diagnostics = 1;
    uint32 total_count = 2;                      // total before limit
}

message Diagnostic {
    string file_path = 1;
    uint32 line = 2;
    uint32 column = 3;
    string severity = 4;
    string message = 5;
    string source = 6;
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

enum DiagnosticSeverityFilter {
    ALL = 0;
    ERROR = 1;
    WARNING = 2;
    INFO = 3;
    HINT = 4;
}
```

## creep-cli Commands

```bash
# Workspace diagnostics (errors and warnings by default)
creep diagnostics /path/to/workspace
creep diagnostics /path/to/workspace --severity error --json

# Single file diagnostics
creep diagnostics /path/to/workspace --file src/main.rs

# Go-to-definition
creep definition /path/to/file.rs:42:10

# Find references
creep references /path/to/file.rs:42:10 --include-declaration
```

### Markdown Output (default)

```markdown
## Workspace Diagnostics (3 errors, 2 warnings)

### Errors
- `src/api/auth.rs:42:5` — cannot find value `token` in this scope (rustc)
- `src/api/auth.rs:58:12` — mismatched types: expected `String`, found `&str` (rustc)
- `src/db/models.rs:15:1` — missing lifetime specifier (rustc)

### Warnings
- `src/main.rs:8:5` — unused import `std::io::Read` (rustc)
- `src/api/mod.rs:22:9` — variable does not need to be mutable (clippy)
```

This is what gets injected into the drone's system prompt or returned from the tool — compact, actionable, no noise.

## Integration with Native Drone

The native drone spec ([native-drone/00-overview.md](native-drone/00-overview.md)) already includes creep-cli as an external tool. LSP integration adds two new uses:

### 1. System Prompt Enrichment

The drone's `PromptBuilder` includes a diagnostics section (priority 140, between project context and constraints):

```
Priority 140:
  Diagnostics — current workspace errors/warnings from Creep LSP
```

At turn start, the drone calls `creep diagnostics <workspace> --severity warning --json`, parses the result, and injects it as a prompt section. The agent starts every turn knowing what's broken.

### 2. On-demand Symbol Lookup

The `creep-definition` and `creep-references` external tools let the agent look up symbols when needed:

```toml
# In drone.toml
[tools.external.creep-diagnostics]
binary = "creep-cli"
args = ["diagnostics"]
description = "Get current compiler errors and warnings for the workspace"
input_schema_path = "tools/creep-diagnostics.schema.json"
permission = "read-only"
output_format = "markdown"

[tools.external.creep-definition]
binary = "creep-cli"
args = ["definition"]
description = "Find where a symbol is defined (file:line:column)"
input_schema_path = "tools/creep-definition.schema.json"
permission = "read-only"
output_format = "markdown"

[tools.external.creep-references]
binary = "creep-cli"
args = ["references"]
description = "Find all references to a symbol (file:line:column)"
input_schema_path = "tools/creep-references.schema.json"
permission = "read-only"
output_format = "markdown"
```

## LSP Client Implementation

The LSP client in Creep handles the JSON-RPC 2.0 protocol over stdio. Patterns taken from Claw's `LspClient`:

### Message Framing

Standard LSP framing: `Content-Length: N\r\n\r\n{json}`. Parse incoming frames, route notifications vs responses.

### Request Tracking

```rust
pub struct LspClient {
    process: Child,
    stdin: BufWriter<ChildStdin>,
    pending: HashMap<i64, oneshot::Sender<Value>>,
    next_id: AtomicI64,
    diagnostics: Arc<DiagnosticsCache>,
}
```

Requests get a unique ID. A background reader task parses responses and routes them to the matching `oneshot::Sender`. Notifications (`publishDiagnostics`) are handled inline by the reader.

### Initialization

```
1. Send initialize(processId, rootUri, capabilities)
2. Wait for InitializeResult
3. Send initialized notification
4. Server begins sending diagnostics
```

Capabilities requested: `textDocumentSync` (full sync), `definitionProvider`, `referencesProvider`. Nothing else — we don't need completions, hover, or code actions.

## Resource Constraints

On the Raspberry Pi:

- **rust-analyzer** is the heaviest concern. It can use 500MB+ RAM on large projects. Mitigation: configure `rust-analyzer.cargo.buildScripts.enable = false` and `rust-analyzer.procMacro.enable = false` in initialization options to reduce memory.
- **Idle timeout** prevents accumulation. Only one language server per language per workspace, killed after inactivity.
- **Diagnostics cache** is small — a few KB even for hundreds of diagnostics.
- **gRPC overhead** is negligible — Creep is already running.

## Scope

### In scope
- LSP server lifecycle management in Creep (spawn, initialize, shutdown, crash recovery)
- Passive diagnostic collection from `publishDiagnostics` notifications
- Document sync via Creep's existing file watcher
- `GetDiagnostics`, `GetFileDiagnostics`, `GotoDefinition`, `FindReferences` gRPC methods
- creep-cli commands: `diagnostics`, `definition`, `references`
- Configuration in `hatchery.toml`

### Out of scope (future)
- Code actions / auto-fix suggestions
- Completions
- Workspace symbols (covered by Creep's planned symbol index)
- Multi-root workspace support (one workspace root per registration)
- Incremental document sync (full sync is simpler and sufficient)
