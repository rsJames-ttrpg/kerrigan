---
title: Creep File Indexer
slug: creep
description: Persistent gRPC sidecar — file indexing, symbol parsing, watch-based updates, .gitignore aware
lastmod: 2026-04-05
tags: [creep, grpc, indexing, symbols]
sources:
  - path: src/creep/src/main.rs
    hash: 3968a2a16c5961a80b537ede011b48c2f87d6f01f9e760b740db1c210838e1f3
  - path: src/creep/src/service.rs
    hash: 1461641263f38362e473905575646dcbaf03298ca4ba8433694c617b86c8a9a9
  - path: src/creep/src/index.rs
    hash: 0526049c89eb55f533281b18a40fd15f1ac6b2eba1cfdc4a4d53cc01765208a4
  - path: src/creep/src/watcher.rs
    hash: 4832ed93e8eb21464c31adbd307f8aefd2a739de9d99c7c75740c2d39698b0d4
  - path: src/creep/src/parser.rs
    hash: ff5c00e8a466fa65dd14fd76bab2239a16ead9db47db0aeaf0d71449f886a7ac
  - path: src/creep/src/symbol_index.rs
    hash: bf2e61f7ace6fd8e47dcbc59de5bad693c8cf664a7b0a2cada3e706612e307f1
  - path: src/creep/src/config.rs
    hash: c173f4114f75d612ee864ae98098375429c787b150f8e60dad41a86a70d77d29
  - path: src/creep/proto/creep.proto
    hash: 3236a2122c3e1140ef9ab81231956813a7ab4b4a318e96f9e77e113a68fcaf1f
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
