---
title: Creep CLI
slug: creep-cli
description: Thin gRPC client for Creep — file search, metadata, symbol lookup, workspace management, LSP diagnostics and navigation
lastmod: 2026-04-05
tags: [creep-cli, cli, grpc, lsp]
sources:
  - path: src/creep-cli/src/main.rs
    hash: 8ee1a3ff2d166ff50fe034c1d6667d2f60a989291999eca6205169eec57fb5bc
sections: [commands, output]
---

# Creep CLI

## Commands

Global flags: `--addr` (env: `CREEP_ADDR`, default: `http://localhost:9090`), `--json` (JSON output).

| Command | Arguments | Description |
|---------|-----------|-------------|
| `search <pattern>` | `--workspace`, `--type` | Find files by glob pattern |
| `metadata <path>` | | Get file metadata (size, hash, type, modified) |
| `register <path>` | | Register workspace for indexing, returns file count |
| `unregister <path>` | | Stop watching workspace |
| `symbols [query]` | `--file`, `--kind`, `--workspace` | Search symbols by name or list file symbols |
| `diagnostics <workspace>` | `--file`, `--severity` | Get LSP diagnostics (default severity: warning) |
| `definition <file:line:col>` | | Go to definition at location (1-indexed) |
| `references <file:line:col>` | `--include-declaration` | Find references at location (1-indexed) |

**Symbol kinds:** function, struct, enum, trait, impl, const, static, type_alias, module, macro.

**LSP commands** connect to the `LspService` gRPC service (separate from `FileIndex`). Location arguments use `file:line:column` format (1-indexed, converted to 0-indexed for LSP internally). The `rsplitn` parser handles colons in file paths (e.g. `C:/src/main.rs:10:1`).

## Output

Default: formatted tables. With `--json`: pretty-printed JSON.

File search shows: path, size (human-readable), modified time, type, hash (truncated 12 chars).
Symbol search shows: kind, name/signature, parent, file:line.
Diagnostics shows: markdown-formatted report grouped by severity (errors, warnings, info, hints) with `file:line:column -- message (source)` format.
Definition/references shows: `file:line:column` (1-indexed).
