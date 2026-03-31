use async_trait::async_trait;

use super::{Notifier, QueenEvent};

pub struct LogNotifier;

#[async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, event: QueenEvent) {
        match event {
            QueenEvent::HatcheryRegistered { name, id } => {
                tracing::info!(%name, %id, "hatchery registered with overseer");
            }
            QueenEvent::DroneSpawned {
                job_run_id,
                drone_type,
            } => {
                tracing::info!(%job_run_id, %drone_type, "drone spawned");
            }
            QueenEvent::DroneCompleted {
                job_run_id,
                exit_code,
            } => {
                tracing::info!(%job_run_id, %exit_code, "drone completed");
            }
            QueenEvent::AuthRequested {
                job_run_id,
                url,
                message,
            } => {
                tracing::warn!(%job_run_id, %url, %message, "drone requires auth - visit URL to approve");
            }
            QueenEvent::DroneFailed { job_run_id, error } => {
                tracing::warn!(%job_run_id, %error, "drone failed");
            }
            QueenEvent::DroneStalled {
                job_run_id,
                last_activity_secs,
            } => {
                tracing::warn!(%job_run_id, %last_activity_secs, "drone stalled");
            }
            QueenEvent::DroneTimedOut { job_run_id } => {
                tracing::warn!(%job_run_id, "drone timed out");
            }
            QueenEvent::CreepStarted => {
                tracing::info!("creep sidecar started");
            }
            QueenEvent::CreepDied { restart_in_secs } => {
                tracing::warn!(%restart_in_secs, "creep sidecar died, restarting");
            }
            QueenEvent::ShuttingDown => {
                tracing::info!("queen shutting down");
            }
        }
    }
}
