# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Kerrigan is a personal agentic development platform built around Claude Code. It runs on a Raspberry Pi with an AI HAT 2, using local inference for lightweight tasks and Claude Code for heavier work.

## Build System

**Buck2** (`2026-01-19` release) is the primary build system with hermetic toolchains (no system rustc dependency). Pinned to this version because `2026-03-15` has a sqlite materializer bug (panics on duplicate inserts for directory outputs).

- **Build overseer:** `buck2 build root//src/overseer:overseer`
- **Run overseer:** `buck2 run root//src/overseer:overseer`
- **List all targets:** `buck2 targets root//...`
- **Clean:** `buck2 clean`

Cargo is still available for local dev convenience (`cargo check` / `cargo test` from `src/overseer/`), but Buck2 is authoritative for builds.

### Remote Execution (BuildBuddy)

Builds can execute remotely on BuildBuddy workers for shared caching and faster CI.

- **Auth:** Set `export BUCK2_RE_HTTP_HEADERS="x-buildbuddy-api-key:<KEY>"` in your shell. No secrets in `.buckconfig`.
- **Enable/disable:** `[project] remote_enabled = true` in `.buckconfig`. Set to `false` for local-only builds.
- **Fallback:** When `BUCK2_RE_HTTP_HEADERS` is unset or BuildBuddy is unreachable, builds fall back to local execution automatically (`local_enabled = True` in hybrid mode).
- **Container image:** Workers use `gcr.io/flame-public/rbe-ubuntu24-04:latest`. Must have glibc >= host system (build scripts are compiled locally, uploaded, and run on workers).

### Vendored Prelude

The Buck2 prelude is vendored in `prelude/` (not using `bundled`). Patches applied on top of the `2026-01-19` tag:

- `prelude/rust/tools/transitive_dependency_symlinks.py` — `mkdir(exist_ok=True)` + symlink force-replace (upstream bug)
- `prelude/rust/tools/{buildscript_run,rustc_action,extract_link_action}.py` — `from __future__ import annotations` (Python 3.8 compat for older worker images)
- `prelude/rust/cargo_buildscript.bzl` — added `OPT_LEVEL` and `DEBUG` env vars (cc-rs build scripts need them)

**Do not switch back to `prelude = bundled`** without verifying the upstream bugs are fixed.

### Starlark gotchas

- `read_config()` in a non-root cell (like `platforms/`) reads that cell's config, not root `.buckconfig`. Use `read_root_config()` instead.
- `[buck2_re_client]` is consumed by the binary — not readable from Starlark via `read_config()`/`read_root_config()`.
- `rustc_flags` must use `attrs.string()` not `attrs.arg()` — the prelude calls `.startswith()` on them.

## Components

### Overseer (`src/overseer/`)
The foundational service. Rust binary (edition 2024). Layered monolith: HTTP (axum) + MCP (rmcp) over a shared service layer, backed by a pluggable database and object store.

- **Memory** — vector-based semantic storage and retrieval; supports sqlite-vec (local) and pgvector (Postgres)
- **Jobs** — definitions, runs (with sub-jobs), and tasks for agent workflow orchestration
- **Decisions** — append-only audit log of agent decisions with context and reasoning
- **Artifacts** — metadata in DB, blobs via object_store (local filesystem, S3, etc.)

Backend selected via URL in config: `database_url` (sqlite:// or postgres://) and `artifact_url` (file:// or s3://). HTTP API on port 3100, MCP via stdio or HTTP. Config: `overseer.toml`. See `src/overseer/CLAUDE.md` for detailed architecture.

### Queen (`src/queen/`)
Hatchery process manager. Actor-based (tokio tasks + mpsc channels). Registers with Overseer, manages drone lifecycles, polls for jobs, health-checks drones and Creep. Config: `hatchery.toml`.

- **Build:** `buck2 build root//src/queen:queen`
- **Test:** `cd src/queen && cargo test`

### Creep (`src/creep/`)
Persistent file-indexing gRPC sidecar. tonic server with FileIndex service + standard health checking. Indexes workspaces respecting .gitignore, watches for changes via notify, blake3 content hashing.

- **Build:** `buck2 build root//src/creep:creep`
- **Test:** `cd src/creep && cargo test`
- **Proto:** `src/creep/proto/creep.proto` — codegen via tonic-build in build.rs. Pre-generated Rust in `proto_gen/` for Buck2.

### drone-sdk (`src/drone-sdk/`)
Shared library for drone binaries. Defines JSON-line protocol (QueenMessage/DroneMessage), DroneRunner trait (setup/execute/teardown), and harness entrypoint.

### Claude Drone (`src/drones/claude/base/`)
First concrete drone. Self-extracting binary embedding user-level Claude Code config at compile time. Creates isolated temp home, clones repo, spawns `claude` CLI.

## Toolchains

Fully hermetic Rust + C/C++ toolchains — no system compiler dependencies.

- **rust** — hermetic nightly 1.96.0 (2026-03-28), edition 2024, nightly features enabled
- **cxx** — hermetic LLVM 22.1.2 (clang/clang++/llvm-ar), defined in `toolchains/cxx_dist.bzl`
- **genrule** — generic build rules
- **python_bootstrap** — required by Rust prelude internals

To update Rust: change `RUST_NIGHTLY` date and SHA256 hashes in `toolchains/BUCK`.
To update LLVM: change `LLVM_VERSION` and SHA256 in `toolchains/BUCK`.
Default edition and nightly features are set in the toolchain — don't override per-target.

## Platforms

Defined in `platforms/BUCK`:
- **default** — hybrid local+remote execution platform (host CPU/OS constraints, BuildBuddy RE when configured)
- **linux-x86_64** — explicit x86_64 Linux target
- **linux-aarch64** — RPi cross-compilation target

The default platform must have CPU/OS constraints populated or `select()` in the prelude's `cargo_package.bzl` can't resolve platform-specific deps (like `libc`). Empty constraints = everything falls to `DEFAULT: None`.

Cross-compile with: `buck2 build root//src/overseer:overseer --target-platforms platforms//:linux-aarch64`

## Dependencies

Hybrid Cargo workspace + **reindeer** (hermetic). The workflow:

1. `cargo add <crate>` in the crate directory (e.g. `src/overseer/`)
2. `./tools/buckify.sh` to regenerate `third-party/BUCK` (NOT raw `reindeer buckify` — the wrapper fixes a rule ordering bug)
3. Add `deps = ["//third-party:crate-name"]` to the crate's BUCK file

- Root `Cargo.toml` is a workspace with all crates as members
- `reindeer.toml` uses `manifest_path = "Cargo.toml"` (hybrid mode — reads deps from workspace)
- `third-party/BUCK` is generated — do not edit by hand (gitignored)
- Crates with `build.rs` need fixups in `third-party/fixups/<crate>/fixups.toml`
- Reindeer skips `[dev-dependencies]` — test-only crates must go in `[dependencies]`
- Crates with C build scripts (like `libsqlite3-sys`, `ring`) need fixups with `[buildscript.run]` including `rustc_link_lib = true` and `rustc_link_search = true`
- Crates using tonic-build proto codegen: `build.rs` runs via Cargo, pre-generated Rust files committed to `proto_gen/` for Buck2 (Buck2 doesn't set `OUT_DIR` the same way)
- The `tools/buckify.sh` wrapper runs reindeer then fixes `buildscript_run`/`http_archive` ordering (reindeer bug: `rule_exists()` needs forward declarations)

### Adding a New Crate

1. Create `src/<name>/Cargo.toml` and `src/<name>/BUCK`
2. Add `"src/<name>"` to workspace `members` in root `Cargo.toml`
3. `./tools/buckify.sh` to regenerate `third-party/BUCK` with new deps
4. If Buck2 build fails on a new dep's build script, add `third-party/fixups/<crate>/fixups.toml` with `[buildscript] run = true`
5. For first-party crates with build.rs, set `env = {"CARGO_MANIFEST_DIR": "."}` in BUCK

Key crates added for multi-backend support: `sea-query` (SQL query builder), `async-trait`, `object_store` (filesystem/S3/GCS blobs), `pgvector` (Postgres vector type), `chrono` (timestamps).

## Pre-commit Hooks

**prek** (Rust-native pre-commit replacement, fetched hermetically by Buck2).

- **Install hooks:** `buck2 run root//tools:prek -- install && buck2 run root//tools:prek -- install --hook-type pre-push`
- **Run manually:** `buck2 run root//tools:prek -- run --all-files`
- **Config:** `prek.toml`

Hooks on pre-commit: trailing whitespace, end-of-file fixer, TOML check, merge conflict check, `cargo fmt --check`, clippy (hermetic via Buck2), cargo test, reindeer sync check.
Hooks on pre-push: `buck2 build root//...`, `buck2 test //...`

Clippy runs via `buck2 build 'root//src/overseer:overseer[clippy.txt]'` — the sub-target produces a text file. The hook fails if any warnings or errors appear.

## Repo Layout

```
Cargo.toml                # Workspace root (members = all src/* crates)
.buckconfig               # Buck2 cell/toolchain config (vendored prelude, RE config)
prelude/                  # Vendored Buck2 prelude (2026-01-19 tag, patched — do not edit casually)
.buckroot                 # Buck2 project root marker
reindeer.toml             # Reindeer config (hybrid mode, reads workspace Cargo.toml)
platforms/BUCK            # Target and execution platform definitions
toolchains/BUCK           # Toolchain definitions + hermetic Rust artifact downloads
toolchains/rust_dist.bzl  # hermetic_rust_toolchain rule (custom RustToolchainInfo)
prek.toml                 # Pre-commit hook config (prek)
tools/BUCK                # Dev tools (prek, reindeer) fetched hermetically by Buck2
third-party/              # Reindeer-managed crate dependencies
  BUCK                    # Generated by reindeer (do not edit)
  fixups/                 # Per-crate build script fixups
overseer.toml               # Overseer runtime config (port, database_url, artifact_url, embedding provider)
hatchery.toml               # Queen + Creep runtime config
src/overseer/               # Overseer crate (see src/overseer/CLAUDE.md for details)
  BUCK                    # Buck2 build target (rust_binary)
  Cargo.toml              # Crate manifest (deps go here via cargo add)
  src/main.rs             # Entry point
  src/{db,services,api,mcp,embedding,storage}/ # Layered architecture modules
  src/db/trait_def.rs     # Database trait (Arc<dyn Database>)
  src/db/sqlite.rs        # SQLite + sqlite-vec implementation
  src/db/postgres.rs      # PostgreSQL + pgvector implementation
  src/db/models.rs        # Shared domain model types
  src/db/tables.rs        # sea-query table/column enums
  migrations/sqlite/        # SQLite migrations (applied via sqlx::migrate!())
  migrations/postgres/      # PostgreSQL migrations (applied via sqlx::migrate!())
  src/storage.rs          # ObjectStore wrapper (local filesystem / S3)
src/queen/                  # Hatchery process manager (see above)
src/creep/                  # File-indexing gRPC sidecar (see above)
src/drone-sdk/              # Shared drone binary SDK
src/drones/claude/base/     # Claude Code drone binary
docs/specs/                 # Design specs
docs/plans/                 # Implementation plans
```

## Hardware Constraints

Target deployment is Raspberry Pi with AI HAT 2. Keep resource usage (memory, CPU, disk I/O) lean. Prefer efficient data structures and avoid unnecessary allocations.
