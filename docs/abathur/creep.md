---
title: Creep File Indexer
slug: creep
description: Persistent gRPC sidecar — file indexing, symbol parsing, watch-based updates, .gitignore aware
lastmod: 2026-04-05
tags: [creep, grpc, indexing, symbols]
sources:
  - path: src/creep/src/main.rs
    hash: ""
  - path: src/creep/src/service.rs
    hash: ""
  - path: src/creep/src/index.rs
    hash: ""
  - path: src/creep/src/watcher.rs
    hash: ""
  - path: src/creep/src/parser.rs
    hash: ""
  - path: src/creep/src/symbol_index.rs
    hash: ""
  - path: src/creep/src/config.rs
    hash: ""
  - path: src/creep/proto/creep.proto
    hash: ""
sections: [grpc-api, file-index, symbol-index, watcher, parser, configuration]
---

# Creep File Indexer

## gRPC API

Six RPC methods on `FileIndex` service:

| RPC | Request | Response |
|-----|---------|----------|
| `SearchFiles` | pattern (glob), workspace?, file_type? | `Vec<FileMetadata>` |
| `GetFileMetadata` | path | `FileMetadata` |
| `RegisterWorkspace` | path | files_indexed count |
| `UnregisterWorkspace` | path | confirmation |
| `SearchSymbols` | query, kind?, workspace? | `Vec<SymbolInfo>` |
| `ListFileSymbols` | path | `Vec<SymbolInfo>` |

**FileMetadata:** path, size, modified_at (i64), file_type, content_hash (blake3).
**SymbolInfo:** name, kind, file, line, end_line, parent (optional), signature (optional).

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

## Watcher

Debounced (500ms) file system monitoring via `notify` crate. Per-workspace watchers with gitignore matchers.

- `watch(workspace)` — starts watcher, builds gitignore matcher
- `unwatch(workspace)` — stops watching
- `is_ignored(path)` — checks against all workspace gitignore matchers

Events: `Created`, `Modified`, `Removed`. Async `process_events()` task consumes events and updates both file and symbol indexes.

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
| `symbol_index` | false |
| `languages` | ["rust"] |

Port also overridable via `CREEP_PORT` env var.
