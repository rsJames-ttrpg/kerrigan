# Native Drone: Implementation Plan Overview

> **For agentic workers:** Each sub-plan is self-contained and can be dispatched independently. Plans must be executed in dependency order. Use superpowers:subagent-driven-development or superpowers:executing-plans for each plan.

**Goal:** Replace the Claude CLI wrapper drone with a native Rust agent runtime that talks directly to LLM APIs, giving full control over the agent loop, tool execution, context management, and observability.

**Architecture:** Two new crates — `src/runtime/` (generic agent engine library) and `src/drones/native/` (kerrigan-specific drone binary implementing `DroneRunner`). The runtime is reusable beyond drones. The drone adds pipeline stages, git workflow, orchestration, and Queen integration.

**Reference Implementation:** [Claw Code](https://github.com/rsJames-ttrpg/claudecode) (`rust/crates/`) — consult for patterns, edge cases, and tested approaches.

**Specs:** `docs/specs/native-drone/00-overview.md` through `06-drone-queen-integration.md`, plus `docs/specs/2026-04-04-creep-lsp-integration-design.md`.

---

## Dependency Graph

```
Plan 00: Crate Scaffolding
  └─▶ Plan 01: API Client
        └─▶ Plan 02: Tool System
              └─▶ Plan 03: Conversation Loop
                    ├─▶ Plan 04: Pipeline & Health Checks
                    │     ├─▶ Plan 05: Config & Prompts
                    │     └─▶ Plan 06: Queen Integration
                    └─▶ Plan 07: Orchestrator
Plan 08: Creep LSP (independent)
```

## Plans

| Plan | Name | Depends On | Estimated Size | Dispatchable? |
|------|------|-----------|---------------|---------------|
| [00](00-crate-scaffolding.md) | Crate Scaffolding | — | Small | Yes |
| [01](01-api-client.md) | Runtime API Client | 00 | Medium | Yes |
| [02](02-tool-system.md) | Runtime Tool System | 00 | Large | Yes (split into 02a/02b/02c) |
| [03](03-conversation-loop.md) | Runtime Conversation Loop | 01, 02 | Medium | Yes |
| [04](04-pipeline.md) | Drone Pipeline & Health Checks | 03 | Medium | Yes |
| [05](05-config-prompts.md) | Drone Config & Prompts | 04 | Medium | Yes |
| [06](06-queen-integration.md) | Drone Queen Integration | 04, 05 | Medium | Yes |
| [07](07-orchestrator.md) | Drone Orchestrator | 03, 04 | Medium | Yes |
| [08](08-creep-lsp.md) | Creep LSP Integration | — | Medium | Yes |

## Execution Strategy

1. **Phase 1 — Runtime foundation:** Plans 00 → 01 → 02 → 03 (sequential, each builds on prior)
2. **Phase 2 — Drone layer:** Plans 04, 05, 06, 07 (04 first, then 05/06/07 can partially parallel)
3. **Phase 3 — Creep LSP:** Plan 08 (fully independent, can run alongside any phase)
4. **Phase 4 — Integration test:** End-to-end with Queen spawning native drone against Ollama

## Common Patterns for All Plans

**New crate setup** (done once in Plan 00, referenced by all):
1. Create `src/{name}/Cargo.toml` with `edition = "2024"`
2. Create `src/{name}/BUCK` following existing patterns
3. Add to workspace `members` in root `Cargo.toml`
4. `./tools/buckify.sh` to regenerate `third-party/BUCK`

**Testing:**
- Unit tests inline (`#[cfg(test)] mod tests`)
- `cargo test` for fast feedback
- `buck2 test //src/{name}:{name}-test` for CI-equivalent

**Commits:** After each task completes, commit with descriptive message.
