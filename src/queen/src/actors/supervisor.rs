use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write as IoWrite};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use std::sync::Arc;
use std::time::{Duration, Instant};

use drone_sdk::protocol::{DroneMessage, JobSpec, QueenMessage};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::messages::{SpawnRequest, StatusQuery, StatusResponse};
use crate::notifier::{Notifier, QueenEvent};
use nydus::NydusClient;

fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("gzip write failed");
    encoder.finish().expect("gzip finish failed")
}

#[allow(dead_code)]
struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
    protocol_rx: mpsc::Receiver<DroneMessage>,
    stdin_tx: Option<mpsc::Sender<QueenMessage>>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: NydusClient,
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
    client: &NydusClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    request: SpawnRequest,
    default_timeout: Duration,
    drone_dir: &Path,
) {
    tracing::info!(job_run_id = %request.job_run_id, drone_type = %request.drone_type, "spawning drone");

    if request.drone_type.contains('/')
        || request.drone_type.contains('\\')
        || request.drone_type.contains("..")
    {
        tracing::error!(
            job_run_id = %request.job_run_id,
            drone_type = %request.drone_type,
            "invalid drone_type: contains path separator"
        );
        if let Err(e) = client
            .update_run(
                &request.job_run_id,
                Some("failed"),
                None,
                Some("invalid drone_type"),
            )
            .await
        {
            tracing::error!(job_run_id = %request.job_run_id, error = %e, "failed to update job run in overseer");
        }
        notifier
            .notify(QueenEvent::DroneFailed {
                job_run_id: request.job_run_id,
                error: "invalid drone_type".to_string(),
            })
            .await;
        return;
    }

    let binary = drone_dir.join(&request.drone_type);
    let mut process = match tokio::process::Command::new(&binary)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            tracing::error!(job_run_id = %request.job_run_id, binary = %binary.display(), error = %e, "failed to spawn drone process");
            if let Err(e) = client
                .update_run(
                    &request.job_run_id,
                    Some("failed"),
                    None,
                    Some(&format!("failed to spawn {}: {e}", binary.display())),
                )
                .await
            {
                tracing::error!(job_run_id = %request.job_run_id, error = %e, "failed to update job run in overseer");
            }
            notifier
                .notify(QueenEvent::DroneFailed {
                    job_run_id: request.job_run_id,
                    error: e.to_string(),
                })
                .await;
            return;
        }
    };

    // Write JobSpec to stdin via blocking task, then keep stdin open for further messages
    let stdin = process.stdin.take().expect("stdin was piped");
    let job_spec = JobSpec {
        job_run_id: request.job_run_id.clone(),
        repo_url: request.repo_url.clone(),
        branch: request.branch.clone(),
        task: request.task.clone(),
        config: request.job_config.clone(),
    };
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<QueenMessage>(16);
    let job_run_id_for_stdin = request.job_run_id.clone();
    tokio::task::spawn_blocking(move || {
        let fd: std::os::fd::OwnedFd = stdin.into_owned_fd().expect("take stdin fd");
        let mut stdin = std::process::ChildStdin::from(fd);
        let msg = QueenMessage::Job(job_spec);
        match serde_json::to_writer(&mut stdin, &msg) {
            Ok(()) => {
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
            }
            Err(e) => {
                tracing::error!(job_run_id = %job_run_id_for_stdin, error = %e, "failed to write job spec to drone stdin");
                return;
            }
        }
        while let Some(msg) = stdin_rx.blocking_recv() {
            match serde_json::to_writer(&mut stdin, &msg) {
                Ok(()) => {
                    let _ = stdin.write_all(b"\n");
                    let _ = stdin.flush();
                }
                Err(e) => {
                    tracing::error!(job_run_id = %job_run_id_for_stdin, error = %e, "failed to write message to drone stdin");
                    break;
                }
            }
        }
    });

    // Read protocol messages from stdout via blocking task
    let stdout = process.stdout.take().expect("stdout was piped");
    let (protocol_tx, protocol_rx) = mpsc::channel::<DroneMessage>(64);
    let job_run_id_for_reader = request.job_run_id.clone();
    tokio::task::spawn_blocking(move || {
        let fd: std::os::fd::OwnedFd = stdout.into_owned_fd().expect("take stdout fd");
        let stdout = std::process::ChildStdout::from(fd);
        let reader = std::io::BufReader::new(stdout);
        for line_result in reader.lines() {
            match line_result {
                Ok(text) => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<DroneMessage>(&text) {
                        Ok(msg) => {
                            if protocol_tx.blocking_send(msg).is_err() {
                                break; // receiver dropped
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                job_run_id = %job_run_id_for_reader,
                                line = %text,
                                error = %e,
                                "failed to parse drone message"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(job_run_id = %job_run_id_for_reader, error = %e, "drone stdout closed");
                    break;
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
        stdin_tx: Some(stdin_tx),
    };

    notifier
        .notify(QueenEvent::DroneSpawned {
            job_run_id: request.job_run_id.clone(),
            drone_type: request.drone_type,
        })
        .await;
    active.insert(request.job_run_id, handle);
}

async fn drain_protocol_messages(
    client: &NydusClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
) {
    let mut completed = Vec::new();

    for (id, handle) in active.iter_mut() {
        loop {
            match handle.protocol_rx.try_recv() {
                Ok(msg) => {
                    handle.last_activity = Instant::now();
                    match msg {
                        DroneMessage::Progress(progress) => {
                            tracing::info!(
                                job_run_id = %id,
                                status = %progress.status,
                                detail = ?progress.detail,
                                "drone progress"
                            );
                        }
                        DroneMessage::AuthRequest(auth) => {
                            tracing::info!(
                                job_run_id = %id,
                                url = %auth.url,
                                message = %auth.message,
                                "drone auth request"
                            );
                            notifier
                                .notify(QueenEvent::AuthRequested {
                                    job_run_id: id.clone(),
                                    url: auth.url,
                                    message: auth.message,
                                })
                                .await;
                            // Poll Overseer for the auth code (user submits via API)
                            if let Some(tx) = handle.stdin_tx.clone() {
                                let poll_client = client.clone();
                                let poll_id = id.clone();
                                tokio::spawn(async move {
                                    loop {
                                        tokio::time::sleep(Duration::from_secs(2)).await;
                                        match poll_client.poll_auth_code(&poll_id).await {
                                            Ok(Some(code)) => {
                                                tracing::info!(job_run_id = %poll_id, "auth code received from user");
                                                let response = QueenMessage::AuthResponse(
                                                    drone_sdk::protocol::AuthResponse {
                                                        approved: true,
                                                        code: Some(code),
                                                    },
                                                );
                                                if tx.send(response).await.is_err() {
                                                    tracing::warn!(job_run_id = %poll_id, "failed to send auth response (stdin closed)");
                                                }
                                                break;
                                            }
                                            Ok(None) => {} // not yet submitted, keep polling
                                            Err(e) => {
                                                tracing::warn!(job_run_id = %poll_id, error = %e, "failed to poll auth code");
                                            }
                                        }
                                    }
                                });
                            }
                        }
                        DroneMessage::Result(output) => {
                            tracing::info!(
                                job_run_id = %id,
                                exit_code = output.exit_code,
                                "drone reported result"
                            );

                            // Store conversation as gzipped artifact
                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            let compressed = gzip_bytes(&conversation_bytes);
                            let artifact_name = format!("{id}-conversation.jsonl.gz");
                            if let Err(e) = client
                                .store_artifact(
                                    &artifact_name,
                                    "application/gzip",
                                    &compressed,
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

                            // Update job run status — require PR URL for success
                            let (status, error) =
                                if output.exit_code == 0 && output.git_refs.pr_url.is_none() {
                                    ("failed", Some("drone completed but no PR was created"))
                                } else if output.exit_code == 0 {
                                    ("completed", None)
                                } else {
                                    ("failed", None)
                                };
                            let result_value = serde_json::to_value(&output).ok();
                            if let Err(e) = client
                                .update_run(id, Some(status), result_value, error)
                                .await
                            {
                                tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
                            }

                            if status == "completed" {
                                notifier
                                    .notify(QueenEvent::DroneCompleted {
                                        job_run_id: id.clone(),
                                        exit_code: output.exit_code,
                                    })
                                    .await;
                            } else {
                                let error_msg = error
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| format!("exit code {}", output.exit_code));
                                notifier
                                    .notify(QueenEvent::DroneFailed {
                                        job_run_id: id.clone(),
                                        error: error_msg,
                                    })
                                    .await;
                            }

                            completed.push(id.clone());
                            break; // Result is terminal
                        }
                        DroneMessage::Error(err) => {
                            tracing::error!(
                                job_run_id = %id,
                                error = %err.message,
                                "drone reported error"
                            );
                            if let Err(e) = client
                                .update_run(id, Some("failed"), None, Some(&err.message))
                                .await
                            {
                                tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
                            }
                            notifier
                                .notify(QueenEvent::DroneFailed {
                                    job_run_id: id.clone(),
                                    error: err.message,
                                })
                                .await;
                            completed.push(id.clone());
                            break; // Error is terminal
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

async fn check_drones(
    client: &NydusClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    stall_threshold: Duration,
) {
    let now = Instant::now();
    let mut completed = Vec::new();

    for (id, handle) in active.iter_mut() {
        // Check if process exited (without a Result message)
        match handle.process.try_wait() {
            Ok(Some(status)) => {
                // Drain remaining protocol messages — drone may have sent Result just before exiting
                let mut got_result = false;
                while let Ok(msg) = handle.protocol_rx.try_recv() {
                    match msg {
                        DroneMessage::Result(output) => {
                            tracing::info!(job_run_id = %id, exit_code = output.exit_code, "drone completed with result");

                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            let compressed = gzip_bytes(&conversation_bytes);
                            if let Err(e) = client
                                .store_artifact(
                                    &format!("{id}-conversation.jsonl.gz"),
                                    "application/gzip",
                                    &compressed,
                                    Some(id),
                                )
                                .await
                            {
                                tracing::warn!(job_run_id = %id, error = %e, "failed to store conversation artifact");
                            }

                            let result_value = serde_json::to_value(&output).ok();
                            let (run_status, error) = if output.exit_code == 0
                                && output.git_refs.pr_url.is_none()
                            {
                                (
                                    "failed",
                                    Some("drone completed but no PR was created".to_string()),
                                )
                            } else if output.exit_code == 0 {
                                ("completed", None)
                            } else {
                                (
                                    "failed",
                                    Some(format!("drone exited with code {}", output.exit_code)),
                                )
                            };

                            if let Err(e) = client
                                .update_run(id, Some(run_status), result_value, error.as_deref())
                                .await
                            {
                                tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
                            }

                            if run_status == "completed" {
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
                                        error: error.clone().unwrap_or_else(|| {
                                            format!("exit code {}", output.exit_code)
                                        }),
                                    })
                                    .await;
                            }

                            got_result = true;
                        }
                        DroneMessage::Error(e) => {
                            tracing::error!(job_run_id = %id, error = %e.message, "drone reported error");
                            if let Err(e) = client
                                .update_run(id, Some("failed"), None, Some(&e.message))
                                .await
                            {
                                tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
                            }
                            notifier
                                .notify(QueenEvent::DroneFailed {
                                    job_run_id: id.clone(),
                                    error: e.message,
                                })
                                .await;
                            got_result = true;
                        }
                        _ => {} // Ignore progress/auth at this point
                    }
                }

                if !got_result {
                    // Process exited without sending Result — treat as unexpected
                    let exit_code = status.code().unwrap_or(-1);
                    tracing::warn!(job_run_id = %id, exit_code, "drone process exited without sending result");
                    if let Err(e) = client
                        .update_run(
                            id,
                            Some("failed"),
                            None,
                            Some(&format!(
                                "process exited unexpectedly with code {exit_code}"
                            )),
                        )
                        .await
                    {
                        tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
                    }
                    notifier
                        .notify(QueenEvent::DroneFailed {
                            job_run_id: id.clone(),
                            error: format!("unexpected exit code {exit_code}"),
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
            let _ = handle.process.wait().await;
            if let Err(e) = client
                .update_run(id, Some("failed"), None, Some("timed out"))
                .await
            {
                tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
            }
            notifier
                .notify(QueenEvent::DroneTimedOut {
                    job_run_id: id.clone(),
                })
                .await;
            completed.push(id.clone());
            continue;
        }

        // Check stall (based on protocol activity)
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

async fn shutdown_all(client: &NydusClient, active: &mut HashMap<String, DroneHandle>) {
    for (id, mut handle) in active.drain() {
        tracing::info!(job_run_id = %id, "killing drone for shutdown");
        let _ = handle.process.kill().await;
        let _ = handle.process.wait().await;
        if let Err(e) = client
            .update_run(&id, Some("cancelled"), None, Some("queen shutting down"))
            .await
        {
            tracing::error!(job_run_id = %id, error = %e, "failed to update job run in overseer");
        }
    }
    // ShuttingDown notification is sent from main.rs only
}
