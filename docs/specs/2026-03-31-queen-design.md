# Queen Design

## Overview

Queen is the process manager within the Hatchery binary. It manages drone lifecycles, communicates with Overseer for job state, and provides notifications to the operator. No LLM calls — pure systems engineering.

## Architecture

```
                    ┌─────────────────────────────────────┐
                    │          Queen (Hatchery binary)    │
                    │                                     │
                    │  ┌───────────┐   ┌──────────────┐   │
                    │  │ Registrar │   │  Heartbeat   │   │
                    │  │  (boot)   │   │  (periodic)  │   │
                    │  └───────────┘   └──────────────┘   │
                    │                                     │
                    │  ┌───────────┐   ┌──────────────┐   │
                    │  │  Poller   │   │    Creep     │   │
                    │  │  (jobs)   │   │   Manager    │   │
                    │  └─────┬─────┘   └──────────────┘   │
                    │        │                            │
                    │  ┌─────▼─────────────────────────┐  │
                    │  │       Supervisor              │  │
                    │  │  ┌───────┐ ┌───────┐ ┌─────┐  │  │
                    │  │  │Drone 1│ │Drone 2│ │  …  │  │  │
                    │  │  └───────┘ └───────┘ └─────┘  │  │
                    │  └───────────────────────────────┘  │
                    │                                     │
                    │  ┌───────────────────────────────┐  │
                    │  │       Notifier (trait)        │  │
                    │  └───────────────────────────────┘  │
                    └─────────────────────────────────────┘
```

Queen uses an actor model built on tokio tasks and mpsc channels. Each actor owns its own state and async loop.

**Actors:**

- **Registrar** — runs once at boot. Discovers available drone types from local storage/registry, registers with Overseer via HTTP (reporting capabilities), retries on failure, then exits. Sets the `hatchery_id` on the shared OverseerClient.
- **Heartbeat** — periodic loop, sends status and active drone count to Overseer. Queries Supervisor for current counts.
- **Poller** — periodic loop, fetches unassigned jobs from Overseer that match this Hatchery's capabilities. Sends spawn requests to Supervisor.
- **Supervisor** — core actor. Owns all drone processes. Receives spawn requests from Poller, monitors health (process alive + activity check), enforces timeouts, collects output on completion, reports results to Overseer.
- **CreepManager** — spawns Creep as a child process, health-checks it, restarts on failure. Runs in background, does not block boot.
- **Notifier** — trait-based notification sink. All actors send events through the Notifier. First implementation: logging to stdout via tracing.

## Boot Sequence

1. Load config (`hatchery.toml` + CLI args + env vars)
2. Init tracing
3. Create OverseerClient
4. Register with Overseer (Registrar actor, retries on failure)
5. Start Creep in background (CreepManager actor, non-blocking)
6. Start Heartbeat actor
7. Start Poller actor
8. Start Supervisor actor
9. Await shutdown signal (Ctrl+C)

Registration happens before polling so Queen has a `hatchery_id`. Creep starts in parallel — drones degrade gracefully without it.

## Configuration

`hatchery.toml` with clap for CLI/env overrides. Every field in `[queen]` overridable via env: `QUEEN_NAME`, `QUEEN_OVERSEER_URL`, `QUEEN_MAX_CONCURRENCY`, etc. Available drone types are discovered at runtime from local storage/registry, not configured.

```toml
[queen]
name = "rpi-1"                          # Hatchery name (unique across fleet)
overseer_url = "http://localhost:3100"   # Overseer API endpoint
heartbeat_interval = 30                  # seconds
poll_interval = 10                       # seconds
max_concurrency = 4                      # max simultaneous drones
drone_timeout = "2h"                     # default timeout if drone doesn't specify
stall_threshold = 300                    # seconds without activity before stalled

[creep]
enabled = true
binary = "./creep"                       # path to creep binary
health_port = 9090                       # creep health check port
restart_delay = 5                        # seconds before restart attempt

[notifications]
backend = "log"                          # "log" for v1
```

## Supervisor and Drone Lifecycle

The Supervisor manages drone processes through a state machine:

**Drone states:** `Spawning -> Running -> Completing -> Done | Failed | TimedOut | Stalled`

**Spawn flow:**
1. Poller sends `SpawnRequest { job_run_id, drone_type, job_config }` to Supervisor
2. Supervisor checks concurrency limit — if full, queues the request
3. Supervisor looks up the drone artifact by type, calls `Drone::run(job_spec)`
4. Drone process starts, Supervisor tracks it in a `HashMap<String, DroneHandle>`

**Health monitoring:**
- Supervisor runs a periodic sweep (every 30s) over active drones
- For each drone: is the process alive? Has it updated tasks/decisions in Overseer within `stall_threshold`?
- Stalled drone: notify, then kill after a grace period
- Timed out drone: kill immediately, update job run as failed

**Completion flow:**
1. Drone process exits (success or failure)
2. Supervisor collects exit status
3. Updates job run in Overseer (completed/failed)
4. Sends notification via Notifier
5. Removes from active map, checks queue for pending spawns

**DroneHandle:**
```rust
struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: tokio::process::Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
}
```

The Supervisor updates `last_activity` by polling Overseer for recent task updates on the drone's job run.

## Actor Communication

Channel-based messaging. No actor framework — just tokio mpsc.

**Message types:**
- Poller -> Supervisor: `SpawnRequest { job_run_id, drone_type, job_config }`
- Heartbeat -> Supervisor: `StatusQuery` / Supervisor -> Heartbeat: `StatusResponse { active_drones, queued_jobs }`
- Any actor -> Notifier: `QueenEvent`

**OverseerClient:** Shared HTTP client passed to actors that need it. Wraps `reqwest::Client` with a base URL and typed methods.

```rust
struct OverseerClient {
    base_url: String,
    client: reqwest::Client,
    hatchery_id: String,
}
```

Methods: `register()`, `heartbeat()`, `poll_jobs()`, `update_job_run()`, `get_recent_activity()`.

## Notifier

```rust
#[async_trait]
trait Notifier: Send + Sync {
    async fn notify(&self, event: QueenEvent);
}

enum QueenEvent {
    HatcheryRegistered { name: String, id: String },
    DroneSpawned { job_run_id: String, drone_type: String },
    DroneCompleted { job_run_id: String, exit_code: i32 },
    DroneFailed { job_run_id: String, error: String },
    DroneStalled { job_run_id: String, last_activity_secs: u64 },
    DroneTimedOut { job_run_id: String },
    CreepStarted,
    CreepDied { restart_in_secs: u64 },
    ShuttingDown,
}
```

First implementation: `LogNotifier` — structured log lines via `tracing::info!` / `tracing::warn!`.

## Shutdown

Ctrl+C triggers graceful shutdown:
1. Stop Poller (no new jobs)
2. Supervisor sends kill to all active drones
3. Wait for drone processes to exit (with hard timeout)
4. Stop Heartbeat
5. Stop CreepManager (kills Creep process)
6. Deregister from Overseer
7. Exit

## Drone Interface

Queen calls drones through a trait. The drone handles its own execution — Queen doesn't know the internals.

```rust
#[async_trait]
trait Drone: Send + Sync {
    async fn run(&self, job_spec: JobSpecification) -> tokio::process::Child;
}
```

Jobs arrive from Overseer with a `drone_type` field already specified. Queen looks up the corresponding drone artifact and launches it. The drone artifact and its `Drone` trait implementation are defined in the `src/drones/` directory — out of scope for this spec.

## Repo Structure

```
src/
  queen/
    Cargo.toml
    BUCK
    src/
      main.rs              # Entry point: load config, boot actors, await shutdown
      config.rs            # hatchery.toml + clap CLI args + env overrides
      overseer_client.rs   # HTTP client for Overseer API
      actors/
        mod.rs
        registrar.rs       # One-shot: register with Overseer
        heartbeat.rs       # Periodic: send heartbeat
        poller.rs          # Periodic: fetch jobs
        supervisor.rs      # Core: manage drone processes
        creep_manager.rs   # Manage Creep sidecar
      notifier/
        mod.rs             # Notifier trait + QueenEvent
        log.rs             # LogNotifier impl
      messages.rs          # SpawnRequest, StatusQuery, etc.
```

Queen is a new crate in the Cargo workspace, separate binary from Overseer, with its own `BUCK` target.

Key dependencies: `tokio`, `reqwest`, `clap`, `serde`, `toml`, `tracing`, `async-trait`, `chrono`.
