# Nydus Client Library + Kerrigan CLI

**Date:** 2026-04-01
**Roadmap item:** #4 — Job submission interface

## Context

Submitting a job currently requires 4 curl commands (create definition, start run, assign to hatchery). Queen has an internal `OverseerClient` that duplicates Overseer's types and is not reusable. The goal is a shared client library and an operator-facing CLI that makes the dev loop accessible with minimal friction.

The dev loop this serves:

```
Problem → Spec → Plan → Implementation → Review → PR
   ↑        ↑      ↑                              │
   │     human   human                          human
   │    approval approval                      merge
   └───────────────────────────────────────────────┘
```

The operator describes a problem. Everything else runs async. The operator gets notified at gates (spec approval, plan approval, PR review) and approves or rejects.

## Approach

Thin HTTP client library (`nydus`) + operator console binary (`kerrigan`). Library is a 1:1 typed wrapper over Overseer's HTTP API. CLI orchestrates the multi-step submit workflow and provides visibility into running jobs. Queen migrates off its internal client to use `nydus`.

## `nydus` — Client Library

**Crate:** `src/nydus/`, library, workspace member.

### Core struct

```rust
pub struct NydusClient {
    base_url: String,
    client: reqwest::Client,
}
```

Stateless — no internal locks or cached IDs. Callers manage their own state (e.g., Queen tracks its hatchery ID separately).

### API methods

| Group | Methods |
|-------|---------|
| Jobs | `create_definition`, `get_definition`, `list_definitions`, `start_run`, `list_runs`, `update_run` |
| Tasks | `create_task`, `list_tasks`, `update_task` |
| Hatcheries | `register`, `heartbeat`, `get`, `list`, `deregister`, `list_jobs`, `assign_job` |
| Artifacts | `store`, `get`, `list` |
| Auth | `submit_auth_code`, `poll_auth_code` |

Each method maps 1:1 to an Overseer HTTP endpoint. No high-level orchestration in the library.

### Response types

Owned by `nydus`, one per resource:

- `JobDefinition` — id, name, description, config
- `JobRun` — id, definition_id, parent_id, status, triggered_by, result, error
- `Task` — id, run_id, subject, status, assigned_to, output, updated_at
- `Hatchery` — id, name, status, capabilities, max_concurrency, active_drones
- `Artifact` — id, name, content_type, run_id

All derive `Debug, Clone, Serialize, Deserialize`.

### Error type

`nydus::Error` with variants:
- `Request` — wraps `reqwest::Error` (network, timeout, etc.)
- `Api` — Overseer returned an error response (status code + body)

Implements `std::error::Error` and `Display`.

### `start_run` config overrides

`start_run` accepts an optional `config_overrides: Option<Value>`. The Overseer-side `StartJobRunRequest` gains this field, and the jobs service merges overrides into the definition's config before persisting the run. Shallow merge: top-level keys from overrides replace the same keys in the definition config. No deep/recursive merge of nested objects.

## `kerrigan` — Operator Console

**Crate:** `src/kerrigan/`, binary, workspace member. Depends on `nydus` and `clap`.

### Config resolution

Overseer URL: `--url` flag > `KERRIGAN_URL` env var > `http://localhost:3100`.

### Commands

```
kerrigan <problem>                          # submit a problem into the dev loop
kerrigan status [<run-id>]                  # what's running, what needs me
kerrigan approve <run-id> [--message "..."] # approve spec/plan at a gate
kerrigan reject <run-id> --message "..."    # reject with feedback
kerrigan auth <run-id> <code>               # OAuth code relay
kerrigan log <run-id>                       # view run output/decisions
```

### `kerrigan <problem>` workflow

1. Look up the entry-point job definition by name (e.g., `spec-from-problem`)
2. Start a run with `{ "problem": "<problem text>" }` as config
3. If `--hatchery <name>` given, resolve by name; otherwise list active hatcheries, pick first with capacity
4. Assign run to hatchery
5. Print run ID, exit

The dev loop continues async from there. Orchestration (advancing stages, gating on approvals, triggering next stages) is Overseer's responsibility — future work in roadmap items #6 and #7.

### `kerrigan status [<run-id>]`

- No args: list all active runs, highlight any waiting for approval
- With run ID: show run detail — current stage, status, tasks, linked artifacts

### `kerrigan approve/reject <run-id>`

Calls `update_run` with the appropriate status transition and optional message. The orchestration layer in Overseer picks up approvals and triggers next stages.

### `kerrigan log <run-id>`

Read-only view of decisions and task outputs for a run.

## Queen Migration

- Delete `src/queen/src/overseer_client.rs`
- Add `nydus` as a dependency
- Replace all `OverseerClient` usage with `NydusClient`
- Move `hatchery_id` tracking from the client into Queen's own state (since `NydusClient` is stateless)

## Overseer Changes

Minimal:
- `StartJobRunRequest` gains `config_overrides: Option<Value>`
- Jobs service merges overrides into definition config when creating a run (`serde_json` object merge, override keys win)
- No new endpoints, no schema changes

## Out of Scope

- Pipeline/chain definitions (roadmap #7)
- Stage-aware job definitions (roadmap #6)
- Evolution Chamber feedback loop (roadmap #9)
- Notification implementations (trait exists, implementations later)
- htmx web UI (future, will consume `nydus`)
- Definition inheritance (can be added later as a CLI or Overseer feature)
