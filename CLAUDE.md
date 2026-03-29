# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Kerrigan is a personal agentic development platform built around Claude Code. It runs on a Raspberry Pi with an AI HAT 2, using local inference for lightweight tasks and Claude Code for heavier work.

## Build System

**Buck2** is the primary build system. Rust toolchain must be on PATH (via rustup).

- **Build cortex:** `buck2 build root//src/cortex:cortex`
- **Run cortex:** `buck2 run root//src/cortex:cortex`
- **List all targets:** `buck2 targets root//...`
- **Clean:** `buck2 clean`

Cargo is still available for local dev convenience (`cargo check` / `cargo test` from `src/cortex/`), but Buck2 is authoritative for builds.

## Components

### Cortex (`src/cortex/`)
The foundational service. Rust binary (edition 2024). Responsible for:
- **RAG / context management** — retrieval-augmented generation for codebase and project context
- **Task storage** — persistent task tracking for agentic workflows
- **Artifact management** — storage and retrieval of build artifacts, outputs, and intermediate results

## Repo Layout

```
.buckconfig          # Buck2 cell/toolchain config (uses bundled prelude)
.buckroot            # Buck2 project root marker
toolchains/BUCK      # Toolchain definitions (system_demo_toolchains)
src/cortex/          # Cortex crate
  BUCK               # Buck2 build target (rust_binary)
  Cargo.toml         # Cargo manifest (local dev)
  src/main.rs        # Entry point
```

## Hardware Constraints

Target deployment is Raspberry Pi with AI HAT 2. Keep resource usage (memory, CPU, disk I/O) lean. Prefer efficient data structures and avoid unnecessary allocations.
