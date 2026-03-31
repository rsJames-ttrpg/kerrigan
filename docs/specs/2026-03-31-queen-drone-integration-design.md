# Queen-Drone Integration Design

## Overview

Wire Queen's Supervisor to invoke real drone binaries using the drone-sdk protocol, replacing the `sleep infinity` placeholder. Queen imports protocol types directly from `drone-sdk` to keep interfaces in sync.

## Changes

### Shared Protocol (drone-sdk dependency)

Queen adds `drone-sdk` as a dependency. The Supervisor uses `drone_sdk::protocol::{QueenMessage, DroneMessage, JobSpec, DroneOutput}` for serializing/deserializing messages on the drone's stdin/stdout. This is the single source of truth for the protocol — no duplicate types.

### Config

Add `drone_dir` to `QueenConfig`:

```toml
[queen]
drone_dir = "./drones"    # Directory containing drone binaries
```

Queen resolves a drone binary as `{drone_dir}/{drone_type}` (e.g. `./drones/claude-drone`). Validate the binary exists before spawning.

### SpawnRequest

Add fields needed to construct the drone's `JobSpec`:

```rust
pub struct SpawnRequest {
    pub job_run_id: String,
    pub drone_type: String,
    pub job_config: Value,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
}
```

### Supervisor: spawn_drone

Replace the `sleep infinity` placeholder:

1. Resolve binary: `{drone_dir}/{drone_type}` — error if not found
2. Spawn the binary with stdin/stdout piped, stderr inherited (goes to Queen's log)
3. Write a `QueenMessage::Job(JobSpec { ... })` JSON line to the drone's stdin
4. Spawn a background tokio task that reads the drone's stdout line-by-line, parsing `DroneMessage` variants
5. The reader task sends parsed messages to the Supervisor via an mpsc channel
6. Supervisor stores the channel receiver in `DroneHandle`

### Supervisor: Protocol Message Handling

Add a new `select!` branch in the main loop that drains protocol messages from active drones:

- **`DroneMessage::Progress`** — log via tracing
- **`DroneMessage::AuthRequest`** — forward to Notifier as `QueenEvent::AuthRequested { job_run_id, url, message }`. For v1 this just logs the URL. The drone will eventually time out waiting for a response since Queen doesn't send `AuthResponse` back yet.
- **`DroneMessage::Result(output)`** — store the conversation history as an artifact in Overseer, update job run as completed with the output, remove drone from active map
- **`DroneMessage::Error(e)`** — update job run as failed, remove from active map

### Supervisor: DroneHandle

Updated to track the protocol channel and the drone's stdin for future auth responses:

```rust
struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
    protocol_rx: mpsc::Receiver<DroneMessage>,
}
```

When a `Progress` or `Result` message arrives, `last_activity` is updated — this replaces the Overseer task polling for stall detection. The drone itself reports activity.

### Poller: Fix drone_type Source

Currently reads `drone_type` from `run.triggered_by` which is semantically wrong. Fix: read from the job run's config or the job definition's config. Since Overseer's job definition has a `config: Value` field, the Poller fetches the definition and reads `config.drone_type`.

Add `get_job_definition(id)` to `OverseerClient`. The Poller calls it for each new run and extracts `drone_type`, `repo_url`, `branch`, and `task` from the definition's config.

### OverseerClient Additions

- `get_job_definition(id: &str) -> Result<JobDefinitionResponse>` — `GET /api/jobs/definitions/{id}` (note: this endpoint doesn't exist yet on Overseer — needs adding)
- `store_drone_output(job_run_id: &str, output: &DroneOutput) -> Result<()>` — stores conversation as an artifact via `POST /api/artifacts`

### Overseer: Get Job Definition by ID

Add a `GET /api/jobs/definitions/{id}` endpoint to Overseer's job API. The route and handler already have `get_job_definition` in the service layer — just needs the HTTP route wired up.

### Notifier: New Event

Add `AuthRequested` variant to `QueenEvent`:

```rust
QueenEvent::AuthRequested {
    job_run_id: String,
    url: String,
    message: String,
}
```

`LogNotifier` logs it at warn level with the URL so the operator can see it.

## What This Does NOT Cover

- Auth response flow (Queen sending `AuthResponse` back to the drone) — v2
- Drone registry/discovery (drone types are just binary names in a directory) — v2
- Multiple concurrent protocol channels (each drone gets its own reader task, aggregated into Supervisor's select loop via a merged stream)

## Testing

- Unit test: `JobSpec` serialization round-trip using `drone_sdk::protocol` types
- Integration test: spawn a mock drone binary (a simple Rust binary that reads JobSpec, writes Result), verify Supervisor processes it correctly
- Existing Queen tests continue to pass
