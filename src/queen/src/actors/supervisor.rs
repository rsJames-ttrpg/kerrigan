use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};

use crate::messages::{SpawnRequest, StatusQuery, StatusResponse};
use crate::notifier::{Notifier, QueenEvent};
use crate::overseer_client::OverseerClient;

#[allow(dead_code)]
struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
}

pub async fn run(
    client: OverseerClient,
    notifier: Arc<dyn Notifier>,
    max_concurrency: i32,
    default_timeout: Duration,
    stall_threshold: Duration,
    mut spawn_rx: mpsc::Receiver<SpawnRequest>,
    mut status_rx: mpsc::Receiver<(StatusQuery, oneshot::Sender<StatusResponse>)>,
) {
    let mut active: HashMap<String, DroneHandle> = HashMap::new();
    let mut queue: VecDeque<SpawnRequest> = VecDeque::new();
    let mut health_ticker = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            Some(request) = spawn_rx.recv() => {
                if (active.len() as i32) < max_concurrency {
                    spawn_drone(&client, &notifier, &mut active, request, default_timeout).await;
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
                        spawn_drone(&client, &notifier, &mut active, request, default_timeout).await;
                    } else {
                        break;
                    }
                }
            }

            else => {
                tracing::info!("all channels closed, supervisor exiting");
                break;
            }
        }
    }

    shutdown_all(&client, &notifier, &mut active).await;
}

async fn spawn_drone(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    request: SpawnRequest,
    default_timeout: Duration,
) {
    tracing::info!(job_run_id = %request.job_run_id, drone_type = %request.drone_type, "spawning drone");

    // Placeholder: real drone launching comes from the Drone trait (not built yet).
    // For now, spawn a sleep process so the supervisor has something to manage.
    let process = match tokio::process::Command::new("sleep")
        .arg("infinity")
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(job_run_id = %request.job_run_id, error = %e, "failed to spawn drone process");
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

    let now = Instant::now();
    let handle = DroneHandle {
        job_run_id: request.job_run_id.clone(),
        drone_type: request.drone_type.clone(),
        process,
        started_at: now,
        timeout: default_timeout,
        last_activity: now,
    };

    notifier
        .notify(QueenEvent::DroneSpawned {
            job_run_id: request.job_run_id.clone(),
            drone_type: request.drone_type,
        })
        .await;
    active.insert(request.job_run_id, handle);
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
        // Check if process exited
        match handle.process.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status.code().unwrap_or(-1);
                if status.success() {
                    tracing::info!(job_run_id = %id, "drone process exited successfully");
                    let _ = client
                        .update_job_run(id, Some("completed"), None, None)
                        .await;
                    notifier
                        .notify(QueenEvent::DroneCompleted {
                            job_run_id: id.clone(),
                            exit_code,
                        })
                        .await;
                } else {
                    tracing::warn!(job_run_id = %id, exit_code, "drone process failed");
                    let _ = client
                        .update_job_run(
                            id,
                            Some("failed"),
                            None,
                            Some(&format!("process exited with code {exit_code}")),
                        )
                        .await;
                    notifier
                        .notify(QueenEvent::DroneFailed {
                            job_run_id: id.clone(),
                            error: format!("exit code {exit_code}"),
                        })
                        .await;
                }
                completed.push(id.clone());
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!(job_run_id = %id, error = %e, "failed to check drone status");
                continue;
            }
        }

        // Check timeout
        if now.duration_since(handle.started_at) > handle.timeout {
            tracing::warn!(job_run_id = %id, "drone timed out, killing");
            let _ = handle.process.kill().await;
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

        // Check stall
        if now.duration_since(handle.last_activity) > stall_threshold {
            notifier
                .notify(QueenEvent::DroneStalled {
                    job_run_id: id.clone(),
                    last_activity_secs: now.duration_since(handle.last_activity).as_secs(),
                })
                .await;
        }

        // Update last_activity from Overseer task data
        if let Ok(tasks) = client.get_tasks_for_run(id).await
            && let Some(latest) = tasks.last()
            && let Ok(updated) = chrono::DateTime::parse_from_rfc3339(&latest.updated_at)
        {
            let age = chrono::Utc::now() - updated.to_utc();
            if age.num_seconds() < stall_threshold.as_secs() as i64 {
                handle.last_activity = now;
            }
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

async fn shutdown_all(
    client: &OverseerClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
) {
    for (id, mut handle) in active.drain() {
        tracing::info!(job_run_id = %id, "killing drone for shutdown");
        let _ = handle.process.kill().await;
        let _ = client
            .update_job_run(&id, Some("cancelled"), None, Some("queen shutting down"))
            .await;
    }
    notifier.notify(QueenEvent::ShuttingDown).await;
}
