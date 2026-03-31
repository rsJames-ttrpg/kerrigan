# Kerrigan System Architecture Design

## Overview

Kerrigan is an agentic development platform. This document defines the high-level system architecture: the components, their responsibilities, how they communicate, and how they're deployed.

## System Topology

```
                    ┌─────────────────────────────┐
                    │       Overseer (k8s)         │
                    │  Memory · Jobs · Decisions    │
                    │  Artifacts · Embeddings       │
                    │    HTTP + MCP APIs            │
                    └──────────────┬───────────────┘
                                   │ HTTP/MCP
                    ┌──────────────┴───────────────┐
                    │                              │
              ┌─────┴──────┐            ┌──────────┴───┐
              │ Hatchery A │            │ Hatchery B   │  (multiple possible)
              │ (RPi)      │            │ (cloud VM)   │
              └─────┬──────┘            └──────────────┘
                    │
       ┌────────────┼────────────┐
       │            │            │
  ┌────┴────┐  ┌───┴────┐  ┌───┴──────┐
  │  Queen  │  │ Creep  │  │ Drone    │ (ephemeral)
  │  +Evo   │  │(sidecar│  │ sessions │
  │ Chamber │  │daemon) │  └──────────┘
  └─────────┘  └────────┘
```

**Three deployment units:**

- **Overseer** — central service, runs in k8s, stateful (DB + object store). Provides persistent memory, job orchestration, decision logging, and artifact storage. Exposes HTTP REST and MCP APIs.
- **Hatchery** — one or more instances, each containing Queen+Evolution Chamber (one binary) and Creep (sidecar process). Can run on RPi, cloud VMs, or any host.
- **Drones** — ephemeral agent sessions spawned by Queen. Self-contained, hermetic packages built by Buck2. Report back through Overseer.

## Components

### Queen

The process manager within the Hatchery binary. Pure systems engineering — no LLM calls.

**Responsibilities:**
- Drone lifecycle management: spawn, health check, terminate
- Drone registry: which drone configs exist, which sessions are active
- Job state coordination with Overseer: poll/receive jobs, update run status
- Notifications and chat relay for operator visibility
- Creep management: start as child process, health check, restart on failure
- Concurrency enforcement: limit active drones based on Hatchery resources

### Evolution Chamber

Analysis engine within the Hatchery binary (same process as Queen). Identifies improvement opportunities from drone session data.

**Three-stage pipeline:**

**Stage 1 — Metric extraction (no LLM):**
- Token/context usage per tool call
- Wall-clock time per phase
- Error counts, retry patterns
- Tool call frequency distributions
- Session outcome (success/failure/timeout)

**Stage 2 — Heuristic pattern detection (no LLM):**
- Repeated tool call sequences (candidate for a composite tool/skill)
- High context consumption phases (candidate for context reduction)
- Recurring failures across sessions (systemic issue)
- Expensive operations that could be served by Creep (cache opportunity)
- Drift detection — drone performing worse over time on similar tasks

**Stage 3 — LLM-assisted analysis (targeted):**
- Only flagged segments from Stage 2 get sent to an LLM
- Bounded context windows — never a full session dump
- Focused questions: "Is there a meaningful abstraction here?", "What's the root cause?"

**Output:** Problem spec documents submitted to Overseer as jobs/artifacts, routed to GitHub issues for the plan/dev/test cycle.

**Output categories:**
- New tool
- New skill
- New MCP service
- Drone config change
- Creep enhancement
- Kerrigan bug
- Process improvement

### Creep

Persistent IDE infrastructure layer. Runs as a sidecar daemon alongside the Queen+EvoChamber binary. Keeps expensive state warm so drones get instant access.

**Core services:**

- **LSP Manager** — spawns and manages language server processes (rust-analyzer, typescript-language-server, etc.) per workspace. Keeps them alive across drone sessions.
- **AST Index** — parses and caches ASTs for the codebase. File watcher keeps them fresh on change. Queryable for symbol definitions, references, call graphs, type info.
- **Semantic Cache** — higher-level derived data built on AST/LSP: dependency graphs, module boundaries, frequently accessed paths. Expensive to compute, changes infrequently.

**API (gRPC):**

```
creep/symbol_lookup    — find definition/references for a symbol
creep/ast_query        — query AST nodes by pattern
creep/diagnostics      — get current LSP diagnostics for a file/workspace
creep/dependency_graph — module/crate dependency relationships
creep/file_index       — fast file search with cached metadata
```

**Lifecycle:** Starts with the Hatchery, watches the workspace filesystem, stays alive as drones come and go. Idles when no drones are active but keeps indexes warm. Disk persistence for restart resilience.

**Implementation:** Rust binary, tree-sitter for fast AST parsing, tower-lsp or direct LSP protocol for language server management, tonic for gRPC, in-memory index with optional disk persistence.

### Drones

Self-contained, hermetic agent packages built by Buck2. A Drone is not a running process — it's a complete, distributable artifact containing everything needed to run an agent session.

**Drone lifecycle:**
1. **Define** — declarative spec in the repo
2. **Build** — Buck2 assembles the artifact (hermetic, cached, reproducible)
3. **Store** — artifact available via Overseer's store or Buck2 cache
4. **Deploy** — Queen pulls the artifact, unpacks, launches the session
5. **Run** — ephemeral session executes against bundled config
6. **Collect** — output/metrics go back to Overseer

**Organization by agent type:**

```
src/drones/
  claude/
    base/              # Shared Claude Code foundation (common MCP servers, base instructions)
    code-reviewer/     # Claude Code configured for reviews
    feature-builder/   # Claude Code configured for feature dev
  gemini/
    base/
    quick-fix/
  pi/                  # Local inference on AI HAT 2
    base/
    triage/            # Lightweight task routing/triage
  kimi/
    base/
    ...
  trait.rs             # Drone trait — common interface Queen uses regardless of agent type
  BUCK                 # drone_package() rule definition
```

**Key aspects:**
- Each agent type (claude, gemini, pi, kimi) has its own directory with a `base/` shared foundation
- Subtypes extend or override the base, layering on additional skills/config
- The Drone trait defines what Queen needs from any drone: spawn, health check, collect output, kill
- Each agent type implements the trait differently (different CLI invocations, config formats)
- Buck2 handles composition — a subtype target depends on its base
- A `drone_package()` Buck2 rule assembles the complete artifact

**Drone contents:**
- Agent config files (CLAUDE.md, GEMINI.md, etc.)
- Skill definitions
- MCP server binaries and configs
- Instructions and prompts
- Resource limits (context, timeout)

## Communication

| Path | Protocol | Frequency | Notes |
|------|----------|-----------|-------|
| Queen <-> Overseer | HTTP | Low | Job state, registration, heartbeat |
| Drones <-> Overseer | MCP / HTTP | Medium | Decisions, memories, tasks, artifacts |
| Drones <-> Creep | gRPC | High | LSP queries, AST lookups, cache reads |
| Queen <-> Creep | HTTP | Low | Health check only |
| Queen -> Drones | Process mgmt | On spawn/kill | Spawn child process, monitor, terminate |

**Design rationale:**
- gRPC for Creep: high-frequency structured queries, protobuf schema enforcement, streaming support (LSP diagnostics)
- HTTP for Overseer: already exists, low frequency management calls
- MCP for Drones <-> Overseer: standard agent protocol, already supported

## Multi-Hatchery Model

**Registration:** When a Hatchery starts, Queen registers with Overseer:
- Hatchery ID (unique, persistent across restarts)
- Available drone types (what's been built and is deployable)
- Capabilities/constraints (architecture, resources, network access)
- Heartbeat interval

**Job routing:** Overseer knows which Hatcheries exist and what they can run:
- Match required drone type to capable Hatcheries
- Consider constraints (GPU, filesystem access, network)
- Assign to a Hatchery — Queen picks it up and spawns the drone

**Concurrency:** Each Hatchery manages its own limits — how many drones can run simultaneously is a Hatchery-level config based on available resources. Queen enforces locally.

**Failure:** If a Hatchery misses heartbeats, Overseer marks its active jobs as orphaned. Another capable Hatchery can pick them up, or they wait for recovery.

Intentionally simple — no complex scheduling algorithms, no preemption. Registration, capability matching, and local concurrency limits.

## Repo Structure

```
src/
  overseer/            # Central service (existing)
  queen/               # Hatchery binary — Queen + Evolution Chamber
  creep/               # Sidecar binary — LSP, AST, caching
  drones/              # Drone definitions and build rules
    claude/
      base/
      code-reviewer/
      feature-builder/
    gemini/
      base/
      quick-fix/
    pi/
      base/
      triage/
    kimi/
      base/
    trait.rs
    BUCK
  proto/               # gRPC protobuf definitions (Creep API)
```

Queen and Creep are separate crates in the Cargo workspace. Drone packages are Buck2 targets. The `proto/` crate is shared between Creep (server) and the drone runtime (client).

## Build System Integration

Drone artifacts are Buck2 build targets. A custom `drone_package()` rule assembles the complete hermetic package:
- Bundles agent config, skills, MCP server binaries, instructions
- Deterministic output, cached and shareable via BuildBuddy
- Evolution Chamber improvements become code changes that produce new drone artifact versions through the normal plan/dev/test/PR cycle
