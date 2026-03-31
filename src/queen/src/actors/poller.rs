use std::collections::HashSet;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::messages::SpawnRequest;
use crate::overseer_client::OverseerClient;

/// Periodic actor: polls Overseer for jobs assigned to this hatchery.
pub async fn run(
    client: OverseerClient,
    interval_secs: u64,
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

        let runs = match client.poll_jobs().await {
            Ok(runs) => runs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to poll jobs from overseer");
                continue;
            }
        };

        let mut current_ids: HashSet<String> = HashSet::new();
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

            let drone_type = def
                .config
                .get("drone_type")
                .and_then(|v| v.as_str())
                .unwrap_or("claude-drone")
                .to_string();

            let repo_url = def
                .config
                .get("repo_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let branch = def
                .config
                .get("branch")
                .and_then(|v| v.as_str())
                .map(String::from);

            let task = def
                .config
                .get("task")
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
        known_runs = current_ids;
    }
}
