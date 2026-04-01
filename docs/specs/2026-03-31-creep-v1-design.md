# Creep v1 Design

## Overview

Creep is a persistent IDE infrastructure sidecar that runs alongside Queen in the Hatchery. It maintains a file index across drone sessions so drones get fast file lookups without cold-starting expensive tooling.

v1 covers: gRPC server skeleton, file index with metadata and content hashing, filesystem watcher, workspace registration. LSP management and AST parsing are deferred to v2/v3.

## Architecture

```
┌─────────────────────────────────────┐
│           Creep (sidecar)           │
│                                     │
│  ┌──────────────┐  ┌────────────┐   │
│  │  gRPC Server │  │   File     │   │
│  │  (tonic)     │  │  Watcher   │   │
│  │              │  │  (notify)  │   │
│  │  FileIndex   │  └─────┬──────┘   │
│  │  Service     │        │          │
│  │              │  ┌─────▼──────┐   │
│  │  Health      │  │   Index    │   │
│  │  Service     │  │ (RwLock<   │   │
│  │              │  │  HashMap>) │   │
│  └──────┬───────┘  └─────▲──────┘   │
│         │                │          │
│         └────────reads───┘          │
└─────────────────────────────────────┘
```

**Three components:**
- **gRPC Server** — tonic server exposing `FileIndex` service + standard `grpc.health.v1.Health` service. Drones connect here. Queen health-checks via the standard gRPC health protocol.
- **File Watcher** — `notify` crate watching registered directories. On change events, updates the index. Respects `.gitignore` via the `ignore` crate.
- **Index** — `Arc<RwLock<HashMap<PathBuf, FileMetadata>>>`. Watcher writes, gRPC reads. In-memory with no disk persistence for v1.

Single binary, single tokio runtime. gRPC and watcher run as concurrent tasks.

## gRPC Service Definition

```protobuf
syntax = "proto3";
package creep.v1;

service FileIndex {
  // Search files by glob pattern
  rpc SearchFiles(SearchFilesRequest) returns (SearchFilesResponse);

  // Get metadata for a specific file
  rpc GetFileMetadata(GetFileMetadataRequest) returns (FileMetadata);

  // Register a workspace directory for indexing
  rpc RegisterWorkspace(RegisterWorkspaceRequest) returns (RegisterWorkspaceResponse);

  // Unregister a workspace (stops watching)
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

Plus the standard `grpc.health.v1.Health` service via tonic's built-in health server.

## File Index

**FileMetadata (Rust):**
- `path: PathBuf`
- `size: u64`
- `modified_at: i64` (unix timestamp)
- `file_type: String` (detected from extension)
- `content_hash: String` (blake3)

**File type detection:** Extension mapping — `.rs` -> "rust", `.py` -> "python", `.toml` -> "toml", `.json` -> "json", `.md` -> "markdown", `.ts` -> "typescript", `.js` -> "javascript", `.go` -> "go", `.java` -> "java", `.c`/`.h` -> "c", `.cpp`/`.hpp` -> "cpp". Unknown extensions -> "unknown".

**Content hashing:** `blake3` — fast, single-pass. Hash on initial index and on change events.

**Index operations:**
- **Initial scan** — on startup (config paths) or on `RegisterWorkspace`. Uses the `ignore` crate's directory walker which respects `.gitignore`, `.git/info/exclude`, and global gitignore.
- **Watch** — `notify` watcher on each registered workspace. On create/modify: check against `ignore` matcher, re-index if not ignored. On delete: remove from index. On rename: remove old, index new.
- **Search** — iterate the index, match paths against the glob pattern. Filter by workspace and file_type if provided.

## Configuration

```toml
[creep]
grpc_port = 9090
workspaces = ["/home/jackm/repos/kerrigan"]
```

Minimal — port and default workspaces to index on startup. Drones register additional workspaces dynamically via the `RegisterWorkspace` RPC.

Queen's `hatchery.toml` `[creep]` section has `health_port` which becomes `grpc_port` (health service runs on the same gRPC port).

## Repo Structure

```
src/
  creep/
    Cargo.toml
    BUCK
    proto/
      creep.proto         # FileIndex service definition
    build.rs              # prost/tonic codegen from proto
    src/
      main.rs             # Entry: load config, start watcher + gRPC server
      config.rs           # TOML config parsing
      index.rs            # Arc<RwLock<HashMap>> + scan/update/search operations
      watcher.rs          # notify file watcher -> index updates
      service.rs          # tonic FileIndex service impl (reads from index)
```

Creep is a new crate in the Cargo workspace with its own Buck2 target.

Key dependencies: `tonic`, `prost`, `tonic-build` (build dep), `tonic-health`, `notify`, `ignore`, `blake3`, `tokio`, `serde`, `toml`, `tracing`, `glob`.

## Integration with Queen

Queen's CreepManager actor spawns the Creep binary as a child process and health-checks it via the gRPC health protocol. Drones connect to Creep on localhost at the configured port. If Creep is unavailable, drones degrade gracefully — they just don't have fast file lookups.

## What This Does NOT Cover

- LSP management (v2)
- AST parsing / tree-sitter (v2)
- Semantic cache / dependency graphs (v3)
- Disk persistence for the index (future — restart resilience)
- Streaming file change notifications to drones (future)
