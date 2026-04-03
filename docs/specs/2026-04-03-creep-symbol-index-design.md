# Creep Symbol Index (11a) Design

## Overview

Add structural code navigation to Creep: find symbol definitions by name, list all symbols in a file. Uses tree-sitter to parse Rust source files and extract symbols into an in-memory index alongside the existing file index. Drones get precise definition lookup without grep false positives and file outlines without reading entire files.

Scope: Rust language only. 10 symbol kinds. Two new gRPC RPCs. CLI subcommand. Skill update. No parse tree caching.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                 Creep (sidecar)                      │
│                                                     │
│  ┌──────────────┐  ┌────────────┐  ┌─────────────┐ │
│  │  gRPC Server │  │   File     │  │   Parser    │ │
│  │  (tonic)     │  │  Watcher   │  │ (tree-sitter│ │
│  │              │  │  (notify)  │  │  + queries) │ │
│  │  FileIndex   │  └─────┬──────┘  └──────┬──────┘ │
│  │  Service     │        │                │        │
│  │  + Symbol    │  ┌─────▼──────┐  ┌──────▼──────┐ │
│  │    RPCs      │  │  FileIndex │  │ SymbolIndex │ │
│  │              │  │ (HashMap)  │  │ (by_file +  │ │
│  │  Health      │  │            │  │  by_name)   │ │
│  │  Service     │  └────────────┘  └─────────────┘ │
│  └──────────────┘                                   │
└─────────────────────────────────────────────────────┘
```

**New components:**

- **Parser module** (`src/creep/src/parser.rs`) — loads tree-sitter-rust grammar, runs S-expression queries against file content, returns `Vec<Symbol>`. Stateless: no cached parse trees. Reparsing a 1000-line file takes ~200us, far cheaper than caching trees (~1MB per file) on RPi.
- **SymbolIndex** (`src/creep/src/symbol_index.rs`) — parallel to FileIndex. Two maps: `by_file: HashMap<PathBuf, Vec<Symbol>>` for file listing, `by_name: HashMap<String, Vec<SymbolRef>>` inverted index for name search.
- **Two new gRPC RPCs** — `SearchSymbols` and `ListFileSymbols`, added to the existing `FileIndex` service (backwards compatible).

**Integration with existing code:**

- Watcher's `process_events` calls parser on file change, updates SymbolIndex
- `RegisterWorkspace` triggers symbol scan alongside existing file scan
- Config gets `symbol_index` (bool, default true) and `languages` (list, default `["rust"]`)

## Symbol Data Model

```rust
pub enum SymbolKind {
    Function, Struct, Enum, Trait, Impl,
    Const, Static, TypeAlias, Module, Macro,
}

pub struct Symbol {
    name: String,              // "process_events", "FileIndex"
    kind: SymbolKind,
    line: u32,                 // 0-indexed start line
    end_line: u32,             // 0-indexed end line
    parent: Option<String>,    // enclosing scope name
    signature: Option<String>, // functions only: "fn foo(x: i32) -> bool"
}

pub struct SymbolRef {
    file: PathBuf,
    line: u32,
    kind: SymbolKind,
}
```

**Parent scoping rules:**

- Methods inside `impl Foo` get `parent = Some("Foo")`
- Items inside `mod bar` get `parent = Some("bar")`
- Top-level items get `parent = None`
- Only one level of nesting tracked (no `mod::impl::fn` chains)

**Signatures** extracted for functions/methods only. Format: `fn name(params) -> ReturnType`. No visibility prefix, no where clauses — just the callable shape.

**Impl blocks** indexed as symbols (kind = Impl, name = type name) so drones can search for "what's implemented on type X."

## gRPC API

Two new RPCs added to the existing `FileIndex` service:

```protobuf
rpc SearchSymbols(SearchSymbolsRequest) returns (SearchSymbolsResponse);
rpc ListFileSymbols(ListFileSymbolsRequest) returns (ListFileSymbolsResponse);

message SymbolInfo {
  string name = 1;
  string kind = 2;              // "function", "struct", "enum", "trait", "impl",
                                // "const", "static", "type_alias", "module", "macro"
  string file = 3;              // absolute file path
  uint32 line = 4;              // 0-indexed start line
  uint32 end_line = 5;          // 0-indexed end line
  optional string parent = 6;   // enclosing scope name
  optional string signature = 7; // functions only
}

message SearchSymbolsRequest {
  string query = 1;             // substring match, case-insensitive
  optional string kind = 2;     // filter by symbol kind string
  optional string workspace = 3; // filter by workspace path
}

message SearchSymbolsResponse {
  repeated SymbolInfo symbols = 1;
}

message ListFileSymbolsRequest {
  string path = 1;              // absolute file path
}

message ListFileSymbolsResponse {
  repeated SymbolInfo symbols = 1;
}
```

**SearchSymbols** — substring match on name, case-insensitive. Optional kind and workspace filters. Returns all matches (no pagination — workspace-scoped results are small enough).

**ListFileSymbols** — all symbols in a file, ordered by line number. The "outline view."

No changes to RegisterWorkspace/UnregisterWorkspace proto — symbol scanning triggered internally.

## tree-sitter Query Design

A single S-expression query captures all 10 symbol kinds:

```scheme
;; Functions (top-level and methods)
(function_item name: (identifier) @name) @definition

;; Structs, enums, traits
(struct_item name: (type_identifier) @name) @definition
(enum_item name: (type_identifier) @name) @definition
(trait_item name: (type_identifier) @name) @definition

;; Impl blocks — capture the type
(impl_item type: (_) @name) @definition

;; Constants, statics, type aliases
(const_item name: (identifier) @name) @definition
(static_item name: (identifier) @name) @definition
(type_item name: (type_identifier) @name) @definition

;; Modules, macros
(mod_item name: (identifier) @name) @definition
(macro_definition name: (identifier) @name) @definition
```

**Post-processing after query matches:**

- Determine `SymbolKind` from the matched `@definition` node's `kind()` string
- For functions: extract `parameters` and `return_type` child nodes to build signature
- For impl methods: walk up to find enclosing `impl_item`, extract its type name as `parent`
- For mod items: track nesting to set `parent` on contained items

**Language dispatch:** `parse_symbols(content, language)` entry point. Unsupported languages return empty vec. Adding a language later means adding a grammar crate and a query string — no architectural changes.

## CLI Interface

New `symbols` subcommand for `creep-cli`:

```bash
# Search by name (substring, case-insensitive)
creep-cli symbols "process"
creep-cli symbols "Config" --kind struct
creep-cli symbols "handler" --workspace /path/repo

# List all symbols in a file (outline view)
creep-cli symbols --file /path/to/file.rs

# JSON output for machine parsing
creep-cli symbols "process" --json
```

**Human-readable search output:**

```
function   fn process_events(index: FileIndex, ...)   src/creep/src/watcher.rs:126
function   fn process_files(root: &Path) -> Result    src/creep/src/index.rs:97
```

**Human-readable file listing (outline) output:**

```
   1  struct     FileMetadata
  17  struct     FileIndex
  22  function   fn new() -> Self
  28  function   fn scan_workspace(&self, path: impl AsRef<Path>) -> Result<u64>
  96  function   fn scan_directory(root: &Path) -> Result<Vec<FileMetadata>>
 113  function   fn index_file(path: &Path) -> Result<FileMetadata>
 135  function   fn detect_file_type(path: &Path) -> String
```

Line numbers displayed as 1-indexed in CLI output (stored 0-indexed internally).

## Incremental Updates & Lifecycle

**On workspace registration (`RegisterWorkspace` RPC):**

1. Existing file scan runs (unchanged)
2. Symbol scan runs in `spawn_blocking` — walks the same files, parses supported languages, populates SymbolIndex
3. Both counts logged: "indexed 342 files, parsed 127 symbols"

**On file change (watcher `process_events`):**

1. Existing `index.update_file()` runs (unchanged)
2. Then `symbol_index.reparse_file()` via `spawn_blocking` — reads file, parses, replaces that file's symbols in both maps
3. On file removal: `symbol_index.remove_file()` cleans both maps

**On workspace unregister:** same as v1 — stops watching, indexed data (files + symbols) remains until process restarts.

**Reparse logic in SymbolIndex:**

- Acquire both `by_file` and `by_name` write locks
- Remove old name index entries for the file
- Parse fresh content, insert new entries into both maps
- Single-file reparse is fast enough to hold locks without contention

**Config gating:** `symbol_index = false` skips scanning and reparsing. RPCs still exist but return empty results. Lets users disable on resource-constrained setups without changing proto/CLI.

## Configuration

New fields in `[creep]` section of `creep.toml`:

```toml
[creep]
grpc_port = 9090
workspaces = ["/path/to/repo"]
symbol_index = true       # default: true
languages = ["rust"]      # default: ["rust"]
```

## Skill & Drone Integration

Update existing `creep-discovery` SKILL.md (no new skill file):

**New "When to Use" entries:**

- Finding function/struct/trait definitions by name — faster and more precise than Grep (excludes comments, strings, variable names)
- Getting a structural overview of a file before reading it (outline view)
- Understanding what's implemented on a type — search for impl blocks

**New "When NOT to Use" entries:**

- Finding call sites / references (use Grep — symbol index finds definitions only)
- Searching file contents (use Grep)
- Reading actual code (use Read — symbols give the map, not the territory)

**Fallback** (unchanged): if `creep-cli symbols` fails with a connection error, drones fall back to Grep.

## Resource Estimates

- **Memory:** ~10-15 MB for a typical workspace (Kerrigan-sized). The inverted name index adds ~2-3 MB on top of the per-file symbol lists.
- **CPU:** tree-sitter parse of a single file: ~200us. Full workspace scan (500 Rust files): ~100ms. Negligible on RPi.
- **Disk:** Zero. In-memory only, no persistence.

## Files Changed

| Action | Path |
|--------|------|
| Create | `src/creep/src/parser.rs` |
| Create | `src/creep/src/symbol_index.rs` |
| Modify | `src/creep/proto/creep.proto` |
| Modify | `src/creep/src/main.rs` |
| Modify | `src/creep/src/service.rs` |
| Modify | `src/creep/src/watcher.rs` |
| Modify | `src/creep/src/config.rs` |
| Modify | `src/creep/Cargo.toml` |
| Modify | `src/creep/BUCK` |
| Regen  | `src/creep/proto_gen/creep.v1.rs` |
| Regen  | `src/creep-cli/proto_gen/creep.v1.rs` |
| Modify | `src/creep-cli/src/main.rs` |
| Modify | `src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md` |

## What This Spec Does NOT Cover

- Additional languages (11d — separate spec)
- Context extraction / enclosing scope queries (11b — separate spec)
- Call graph / relationship graph (11c — separate spec)
- LSP proxy (12 — separate spec)
- Pagination or result limits (not needed at workspace scale)
- Persistence / disk-backed symbol index (not needed — scan is fast)
