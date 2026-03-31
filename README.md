# Kerrigan

An agentic development platform. Runs on a Raspberry Pi with an AI HAT 2 for local inference, with Claude Code and other AI agents for heavier work.

## Why Kerrigan

LLM-powered coding agents are powerful but introduce a new class of supply chain risk. An agent that can write code, install packages, and execute commands is an ideal vector for prompt injection, dependency confusion, and subtle backdoors. The typical setup — an LLM with broad system access and whatever packages it decides to pull — is the supply chain attack surface that security teams have been warning about for years, now with an autonomous operator.

Kerrigan exists to put that power inside a controlled, auditable pipeline:

- **Hermetic builds** — Buck2 builds everything from pinned, hashed sources. No ambient system compilers, no implicit dependencies. What goes in is declared; what comes out is reproducible. Drone artifacts are self-contained binaries, not "install these 200 packages and hope for the best."
- **Auditable decisions** — Every agent action flows through Overseer's append-only decision log. What was decided, why, by which agent, with what context. Post-hoc forensics are built in, not bolted on.
- **Constrained agents** — Drones run from pre-built, immutable artifacts with declared tool sets. An agent can't install arbitrary packages or reach arbitrary services. The attack surface is the artifact definition, which is code-reviewed like any other change.
- **Standalone distribution** — Components compile to static binaries targeting specific platforms (x86_64, aarch64). No runtime dependency on system libraries, package managers, or container registries. Deploy by copying a file.
- **Evolution through process** — When the system identifies improvements (new tools, better workflows), they don't go live automatically. They become problem specs, flow to GitHub issues, and go through the same plan/dev/test/review cycle as any other change.

## Architecture

```
                    ┌─────────────────────────────┐
                    │       Overseer (k8s)        │
                    │  Memory · Jobs · Decisions  │
                    │  Artifacts · Embeddings     │
                    │    HTTP + MCP APIs          │
                    └──────────────┬──────────────┘
                                   │ HTTP/MCP
                    ┌──────────────┴───────────────┐
                    │                              │
              ┌─────┴──────┐            ┌──────────┴───┐
              │ Hatchery A │            │ Hatchery B   │
              │ (RPi)      │            │ (cloud VM)   │
              └─────┬──────┘            └──────────────┘
                    │
       ┌────────────┼────────────┐
       │            │            │
  ┌────┴────┐   ┌───┴────┐   ┌───┴──────┐
  │  Queen  │   │ Creep  │   │ Drone    │ (ephemeral)
  │  +Evo   │   │(sidecar│   │ sessions │
  │ Chamber │   │daemon) │   └──────────┘
  └─────────┘   └────────┘
```

### Overseer

Central service running in k8s. Persistent memory (vector search), job orchestration, decision audit log, and artifact storage. Exposes HTTP REST and MCP APIs. Supports SQLite (local) and PostgreSQL (production) backends.

### Hatchery

A deployment unit containing Queen, Evolution Chamber, and Creep. Multiple Hatcheries can register with Overseer and run on different hosts (RPi, cloud VMs, etc.).

**Queen** — Process manager. Spawns, monitors, and terminates drone sessions. Manages job state with Overseer. Handles notifications and operator chat. No LLM calls — pure systems engineering.

**Evolution Chamber** — Analysis engine sharing a process with Queen. Examines completed drone sessions through a three-stage pipeline: metric extraction, heuristic pattern detection, and targeted LLM analysis. Outputs problem specs routed to GitHub issues for the plan/dev/test cycle. Identifies opportunities for new tools, skills, MCP services, drone config changes, Creep enhancements, bug fixes, and process improvements.

**Creep** — Persistent IDE infrastructure sidecar. Manages language servers (LSP), caches ASTs (tree-sitter), and maintains semantic indexes across drone sessions. Exposes a gRPC API so drones get instant access to symbol lookups, diagnostics, dependency graphs, and file indexes without cold-starting expensive tooling.

### Drones

Self-contained, hermetic agent packages built by Buck2. Each drone is a complete, distributable artifact containing an agent runtime (Claude Code, Gemini CLI, local Pi inference, Kimi Code), pre-configured skills, MCP servers, and instructions. Queen deploys and launches them as ephemeral sessions.

Organized by agent type with shared bases and task-specific subtypes:

```
src/drones/
  claude/           gemini/          pi/             kimi/
    base/             base/            base/           base/
    code-reviewer/    quick-fix/       triage/         ...
    feature-builder/
```

## Communication

| Path | Protocol | Purpose |
|------|----------|---------|
| Queen <-> Overseer | HTTP | Job state, registration, heartbeat |
| Drones <-> Overseer | MCP / HTTP | Decisions, memories, tasks, artifacts |
| Drones <-> Creep | gRPC | LSP queries, AST lookups, cache reads |
| Queen <-> Creep | HTTP | Health checks |

## Build System

### Why Buck2

Most build systems trust the environment: whatever compiler is on PATH, whatever libraries are installed, whatever the network serves today. That's fine until an agent is assembling its own toolchain from the output — then ambient trust becomes ambient risk.

Buck2 gives us:

- **Hermeticity** — Toolchains are fetched as pinned, SHA256-verified archives. Builds don't touch system compilers. The same inputs produce the same outputs regardless of host state.
- **Content-addressed caching** — Build outputs are keyed by the hash of their inputs. Rebuilds only happen when something actually changed. Shared across machines via BuildBuddy remote execution.
- **Cross-compilation** — A single command targets x86_64 or aarch64. Drone artifacts for RPi are built on a developer laptop or CI worker without an emulator.
- **Custom rules** — `drone_package()` is a first-class build rule, not a shell script. Buck2 tracks its inputs (skills, MCP configs, agent binaries) and rebuilds only when they change.
- **Dependency governance** — Third-party crates are managed through Reindeer, generating deterministic BUCK files from a locked Cargo workspace. No transitive dependency surprises. Crates with build scripts require explicit fixups.

### Toolchains

Fully hermetic — no system compiler dependencies.

- **Rust:** nightly 1.96.0 (2026-03-28), edition 2024
- **C/C++:** LLVM 22.1.2 (clang/clang++/llvm-ar)
- **Remote execution:** BuildBuddy (optional, falls back to local)

### Commands

```sh
buck2 build root//src/overseer:overseer        # Build overseer
buck2 run root//src/overseer:overseer           # Run overseer
buck2 build root//...                           # Build everything
buck2 build root//src/overseer:overseer \
  --target-platforms platforms//:linux-aarch64  # Cross-compile for RPi
```

## Project Layout

```
src/
  overseer/          # Central service (Rust binary)
  queen/             # Hatchery binary — Queen + Evolution Chamber
  creep/             # Sidecar binary — LSP, AST, caching
  drones/            # Drone definitions and build rules
  proto/             # gRPC protobuf definitions (Creep API)
platforms/           # Buck2 execution platform definitions
toolchains/          # Hermetic Rust/LLVM toolchain definitions
prelude/             # Vendored Buck2 prelude (patched)
third-party/         # Reindeer-managed crate dependencies
tools/               # Dev tools (prek, reindeer, buckify)
docs/specs/          # Design specifications
```

## Configuration

Runtime config in `overseer.toml`. Database backend selected by URL scheme (`sqlite://` or `postgres://`). Artifact storage via `file://` or `s3://`.

## Development

Which buck2?

```sh
gh release download 2026-01-19 --repo facebook/buck2 --pattern 'buck2-x86_64-unknown-linux-gnu.zst' -O - | zstd -d | sudo tee /usr/local/bin/buck2
```


```sh
# Pre-commit hooks
buck2 run root//tools:prek -- install
buck2 run root//tools:prek -- run --all-files

# Add a dependency
cargo add <crate> -p overseer
./tools/buckify.sh
# Add deps = ["//third-party:crate-name"] to BUCK file
```
