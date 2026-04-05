---
title: Creep CLI
slug: creep-cli
description: Thin gRPC client for Creep — file search, metadata, symbol lookup, workspace management
lastmod: 2026-04-05
tags: [creep-cli, cli, grpc]
sources:
  - path: src/creep-cli/src/main.rs
    hash: b9d2a583e28ab310f63e59d1223cf62f4541969e987c59af78cf2a2f00b65a52
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

**Symbol kinds:** function, struct, enum, trait, impl, const, static, type_alias, module, macro.

## Output

Default: formatted tables. With `--json`: pretty-printed JSON.

File search shows: path, size (human-readable), modified time, type, hash (truncated 12 chars).
Symbol search shows: kind, name/signature, parent, file:line.
