# Kerrigan Roadmap

## Goal

Submit a problem spec, get a PR back. Kerrigan develops itself.

```
Problem → Spec → Plan → Implementation → Review → PR
   ↑        ↑      ↑                              │
   │     human   human                          human
   │    approval approval                      merge
   └───────────────────────────────────────────────┘
```

Human input: problem definition, spec approval, plan approval, final PR review/merge.
Everything else runs autonomously as drone work.

## What Exists

| Component | Status | PR |
|---|---|---|
| Overseer (central service) | Merged | #1, #2, #3 |
| Queen (process manager) | Merged | #4, #6 |
| drone-sdk (protocol + trait) | Merged | #5 |
| claude-drone (skeleton) | Merged | #5 |
| Queen-Drone integration | Merged | #6 |
| Creep v1 (file index gRPC) | Open | #7 |
| Dev container (all-in-one) | Done | — |
| Claude CLI bundled in drone | Done | — |
| Auth flow (stderr streaming, relay) | Done (partial) | — |
| Job status fix (pending not running) | Done | — |

## What's Missing

### Phase 1: Claude Drone Config Design
*The drone skeleton exists but has no real configuration. This is product design, not just code.*

**1. Drone config and behavior design** `[done — spec + implementation]`
- Permission model — what can the drone do without asking?
- Tool restrictions — what should it NOT have access to?
- CLAUDE.md instructions — what makes a drone effective at dev tasks?
- Hooks — progress reporting to Queen, PR creation, auth handling
- MCP servers — Overseer (memory/decisions), Creep (file lookups when ready)
- Skills/plugins — which superpowers skills? Custom skills?
- Per-stage configs — spec writing vs implementation vs review need different prompts and tool access
- Depends on: nothing, can start now

**2. Vendor and bundle drone config** `[done — base drone working]`
- Claude CLI hermetically fetched by Buck2, embedded via include_bytes! (~228MB)
- Drone extracts CLI to temp home, runs it from there
- Dev container runs Overseer + Queen + drone end-to-end
- Auth: credential mount works; headless OAuth needs custom implementation
- Stage-specific subtypes (spec-writer, implementer, reviewer) still TODO
- Depends on: #1

### Phase 2: End-to-End Smoke Test
*Make the full loop actually run: Overseer → Queen → Drone → Claude Code → Output*

**3. Integration testing** `[done — first successful drone task]`
- Dev container runs full loop: Overseer → Queen → Drone → Claude CLI → Result
- Fixed: job run status (pending not running), non-root container, CLI bundling
- Auth: credential mount bypass works; headless OAuth is the remaining gap
- Depends on: #2

**4. Job submission interface** `[next — overseer client library + CLI/UI]`
- Overseer client library (Rust crate) — reusable by Queen, CLI, web UI
- CLI tool: `kerrigan submit`, `kerrigan status`, `kerrigan auth`
- Simple htmx web UI for job submission and status
- Replaces the current curl-based job submission
- Depends on: #3

### Phase 3: Autonomous Development Loop
*Submit a problem, get a PR back.*

**5. Drone PR workflow** `[implementation]`
- claude-drone configures Claude Code to work on a branch, commit, push, create PR
- Drone hooks handle branch creation and PR submission
- Git refs (branch, PR URL) reported back in DroneOutput and stored in Overseer
- Depends on: #3, #4

**6. Job templates for dev stages** `[spec needed]`
- Pre-defined job definitions for each stage of the dev loop:
  - `spec-from-problem` — takes a problem description, produces a design spec
  - `plan-from-spec` — takes a spec, produces an implementation plan
  - `implement-from-plan` — takes a plan, produces code + tests + PR
  - `review-pr` — reviews a PR, produces feedback or approval
- Each stage uses a different drone subtype with stage-appropriate config
- Depends on: #2, #5

**7. Job chaining** `[spec needed]`
- When a drone completes a stage, automatically trigger the next stage
- Overseer's parent_id on job runs supports the hierarchy
- Queen needs logic: on job completion, check if there's a next stage to trigger
- Human gates: spec approval and plan approval pause the chain and notify the operator
- Depends on: #6

### Phase 4: Quality and Feedback

**8. Creep integration with drones** `[implementation]`
- Creep gRPC client in drone-sdk or as an MCP server bundled in drone config
- Drones get fast file lookups, symbol search
- Depends on: Creep v1 merged (#7)

**9. Evolution Chamber v1** `[spec needed]`
- Heuristic analysis of completed drone sessions
- Metrics: context usage, tool call patterns, failures, cost
- Targeted LLM analysis on flagged segments
- Outputs problem specs → GitHub issues → feeds back into the dev loop
- Depends on: #5 (needs real drone output to analyze)

**10. Auth flow** `[implementation]`
- Queen forwards auth_response back to drones (currently stdin is closed after JobSpec)
- Retain drone stdin handle in Queen's supervisor
- Or: pre-auth on host and avoid the problem entirely
- Depends on: #3

### Phase 5: Scale and Polish

**11. Creep v2** — tree-sitter AST parsing, symbol index
**12. Creep v3** — LSP management (warm language servers across sessions)
**13. Additional drone types** — Gemini CLI, local Pi inference for triage
**14. Dashboard** — operator visibility (jobs, drones, hatchery status)
**15. Deployment** — k8s manifests for Overseer, container/systemd for Hatchery
**16. Evolution Chamber v2** — automated improvement implementation (not just problem specs)

## Critical Path to Dogfooding

```
#1 Drone config design ✅
 └→ #2 Vendor drone configs ✅ (base drone, subtypes TODO)
     └→ #3 Integration smoke test ✅ (first drone task completed)
         ├→ #4 Job submission ← NEXT (client lib + CLI/UI)
         │   └→ #5 PR workflow
         │       └→ #6 Job templates
         │           └→ #7 Job chaining ← DOGFOODING
         └→ #10 Auth flow (partial — cred mount works, headless OAuth TODO)
```

Items 4-7 are the remaining critical path. #4 is next: Overseer client library, then CLI and/or htmx UI on top.
