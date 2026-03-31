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

            let drone_type = run.triggered_by.clone();

            let request = SpawnRequest {
                job_run_id: run.id.clone(),
                drone_type,
                job_config: run.result.unwrap_or(serde_json::json!({})),
            };

            if spawn_tx.send(request).await.is_err() {
                tracing::warn!("supervisor channel closed, stopping poller");
                return;
            }
        }
        known_runs = current_ids;
    }
}
