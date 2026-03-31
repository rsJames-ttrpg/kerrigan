use std::sync::Arc;
use std::time::Duration;

use crate::notifier::{Notifier, QueenEvent};
use crate::overseer_client::OverseerClient;

/// One-shot actor: registers this hatchery with Overseer, retries on failure.
pub async fn run(
    client: OverseerClient,
    name: String,
    max_concurrency: i32,
    notifier: Arc<dyn Notifier>,
) -> anyhow::Result<()> {
    let capabilities = serde_json::json!({});

    loop {
        match client
            .register(&name, capabilities.clone(), max_concurrency)
            .await
        {
            Ok(hatchery) => {
                notifier
                    .notify(QueenEvent::HatcheryRegistered {
                        name: name.clone(),
                        id: hatchery.id.clone(),
                    })
                    .await;
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to register with overseer, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
