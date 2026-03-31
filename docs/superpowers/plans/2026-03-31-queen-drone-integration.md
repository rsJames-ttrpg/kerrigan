# Queen-Drone Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire Queen's Supervisor to invoke real drone binaries using the drone-sdk protocol, replacing the `sleep infinity` placeholder.

**Architecture:** Queen adds `drone-sdk` as a dependency for protocol types. Supervisor spawns drone binaries from a configured directory, communicates via JSON-line protocol on stdin/stdout. A background reader task per drone feeds protocol messages back to the Supervisor. Overseer gets a new `GET /api/jobs/definitions/{id}` endpoint so Poller can fetch job details.

**Tech Stack:** Rust (edition 2024), tokio, drone-sdk, serde_json, reqwest

---

## File Structure

**Modified files:**
- `src/queen/Cargo.toml` — add drone-sdk dependency
- `src/queen/BUCK` — add drone-sdk dep
- `src/queen/src/config.rs` — add `drone_dir` field
- `src/queen/src/messages.rs` — add fields to SpawnRequest, add DroneProtocolMessage
- `src/queen/src/notifier/mod.rs` — add AuthRequested event
- `src/queen/src/notifier/log.rs` — handle AuthRequested
- `src/queen/src/overseer_client.rs` — add get_job_definition, store_artifact methods
- `src/queen/src/actors/poller.rs` — fix drone_type source, add repo_url/branch/task
- `src/queen/src/actors/supervisor.rs` — replace sleep with real drone invocation + protocol
- `src/queen/src/main.rs` — pass drone_dir to supervisor
- `src/overseer/src/api/jobs.rs` — add GET /definitions/{id} route

---

### Task 1: Overseer — add GET job definition by ID endpoint

**Files:**
- Modify: `src/overseer/src/api/jobs.rs`

- [ ] **Step 1: Add the route**

In `src/overseer/src/api/jobs.rs`, add to the `router()` function:

```rust
.route("/definitions/{id}", get(get_job_definition))
```

- [ ] **Step 2: Add the handler**

Add after the `list_job_definitions` handler:

```rust
async fn get_job_definition(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .get_job_definition(&id)
        .await?
        .ok_or_else(|| crate::error::OverseerError::NotFound(format!("job_definition {id}")))?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
```

- [ ] **Step 3: Verify build and tests**

Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/api/jobs.rs
git commit -m "feat(overseer): add GET /api/jobs/definitions/{id} endpoint"
```

---

### Task 2: Queen — add drone-sdk dependency and drone_dir config

**Files:**
- Modify: `src/queen/Cargo.toml`
- Modify: `src/queen/BUCK`
- Modify: `src/queen/src/config.rs`

- [ ] **Step 1: Add drone-sdk to Queen's dependencies**

In `src/queen/Cargo.toml`, add:
```toml
drone-sdk = { path = "../drone-sdk" }
```

In `src/queen/BUCK`, add to both `QUEEN_DEPS`:
```python
"//src/drone-sdk:drone-sdk",
```

- [ ] **Step 2: Add drone_dir to QueenConfig**

In `src/queen/src/config.rs`, add a default function:
```rust
fn default_drone_dir() -> String {
    "./drones".to_string()
}
```

Add the field to `QueenConfig`:
```rust
#[serde(default = "default_drone_dir")]
pub drone_dir: String,
```

- [ ] **Step 3: Add drone_dir to CLI**

Add to the `Cli` struct:
```rust
/// Directory containing drone binaries
#[arg(long, env = "QUEEN_DRONE_DIR")]
pub drone_dir: Option<String>,
```

Update `apply_overrides`:
```rust
if let Some(ref dir) = cli.drone_dir {
    self.queen.drone_dir = dir.clone();
}
```

- [ ] **Step 4: Update existing tests**

In the `test_parse_minimal_config` test, add:
```rust
assert_eq!(config.queen.drone_dir, "./drones");
```

In the `test_cli_overrides` test, add `drone_dir: None` to the `Cli` struct construction.

- [ ] **Step 5: Run buckify and verify**

Run: `./tools/buckify.sh`
Run: `cd src/queen && cargo test`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/queen/Cargo.toml src/queen/BUCK src/queen/src/config.rs Cargo.lock
git commit -m "feat(queen): add drone-sdk dependency and drone_dir config"
```

---

### Task 3: Update messages and notifier for drone protocol

**Files:**
- Modify: `src/queen/src/messages.rs`
- Modify: `src/queen/src/notifier/mod.rs`
- Modify: `src/queen/src/notifier/log.rs`

- [ ] **Step 1: Update SpawnRequest with job details**

Replace `src/queen/src/messages.rs`:

```rust
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub job_run_id: String,
    pub drone_type: String,
    pub job_config: Value,
    pub repo_url: String,
    pub branch: Option<String>,
    pub task: String,
}

#[derive(Debug)]
pub struct StatusQuery;

#[derive(Debug, Clone)]
pub struct StatusResponse {
    pub active_drones: i32,
    pub queued_jobs: i32,
}
```

- [ ] **Step 2: Add AuthRequested to QueenEvent**

In `src/queen/src/notifier/mod.rs`, add a new variant to `QueenEvent`:

```rust
AuthRequested {
    job_run_id: String,
    url: String,
    message: String,
},
```

- [ ] **Step 3: Handle AuthRequested in LogNotifier**

In `src/queen/src/notifier/log.rs`, add the match arm:

```rust
QueenEvent::AuthRequested { job_run_id, url, message } => {
    tracing::warn!(%job_run_id, %url, %message, "drone requires auth - visit URL to approve");
}
```

- [ ] **Step 4: Verify build**

Run: `cd src/queen && cargo check`
Expected: compile errors in poller.rs (SpawnRequest has new required fields) — expected, will fix in Task 4

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/messages.rs src/queen/src/notifier/
git commit -m "feat(queen): update SpawnRequest with job details and add AuthRequested event"
```

---

### Task 4: Update Poller to fetch job definitions

**Files:**
- Modify: `src/queen/src/overseer_client.rs`
- Modify: `src/queen/src/actors/poller.rs`

- [ ] **Step 1: Add JobDefinitionResponse and get_job_definition to OverseerClient**

In `src/queen/src/overseer_client.rs`, add the response type:

```rust
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct JobDefinitionResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub config: Value,
}
```

Add the method to `OverseerClient`:

```rust
pub async fn get_job_definition(&self, id: &str) -> Result<JobDefinitionResponse> {
    let response = self
        .client
        .get(format!("{}/api/jobs/definitions/{id}", self.base_url))
        .send()
        .await?
        .error_for_status()?
        .json::<JobDefinitionResponse>()
        .await?;
    Ok(response)
}
```

- [ ] **Step 2: Update Poller to extract job details from definition**

Replace the job processing loop body in `src/queen/src/actors/poller.rs`. For each new run, fetch the job definition and extract `drone_type`, `repo_url`, `branch`, and `task` from its config:

```rust
        for run in runs {
            current_ids.insert(run.id.clone());
            if known_runs.contains(&run.id) {
                continue;
            }

            // Fetch the job definition to get drone_type, repo, and task details
            let def = match client.get_job_definition(&run.definition_id).await {
                Ok(def) => def,
                Err(e) => {
                    tracing::warn!(
                        job_run_id = %run.id,
                        definition_id = %run.definition_id,
                        error = %e,
                        "failed to fetch job definition, skipping run"
                    );
                    continue;
                }
            };

            let drone_type = def.config.get("drone_type")
                .and_then(|v| v.as_str())
                .unwrap_or("claude-drone")
                .to_string();

            let repo_url = def.config.get("repo_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let branch = def.config.get("branch")
                .and_then(|v| v.as_str())
                .map(String::from);

            let task = def.config.get("task")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let request = SpawnRequest {
                job_run_id: run.id.clone(),
                drone_type,
                job_config: def.config.clone(),
                repo_url,
                branch,
                task,
            };

            if spawn_tx.send(request).await.is_err() {
                tracing::warn!("supervisor channel closed, stopping poller");
                return;
            }
        }
```

- [ ] **Step 3: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles (supervisor.rs may have warnings about unused fields — that's fine, Task 5 will use them)

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/overseer_client.rs src/queen/src/actors/poller.rs
git commit -m "feat(queen): poller fetches job definitions for drone_type and job details"
```

---

### Task 5: Rewrite Supervisor to use drone protocol

**Files:**
- Modify: `src/queen/src/actors/supervisor.rs`
- Modify: `src/queen/src/overseer_client.rs`
- Modify: `src/queen/src/main.rs`

This is the core integration task. The Supervisor replaces `sleep infinity` with real drone binary invocation and protocol handling.

- [ ] **Step 1: Add store_artifact method to OverseerClient**

In `src/queen/src/overseer_client.rs`, add:

```rust
#[derive(Debug, Serialize)]
struct StoreArtifactRequest {
    name: String,
    content_type: String,
    data: String,
    run_id: Option<String>,
}

/// Store drone output (conversation history) as an artifact in Overseer.
pub async fn store_artifact(
    &self,
    name: &str,
    content_type: &str,
    data: &[u8],
    run_id: Option<&str>,
) -> Result<Value> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let body = StoreArtifactRequest {
        name: name.to_string(),
        content_type: content_type.to_string(),
        data: encoded,
        run_id: run_id.map(String::from),
    };
    let response = self
        .client
        .post(format!("{}/api/artifacts", self.base_url))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    Ok(response)
}
```

Add `base64 = "0.22"` to `src/queen/Cargo.toml` and `"//third-party:base64"` to `QUEEN_DEPS` in the BUCK file.

- [ ] **Step 2: Rewrite supervisor.rs**

Replace `src/queen/src/actors/supervisor.rs` with the full integrated version:

```rust
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use drone_sdk::protocol::{DroneMessage, JobSpec, QueenMessage};

use crate::messages::{SpawnRequest, StatusQuery, StatusResponse};
use crate::notifier::{Notifier, QueenEvent};
use crate::overseer_client::OverseerClient;

struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: tokio::process::Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
    protocol_rx: mpsc::Receiver<DroneMessage>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: OverseerClient,
    notifier: Arc<dyn Notifier>,
    max_concurrency: i32,
    default_timeout: Duration,
    stall_threshold: Duration,
    drone_dir: PathBuf,
    mut spawn_rx: mpsc::Receiver<SpawnRequest>,
    mut status_rx: mpsc::Receiver<(StatusQuery, oneshot::Sender<StatusResponse>)>,
    token: CancellationToken,
) {
    let mut active: HashMap<String, DroneHandle> = HashMap::new();
    let mut queue: VecDeque<SpawnRequest> = VecDeque::new();
    let mut health_ticker = tokio::time::interval(Duration::from_secs(5));

    loop {
        // Drain protocol messages from all active drones (non-blocking)
        drain_protocol_messages(&client, &notifier, &mut active).await;

        tokio::select! {
            Some(request) = spawn_rx.recv() => {
                if (active.len() as i32) < max_concurrency {
                    spawn_drone(&client, &notifier, &mut active, request, default_timeout, &drone_dir).await;
                } else {
                    tracing::info!(job_run_id = %request.job_run_id, "concurrency limit reached, queueing");
                    queue.push_back(request);
                }
            }

            Some((_, resp_tx)) = status_rx.recv() => {
                let _ = resp_tx.send(StatusResponse {
                    active_drones: active.len() as i32,
                    queued_jobs: queue.len() as i32,
                });
            }

            _ = health_ticker.tick() => {
                check_drones(&client, &notifier, &mut active, stall_threshold).await;

                while (active.len() as i32) < max_concurrency {
                    if let Some(request) = queue.pop_front() {
                        spawn_drone(&client, &notifier, &mut active, request, default_timeout, &drone_dir).await;
                    } else {
                        break;
                    }
                }
            }

            _ = token.cancelled() => {
                tracing::info!("supervisor cancelled, shutting down drones");
                break;
            }

            else => {
                tracing::info!("all channels closed, supervisor exiting");
                break;
            }
        }
    }

    shutdown_all(&client, &mut active).await;
}

async fn spawn_drone(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    request: SpawnRequest,
    default_timeout: Duration,
    drone_dir: &PathBuf,
) {
    let binary_path = drone_dir.join(&request.drone_type);

    if !binary_path.exists() {
        tracing::error!(
            job_run_id = %request.job_run_id,
            drone_type = %request.drone_type,
            path = %binary_path.display(),
            "drone binary not found"
        );
        let _ = client
            .update_job_run(
                &request.job_run_id,
                Some("failed"),
                None,
                Some(&format!("drone binary not found: {}", binary_path.display())),
            )
            .await;
        notifier
            .notify(QueenEvent::DroneFailed {
                job_run_id: request.job_run_id,
                error: format!("binary not found: {}", binary_path.display()),
            })
            .await;
        return;
    }

    tracing::info!(
        job_run_id = %request.job_run_id,
        drone_type = %request.drone_type,
        binary = %binary_path.display(),
        "spawning drone"
    );

    // Spawn drone binary with stdin/stdout piped for protocol
    let mut process = match tokio::process::Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(
                job_run_id = %request.job_run_id,
                error = %e,
                "failed to spawn drone process"
            );
            let _ = client
                .update_job_run(
                    &request.job_run_id,
                    Some("failed"),
                    None,
                    Some(&format!("failed to spawn: {e}")),
                )
                .await;
            notifier
                .notify(QueenEvent::DroneFailed {
                    job_run_id: request.job_run_id,
                    error: e.to_string(),
                })
                .await;
            return;
        }
    };

    // Write JobSpec to drone's stdin
    let job_spec = QueenMessage::Job(JobSpec {
        job_run_id: request.job_run_id.clone(),
        repo_url: request.repo_url.clone(),
        branch: request.branch.clone(),
        task: request.task.clone(),
        config: request.job_config.clone(),
    });

    if let Some(mut stdin) = process.stdin.take() {
        let job_json = match serde_json::to_string(&job_spec) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize job spec");
                let _ = process.kill().await;
                return;
            }
        };
        // Write synchronously in a blocking task to avoid holding the process
        let write_result = tokio::task::spawn_blocking(move || {
            writeln!(stdin, "{job_json}").and_then(|()| stdin.flush())
        })
        .await;
        if let Err(e) = write_result {
            tracing::error!(error = %e, "failed to write job spec to drone stdin");
            let _ = process.kill().await;
            return;
        }
    }

    // Spawn background reader task for drone's stdout (protocol messages)
    let (protocol_tx, protocol_rx) = mpsc::channel::<DroneMessage>(32);
    let stdout = process.stdout.take();
    let reader_job_id = request.job_run_id.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(stdout) = stdout {
            let reader = std::io::BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) if line.trim().is_empty() => continue,
                    Ok(line) => match serde_json::from_str::<DroneMessage>(&line) {
                        Ok(msg) => {
                            if protocol_tx.blocking_send(msg).is_err() {
                                break; // Supervisor dropped the receiver
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                job_run_id = %reader_job_id,
                                error = %e,
                                line = %line,
                                "failed to parse drone protocol message"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::debug!(
                            job_run_id = %reader_job_id,
                            error = %e,
                            "drone stdout closed"
                        );
                        break;
                    }
                }
            }
        }
    });

    let now = Instant::now();
    let handle = DroneHandle {
        job_run_id: request.job_run_id.clone(),
        drone_type: request.drone_type.clone(),
        process,
        started_at: now,
        timeout: default_timeout,
        last_activity: now,
        protocol_rx,
    };

    notifier
        .notify(QueenEvent::DroneSpawned {
            job_run_id: request.job_run_id.clone(),
            drone_type: request.drone_type,
        })
        .await;
    active.insert(request.job_run_id, handle);
}

/// Drain all pending protocol messages from active drones.
async fn drain_protocol_messages(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
) {
    let mut completed = Vec::new();

    for (id, handle) in active.iter_mut() {
        while let Ok(msg) = handle.protocol_rx.try_recv() {
            handle.last_activity = Instant::now();

            match msg {
                DroneMessage::Progress(p) => {
                    tracing::info!(
                        job_run_id = %id,
                        status = %p.status,
                        detail = ?p.detail,
                        "drone progress"
                    );
                }
                DroneMessage::AuthRequest(auth) => {
                    notifier
                        .notify(QueenEvent::AuthRequested {
                            job_run_id: id.clone(),
                            url: auth.url,
                            message: auth.message,
                        })
                        .await;
                    // v1: no auth_response sent back — drone will time out
                }
                DroneMessage::Result(output) => {
                    tracing::info!(
                        job_run_id = %id,
                        exit_code = output.exit_code,
                        artifacts = ?output.artifacts,
                        git_branch = ?output.git_refs.branch,
                        git_pr = ?output.git_refs.pr_url,
                        "drone completed with result"
                    );

                    // Store conversation as artifact in Overseer
                    let conversation_json = serde_json::to_vec_pretty(&output.conversation)
                        .unwrap_or_default();
                    if let Err(e) = client
                        .store_artifact(
                            &format!("{id}-conversation.json"),
                            "application/json",
                            &conversation_json,
                            Some(id),
                        )
                        .await
                    {
                        tracing::warn!(
                            job_run_id = %id,
                            error = %e,
                            "failed to store conversation artifact"
                        );
                    }

                    // Update job run in Overseer
                    let result_value = serde_json::json!({
                        "exit_code": output.exit_code,
                        "artifacts": output.artifacts,
                        "git_refs": {
                            "branch": output.git_refs.branch,
                            "pr_url": output.git_refs.pr_url,
                        }
                    });

                    let status = if output.exit_code == 0 { "completed" } else { "failed" };
                    let error = if output.exit_code != 0 {
                        Some(format!("drone exited with code {}", output.exit_code))
                    } else {
                        None
                    };

                    let _ = client
                        .update_job_run(id, Some(status), Some(result_value), error.as_deref())
                        .await;

                    if output.exit_code == 0 {
                        notifier
                            .notify(QueenEvent::DroneCompleted {
                                job_run_id: id.clone(),
                                exit_code: output.exit_code,
                            })
                            .await;
                    } else {
                        notifier
                            .notify(QueenEvent::DroneFailed {
                                job_run_id: id.clone(),
                                error: format!("exit code {}", output.exit_code),
                            })
                            .await;
                    }

                    completed.push(id.clone());
                }
                DroneMessage::Error(e) => {
                    tracing::error!(job_run_id = %id, error = %e.message, "drone reported error");
                    let _ = client
                        .update_job_run(id, Some("failed"), None, Some(&e.message))
                        .await;
                    notifier
                        .notify(QueenEvent::DroneFailed {
                            job_run_id: id.clone(),
                            error: e.message,
                        })
                        .await;
                    completed.push(id.clone());
                }
            }
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

async fn check_drones(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    stall_threshold: Duration,
) {
    let now = Instant::now();
    let mut completed = Vec::new();

    for (id, handle) in active.iter_mut() {
        // Check if process exited (catches cases where drone exits without sending Result)
        match handle.process.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                tracing::warn!(
                    job_run_id = %id,
                    exit_code,
                    "drone process exited without sending result"
                );
                let _ = client
                    .update_job_run(
                        id,
                        Some("failed"),
                        None,
                        Some(&format!("process exited unexpectedly with code {exit_code}")),
                    )
                    .await;
                notifier
                    .notify(QueenEvent::DroneFailed {
                        job_run_id: id.clone(),
                        error: format!("unexpected exit code {exit_code}"),
                    })
                    .await;
                completed.push(id.clone());
                continue;
            }
            Ok(None) => {} // Still running
            Err(e) => {
                tracing::error!(job_run_id = %id, error = %e, "failed to check drone status");
                continue;
            }
        }

        // Check timeout
        if now.duration_since(handle.started_at) > handle.timeout {
            tracing::warn!(job_run_id = %id, "drone timed out, killing");
            let _ = handle.process.kill().await;
            let _ = handle.process.wait().await;
            let _ = client
                .update_job_run(id, Some("failed"), None, Some("timed out"))
                .await;
            notifier
                .notify(QueenEvent::DroneTimedOut {
                    job_run_id: id.clone(),
                })
                .await;
            completed.push(id.clone());
            continue;
        }

        // Check stall (activity-based — last_activity updated by protocol messages)
        if now.duration_since(handle.last_activity) > stall_threshold {
            notifier
                .notify(QueenEvent::DroneStalled {
                    job_run_id: id.clone(),
                    last_activity_secs: now.duration_since(handle.last_activity).as_secs(),
                })
                .await;
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

async fn shutdown_all(client: &OverseerClient, active: &mut HashMap<String, DroneHandle>) {
    for (id, mut handle) in active.drain() {
        tracing::info!(job_run_id = %id, "killing drone for shutdown");
        let _ = handle.process.kill().await;
        let _ = handle.process.wait().await;
        let _ = client
            .update_job_run(&id, Some("cancelled"), None, Some("queen shutting down"))
            .await;
    }
}
```

- [ ] **Step 3: Update main.rs to pass drone_dir**

In `src/queen/src/main.rs`, update the supervisor spawn to pass `drone_dir`:

Add `use std::path::PathBuf;` to imports.

Change the supervisor invocation:
```rust
    let drone_dir = PathBuf::from(&config.queen.drone_dir);
```

And update the `actors::supervisor::run(...)` call to include `drone_dir` after `stall_threshold`:
```rust
        actors::supervisor::run(
            supervisor_client,
            supervisor_notifier,
            max_concurrency,
            default_timeout,
            stall_threshold,
            drone_dir,
            spawn_rx,
            status_query_rx,
            supervisor_token,
        )
```

- [ ] **Step 4: Run buckify if base64 was added**

Run: `./tools/buckify.sh` (base64 is already in the workspace from overseer, but BUCK needs the dep)

- [ ] **Step 5: Verify build**

Run: `cd src/queen && cargo check`
Expected: compiles

- [ ] **Step 6: Run all tests**

Run: `cd src/queen && cargo test`
Run: `cd src/overseer && cargo test`
Expected: ALL PASS

- [ ] **Step 7: Commit**

```bash
git add src/queen/ src/overseer/ Cargo.lock
git commit -m "feat(queen): integrate real drone protocol in supervisor

- Spawn drone binaries from drone_dir with stdin/stdout piped
- Write JobSpec via drone-sdk protocol types
- Background reader task parses DroneMessage from stdout
- Handle Progress, AuthRequest, Result, Error protocol messages
- Store conversation history as Overseer artifact
- Stall detection now based on protocol activity instead of Overseer polling
- Remove sleep infinity placeholder"
```

---

### Task 6: Build verification

**Files:**
- None modified — verification only

- [ ] **Step 1: Run all tests**

Run: `cd src/overseer && cargo test`
Run: `cd src/queen && cargo test`
Run: `cd src/drone-sdk && cargo test`
Expected: ALL PASS

- [ ] **Step 2: Build everything with Buck2**

Run: `buck2 build root//...`
Expected: BUILD SUCCEEDED

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p queen -p overseer -p drone-sdk -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: All hooks pass

- [ ] **Step 5: Commit any formatting fixes**

```bash
git add -u
git commit -m "style: apply cargo fmt"
```
