---
title: Creep File Indexer
slug: creep
description: Persistent gRPC sidecar — file indexing, symbol parsing, LSP integration, watch-based updates, .gitignore aware
lastmod: 2026-04-06
tags: [creep, grpc, indexing, symbols, lsp]
sources:
  - path: src/creep/src/main.rs
    hash: 45a5b625ebbac7801f7a32044af9c4be9aeb2e005cc7eb0e4cd00b344645359f
  - path: src/creep/src/service.rs
    hash: 1461641263f38362e473905575646dcbaf03298ca4ba8433694c617b86c8a9a9
  - path: src/creep/src/index.rs
    hash: 0526049c89eb55f533281b18a40fd15f1ac6b2eba1cfdc4a4d53cc01765208a4
  - path: src/creep/src/watcher.rs
    hash: f36b3577258d892220e38a549900d95658746592e9dda5f1c23c26150f5b8283
  - path: src/creep/src/parser.rs
    hash: ff5c00e8a466fa65dd14fd76bab2239a16ead9db47db0aeaf0d71449f886a7ac
  - path: src/creep/src/symbol_index.rs
    hash: bf2e61f7ace6fd8e47dcbc59de5bad693c8cf664a7b0a2cada3e706612e307f1
  - path: src/creep/src/config.rs
    hash: 89c3b49dc17169fc57f8ab26da21eb3d045fc1897068db982a1602f33be79810
  - path: src/creep/proto/creep.proto
    hash: 36d9c864db4a6e786fce872be8b250204b3f758525efabe44d5c218e8f1d94ab
  - path: src/creep/src/lsp/client.rs
    hash: 2ab255fc197ac9fbfd760b1c957a9aa78657ddec1053a9cc69956b704c0ed50a
  - path: src/creep/src/lsp/manager.rs
    hash: 06bd54c264f44a232eb71d2e4c14b16c12259b36e6507a37dc2ebad671b49afe
  - path: src/creep/src/lsp/diagnostics.rs
    hash: 945b614a2a349fc2aaae86767218e765bf529ccfe0755c745a4bcbd17d2bc572
  - path: src/creep/src/lsp_service.rs
    hash: 3ec91b7d87049710f4bd79b0868d602502b2e110af865de27daf8babfdb27fd1
sections: [grpc-api, file-index, symbol-index, lsp-integration, watcher, parser, configuration]
---

# Creep File Indexer

## gRPC API

Two gRPC services: `FileIndex` (file/symbol operations) and `LspService` (LSP-backed diagnostics and navigation).

**FileIndex** — six RPC methods:

| RPC | Request | Response |
|-----|---------|----------|
| `SearchFiles` | pattern (glob), workspace?, file_type? | `Vec<FileMetadata>` |
| `GetFileMetadata` | path | `FileMetadata` |
| `RegisterWorkspace` | path | files_indexed count |
| `UnregisterWorkspace` | path | confirmation |
| `SearchSymbols` | query, kind?, workspace? | `Vec<SymbolInfo>` |
| `ListFileSymbols` | path | `Vec<SymbolInfo>` |

**LspService** — four RPC methods:

| RPC | Request | Response |
|-----|---------|----------|
| `GetDiagnostics` | workspace_path, min_severity (1-4), max_results | `Vec<Diagnostic>`, total_count |
| `GetFileDiagnostics` | workspace_path, file_path | `Vec<Diagnostic>` |
| `GotoDefinition` | file_path, line, column (0-indexed) | `Vec<SymbolLocation>` |
| `FindReferences` | file_path, line, column, include_declaration | `Vec<SymbolLocation>` |

**FileMetadata:** path, size, modified_at (i64), file_type, content_hash (blake3).
**SymbolInfo:** name, kind, file, line, end_line, parent (optional), signature (optional).
**Diagnostic:** file_path, line, column, severity (string), message, source.
**SymbolLocation:** file_path, start_line, start_column, end_line, end_column.

Health checking via `tonic_health` — marks `FileIndex` as serving on startup.

## File Index

`FileIndex` wraps `Arc<RwLock<HashMap<PathBuf, FileMetadata>>>`.

- `scan_workspace(path)` — recursive walk respecting `.gitignore` (via `ignore` crate), blake3 content hashing, returns file count
- `update_file(path)` — incremental single-file index update
- `remove_file(path)` — remove from index
- `search(pattern, workspace, file_type)` — glob matching via `glob_match`
- `get(path)` — direct lookup

File type detection maps extensions: `.rs`→"rust", `.py`→"python", `.ts`→"typescript", etc.

## Symbol Index

Inverted index for fast symbol lookup:

- `by_file: HashMap<PathBuf, Vec<Symbol>>` — symbols per file
- `by_name: HashMap<String, Vec<SymbolRef>>` — inverted name→locations index

Methods:
- `reparse_file(path)` — synchronous (call from `spawn_blocking`), reads file, detects language, parses symbols, updates both indexes
- `remove_file(path)` — removes from both indexes
- `search(query, kind, workspace)` — case-insensitive substring match, filters by kind/workspace
- `list_file_symbols(path)` — all symbols for a file, sorted by line
- `scan_workspace(root)` — synchronous, recursively parses all supported files

## LSP Integration

Creep manages external LSP server processes to provide diagnostics, go-to-definition, and find-references.

### LspClient

Spawns an LSP server as a child process communicating via stdio JSON-RPC. One client per (server, workspace) pair.

- `connect(config, workspace)` — spawns the server process, pipes stdin/stdout/stderr
- `initialize(workspace_root)` — performs LSP initialize handshake, sends `initialized` notification
- `request(method, params)` — sends JSON-RPC request, waits for response with 30s timeout
- `notify(method, params)` — sends JSON-RPC notification (no response expected)
- `open_document(path, content, language_id)` — `textDocument/didOpen`
- `change_document(path, content, version)` — `textDocument/didChange` (full content sync)
- `close_document(path)` — `textDocument/didClose`
- `goto_definition(file, line, column)` — `textDocument/definition`, returns `Vec<SymbolLocation>`
- `find_references(file, line, column, include_declaration)` — `textDocument/references`
- `shutdown()` — sends shutdown + exit, waits up to 5s for graceful exit, then kills

Background reader task parses JSON-RPC frames from stdout: routes responses to pending oneshot channels, handles `textDocument/publishDiagnostics` notifications by updating the diagnostics cache. Stderr is piped to `tracing::warn` with the server name for debugging.

### DiagnosticsCache

Thread-safe `BTreeMap<PathBuf, Vec<LspDiagnostic>>` behind `Arc<RwLock>`. Updated by the reader task on `publishDiagnostics` notifications. Empty diagnostic lists remove the file entry (server cleared the diagnostics).

Severity levels: Error (1), Warning (2), Info (3), Hint (4). `get_all(min_severity)` filters by severity threshold.

### LspManager

Manages LSP server lifecycle across workspaces. Maintains an `extension_map` mapping file extensions to server names, and a `clients` map keyed by `"server_name:workspace_path"`.

- `ensure_server(workspace, file_ext)` — lazily starts and initializes a server if not running
- `get_client(workspace, file_ext)` — returns existing client without starting
- `find_workspace_for_file(path)` — finds which registered workspace contains a file
- `diagnostics(workspace, min_severity)` — aggregates diagnostics across all servers for a workspace
- `file_diagnostics(file)` — diagnostics for a single file across all servers
- `shutdown_all()` — gracefully shuts down all managed servers

### LspService (gRPC)

`LspServiceImpl` wraps `Arc<Mutex<LspManager>>` and implements the `LspService` gRPC trait. `GotoDefinition` and `FindReferences` auto-start the appropriate LSP server via `ensure_server()`. Returns `NOT_FOUND` if no workspace is registered, `UNIMPLEMENTED` if no server handles the file extension.

## Watcher

Debounced (500ms) file system monitoring via `notify` crate. Per-workspace watchers with gitignore matchers.

- `watch(workspace)` — starts watcher, builds gitignore matcher
- `unwatch(workspace)` — stops watching
- `is_ignored(path)` — checks against all workspace gitignore matchers

Events: `Created`, `Modified`, `Removed`. Async `process_events()` task consumes events and updates file index, symbol index, and LSP servers. For create/modify events on files with LSP-handled extensions, sends `didOpen` or `didChange` notifications. For remove events, sends `didClose`. This wiring ensures LSP servers receive document sync notifications that trigger `publishDiagnostics` updates.

## Parser

Tree-sitter-based symbol extraction. Currently only Rust is supported.

**Symbol kinds:** Function, Struct, Enum, Trait, Impl, Const, Static, TypeAlias, Module, Macro.

- `parse_symbols(content, language)` — dispatches to language-specific parser
- `parse_rust_symbols(content)` — S-expression query over tree-sitter parse tree
- `find_parent_scope(node)` — finds enclosing `impl` or `mod` block
- `build_function_signature(node)` — extracts `fn name(params) -> ReturnType`

## Configuration

`creep.toml`:

| Key | Default |
|-----|---------|
| `grpc_port` | 9090 |
| `workspaces` | [] |
| `symbol_index` | true |
| `languages` | ["rust"] |
| `lsp.<name>.command` | (required) |
| `lsp.<name>.args` | [] |
| `lsp.<name>.extensions` | (required) |
| `lsp.<name>.language_id` | (required) |

Port also overridable via `CREEP_PORT` env var.

LSP server example in `creep.toml`:
```toml
[creep.lsp.rust]
command = "rust-analyzer"
args = []
extensions = [".rs"]
language_id = "rust"
```
