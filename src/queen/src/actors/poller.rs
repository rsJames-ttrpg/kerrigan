use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use crate::messages::SpawnRequest;
use nydus::NydusClient;

/// Periodic actor: polls Overseer for jobs assigned to this hatchery.
pub async fn run(
    client: NydusClient,
    interval_secs: u64,
    hatchery_id: Arc<RwLock<Option<String>>>,
    spawn_tx: mpsc::Sender<SpawnRequest>,
    token: CancellationToken,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
    let mut known_runs: HashSet<String> = HashSet::new();

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = token.cancelled() => {
                tracing::info!("poller actor cancelled");
                return;
            }
        }

        let id = match hatchery_id.read().await.as_ref() {
            Some(id) => id.clone(),
            None => {
                tracing::warn!("no hatchery id yet, skipping poll");
                continue;
            }
        };

        let runs = match client.list_hatchery_jobs(&id, Some("pending")).await {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to poll jobs from overseer");
                continue;
            }
        };

        for run in runs {
            if known_runs.contains(&run.id) {
                continue;
            }

            // Fetch the job definition to get drone_type, repo, and task details
            let def = match client.get_definition(&run.definition_id).await {
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

            // Merge config_overrides on top of definition config
            let mut config = def.config.clone();
            if let Some(overrides) = &run.config_overrides {
                if let (Some(base), Some(over)) = (config.as_object_mut(), overrides.as_object()) {
                    for (k, v) in over {
                        base.insert(k.clone(), v.clone());
                    }
                }
            }

            let drone_type = config
                .get("drone_type")
                .and_then(|v| v.as_str())
                .unwrap_or("claude-drone")
                .to_string();

            let repo_url = match config.get("repo_url").and_then(|v| v.as_str()) {
                Some(url) if !url.is_empty() => url.to_string(),
                _ => {
                    tracing::warn!(
                        job_run_id = %run.id,
                        definition_id = %run.definition_id,
                        "job definition missing required 'repo_url' in config, skipping"
                    );
                    let _ = client
                        .update_run(
                            &run.id,
                            Some("failed"),
                            None,
                            Some("missing repo_url in job config"),
                        )
                        .await;
                    known_runs.insert(run.id);
                    continue;
                }
            };

            let branch = config
                .get("branch")
                .and_then(|v| v.as_str())
                .map(String::from);

            let task = match config.get("task").and_then(|v| v.as_str()) {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => {
                    tracing::warn!(
                        job_run_id = %run.id,
                        definition_id = %run.definition_id,
                        "job definition missing required 'task' in config, skipping"
                    );
                    let _ = client
                        .update_run(
                            &run.id,
                            Some("failed"),
                            None,
                            Some("missing task in job config"),
                        )
                        .await;
                    known_runs.insert(run.id);
                    continue;
                }
            };

            let request = SpawnRequest {
                job_run_id: run.id.clone(),
                drone_type,
                job_config: config,
                repo_url,
                branch,
                task,
            };

            if spawn_tx.send(request).await.is_err() {
                tracing::warn!("supervisor channel closed, stopping poller");
                return;
            }

            known_runs.insert(run.id);
        }
    }
}
