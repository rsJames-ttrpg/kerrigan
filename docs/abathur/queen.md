---
title: Queen Hatchery Manager
slug: queen
description: Actor-based process manager — job polling, drone lifecycle, notifications, evolution
lastmod: 2026-04-06
tags: [queen, hatchery, actors, drones, notifications]
sources:
  - path: src/queen/src/main.rs
    hash: fe0adecce258ca4dd3090d6dec4c7ef85fc7bd81c8b9efb8c6561d49eea035d5
  - path: src/queen/src/config.rs
    hash: 0817c30a5d093a59f544dea8a078569aea705fd1280496cd93268a092590e6f4
  - path: src/queen/src/messages.rs
    hash: 134afb9ab6df2f36fc00be9adb76f9ee5e03657045062458085cbdafaf7c0c4f
  - path: src/queen/src/actors/supervisor.rs
    hash: a249166e0286de5e9e95bba642d71005988a39349e644f2114af7d9cf5344215
  - path: src/queen/src/actors/poller.rs
    hash: 7317cc684b88dd4cc9c23a4cc1858b0fc3735c415880cdb10efd52d3b610d0e8
  - path: src/queen/src/actors/heartbeat.rs
    hash: 256d7313b8218d6a15a12099f7a3b354d03d105f2371025171ba57fdcc62c6c9
  - path: src/queen/src/actors/registrar.rs
    hash: b130e383ab98472670d2b647c2833daa410813096dee757d2d93ac3e13e45f97
  - path: src/queen/src/actors/creep_manager.rs
    hash: 2523ed8a5b7c8baacfa625ab5813b6c8263cb993523d1d98f4b9afb3a3d614ba
  - path: src/queen/src/actors/evolution.rs
    hash: 4bd5634d60ccd889e850a706643e359f3c99975b56a79370b62c3c61aa321753
  - path: src/queen/src/notifier/mod.rs
    hash: 1bff129fd9519aa6e66a2d0d3bb44425a8445ff1fa85d9bc4009eae47ff1e112
  - path: src/queen/src/notifier/webhook.rs
    hash: fdf0caac58940d35200e36a64dab6519c0ae517e0a9ae8e109ea352ab9bf81a8
sections: [architecture, actors, job-claiming-flow, notification-system, evolution-actor, configuration, shutdown]
---

# Queen Hatchery Manager

## Architecture

Queen is a tokio-based actor system. Each actor runs as a detached tokio task communicating via mpsc channels. Startup is sequential for registration (blocking), then all actors spawn concurrently.

```
main()
  → registrar::run()  [BLOCKING — retries until Overseer responds]
  → spawn heartbeat, poller, supervisor, creep_manager, evolution
  → await Ctrl+C → cancel all → deregister
```

Shared state: `hatchery_id` in `Arc<RwLock<Option<String>>>`, `NydusClient` for Overseer API, `CancellationToken` for shutdown.

## Actors

### Registrar (one-shot)
Blocks main until Overseer registration succeeds. Retries every 5s on failure. Stores assigned `hatchery_id` in shared `Arc<RwLock>`. Fires `QueenEvent::HatcheryRegistered`.

### Poller (periodic, default 10s)
1. `client.list_pending_runs()` — fetch unassigned pending jobs
2. `client.assign_job(hatchery_id, run_id)` — atomic claim (race-safe)
3. Fetch definition, merge `config_overrides` on top of definition config
4. **Credential injection:** `client.match_credentials(repo_url)` → inject `github_pat` into `config.secrets`
5. **URL injection:** inject `overseer_url` and `default_repo_url` if missing
6. Validate required fields: `drone_type`, `repo_url`, `task`
7. Send `SpawnRequest` to Supervisor via `spawn_tx` channel

On validation failure, immediately fails the run in Overseer.

### Supervisor (long-running, core engine)

Maintains `active: HashMap<String, DroneHandle>` and `queue: VecDeque<SpawnRequest>`.

**Select loop:**
- Spawn request → if under `max_concurrency`, spawn drone; else queue
- Status query → reply with active/queued counts (for heartbeat)
- Health tick (5s) → drain protocol messages, check drone health, dequeue

**DroneHandle:**
```rust
struct DroneHandle {
    job_run_id: String,
    process: Child,           // OS process
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,   // updated on protocol msg or stderr
    stall_notified: bool,
    protocol_rx: mpsc::Receiver<DroneMessage>,
    stderr_rx: mpsc::Receiver<()>,
    stdin_tx: Option<mpsc::Sender<QueenMessage>>,
}
```

**spawn_drone():** validates drone_type (no path traversal), spawns `Command::new(drone_dir/drone_type)` with piped I/O, writes `QueenMessage::Job(spec)` to stdin, starts protocol reader (stdout JSONL) and stderr monitor tasks, marks run as "running".

**Protocol message handling:**
- `Progress` → log, update `last_activity`
- `AuthRequest` → notify, spawn async poll for `client.poll_auth_code()`, relay code via stdin
- `Result` → gzip conversation + session artifacts, store via Nydus, update run status (fail if `pr_required` but no `pr_url`), remove from active
- `Error` → update run to "failed", remove from active

**Health checks:**
- Process exit without Result → mark "failed" with "unexpected exit"
- Timeout exceeded → kill process, mark "failed" with "timed out"
- Stall detection → if `last_activity` exceeds `stall_threshold`, notify once

### Heartbeat (periodic, default 30s)
Sends `StatusQuery` to Supervisor via oneshot channel, reports active drone count to Overseer via `client.heartbeat()`.

### Creep Manager (sidecar supervisor)
If enabled, spawns Creep binary and restarts on failure after `restart_delay` seconds. Fires `CreepStarted` and `CreepDied` events.

## Job Claiming Flow

```
Overseer                     Poller                    Supervisor
────────────────────────────────────────────────────────────────
list_pending_runs() ────→ pending runs
                          claim(hatchery_id, run_id)
                          fetch definition
                          inject credentials
                          validate fields
                          SpawnRequest ──────────→ spawn_drone()
                                                   write JobSpec
                                                   update "running"
```

Claiming is atomic — if another Queen claims first, the claim call returns an error and the run is skipped.

## Notification System

```rust
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, event: QueenEvent);
}
```

**Events:** `HatcheryRegistered`, `DroneSpawned`, `DroneCompleted`, `DroneFailed`, `DroneStalled`, `DroneTimedOut`, `AuthRequested`, `CreepStarted`, `CreepDied`, `ShuttingDown`.

**LogNotifier** — logs via `tracing::{info,warn}`.

**WebhookNotifier** — POSTs JSON to URL with:
- Event filtering: `events: ["drone_failed", "drone_stalled"]`
- Bearer auth: `token: "secret"` or `token: "env:VAR_NAME"`
- Template rendering: `{{event_type}}`, `{{job_run_id}}`, `{{error}}`, `{{message}}`
- 10s request timeout, optional TLS skip verify

## Evolution Actor

Disabled by default. When enabled, monitors completed runs and triggers heuristic analysis.

**Triggers:**
- Count-based: N completed runs since last analysis (`run_interval`, default 10)
- Time-based: elapsed since last analysis (`time_interval`, default 24h)

**Flow:**
1. Resolve evolution job definition ID once at startup
2. Recover last analysis time from most recent "evolution-report" artifact
3. Poll every 60s for completed runs
4. On trigger: `EvolutionChamber::analyze()` → serialize report → store as artifact → submit `evolve-from-analysis` job

## Configuration

`hatchery.toml`:

| Section | Key | Default |
|---------|-----|---------|
| `queen.name` | Hatchery identifier | (required) |
| `queen.overseer_url` | Overseer endpoint | http://localhost:3100 |
| `queen.poll_interval` | Job poll interval (s) | 10 |
| `queen.heartbeat_interval` | Heartbeat interval (s) | 30 |
| `queen.max_concurrency` | Max concurrent drones | 4 |
| `queen.drone_timeout` | Per-drone timeout | "2h" |
| `queen.stall_threshold` | Stall detection (s) | 300 |
| `queen.drone_dir` | Drone binary directory | "./drones" |
| `queen.default_repo_url` | Fallback repo URL | None |
| `creep.enabled` | Enable Creep sidecar | true |
| `creep.binary` | Creep binary path | "./creep" |
| `creep.health_port` | Creep health check port | 9090 |
| `creep.restart_delay` | Restart delay (s) | 5 |
| `creep.lsp.<name>.command` | LSP server binary | (required) |
| `creep.lsp.<name>.args` | LSP server arguments | [] |
| `creep.lsp.<name>.extensions` | File extensions handled | (required) |
| `creep.lsp.<name>.language_id` | LSP language identifier | (required) |
| `notifications.backend` | "log" or "webhook" | "log" |
| `evolution.enabled` | Enable evolution actor | false |
| `evolution.run_interval` | Runs between analyses | 10 |

CLI flags (`--name`, `--overseer-url`, `--max-concurrency`, `--drone-dir`) and env vars (`QUEEN_NAME`, etc.) override config.

## Shutdown

1. `Ctrl+C` → `CancellationToken::cancel()`
2. All actors exit their select loops
3. Supervisor calls `shutdown_all()` — kills all active drone processes, updates runs to "cancelled"
4. Main deregisters from Overseer
5. Fires `QueenEvent::ShuttingDown`
