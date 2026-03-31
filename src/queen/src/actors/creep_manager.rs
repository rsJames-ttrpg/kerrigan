use std::sync::Arc;
use std::time::Duration;

use crate::config::CreepConfig;
use crate::notifier::{Notifier, QueenEvent};

pub async fn run(config: CreepConfig, notifier: Arc<dyn Notifier>) {
    if !config.enabled {
        tracing::info!("creep sidecar disabled in config");
        return;
    }

    loop {
        tracing::info!(binary = %config.binary, "starting creep sidecar");

        let mut child = match tokio::process::Command::new(&config.binary)
            .arg("--health-port")
            .arg(config.health_port.to_string())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                tracing::error!(error = %e, "failed to start creep, retrying in {}s", config.restart_delay);
                notifier
                    .notify(QueenEvent::CreepDied {
                        restart_in_secs: config.restart_delay,
                    })
                    .await;
                tokio::time::sleep(Duration::from_secs(config.restart_delay)).await;
                continue;
            }
        };

        notifier.notify(QueenEvent::CreepStarted).await;

        match child.wait().await {
            Ok(status) => {
                tracing::warn!(
                    exit_code = status.code().unwrap_or(-1),
                    "creep exited, restarting in {}s",
                    config.restart_delay
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "error waiting for creep");
            }
        }

        notifier
            .notify(QueenEvent::CreepDied {
                restart_in_secs: config.restart_delay,
            })
            .await;
        tokio::time::sleep(Duration::from_secs(config.restart_delay)).await;
    }
}
