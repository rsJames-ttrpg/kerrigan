use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::messages::{StatusQuery, StatusResponse};
use crate::overseer_client::OverseerClient;

/// Periodic actor: sends heartbeats to Overseer with current drone status.
pub async fn run(
    client: OverseerClient,
    interval_secs: u64,
    status_tx: mpsc::Sender<(StatusQuery, oneshot::Sender<StatusResponse>)>,
    token: CancellationToken,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = token.cancelled() => {
                tracing::info!("heartbeat actor cancelled");
                return;
            }
        }

        let (resp_tx, resp_rx) = oneshot::channel();
        if status_tx.send((StatusQuery, resp_tx)).await.is_err() {
            tracing::warn!("supervisor channel closed, stopping heartbeat");
            return;
        }

        let status_resp = match resp_rx.await {
            Ok(resp) => resp,
            Err(_) => {
                tracing::warn!("supervisor did not respond to status query");
                continue;
            }
        };

        let status = "online";
        if let Err(e) = client.heartbeat(status, status_resp.active_drones).await {
            tracing::warn!(error = %e, "heartbeat failed");
        }
    }
}
