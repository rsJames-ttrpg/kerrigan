use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::config::CreepConfig;
use crate::notifier::{Notifier, QueenEvent};

pub async fn run(config: CreepConfig, notifier: Arc<dyn Notifier>, token: CancellationToken) {
    if !config.enabled {
        tracing::info!("creep sidecar disabled in config");
        return;
    }

    loop {
        if token.is_cancelled() {
            tracing::info!("creep manager cancelled before starting");
            return;
        }

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
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(config.restart_delay)) => {}
                    _ = token.cancelled() => {
                        tracing::info!("creep manager cancelled during retry delay");
                        return;
                    }
                }
                continue;
            }
        };

        notifier.notify(QueenEvent::CreepStarted).await;

        tokio::select! {
            result = child.wait() => {
                match result {
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
            }
            _ = token.cancelled() => {
                tracing::info!("creep manager cancelled, killing creep process");
                let _ = child.kill().await;
                let _ = child.wait().await;
                return;
            }
        }

        notifier
            .notify(QueenEvent::CreepDied {
                restart_in_secs: config.restart_delay,
            })
            .await;

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(config.restart_delay)) => {}
            _ = token.cancelled() => {
                tracing::info!("creep manager cancelled during restart delay");
                return;
            }
        }
    }
}
