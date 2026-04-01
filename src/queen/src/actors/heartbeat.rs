use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::messages::{StatusQuery, StatusResponse};
use nydus::NydusClient;

/// Periodic actor: sends heartbeats to Overseer with current drone status.
pub async fn run(
    client: NydusClient,
    interval_secs: u64,
    hatchery_id: Arc<RwLock<Option<String>>>,
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

        let id = match hatchery_id.read().await.as_ref() {
            Some(id) => id.clone(),
            None => {
                tracing::warn!("no hatchery id yet, skipping heartbeat");
                continue;
            }
        };

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
        if let Err(e) = client
            .heartbeat(&id, status, status_resp.active_drones)
            .await
        {
            tracing::warn!(error = %e, "heartbeat failed");
        }
    }
}
