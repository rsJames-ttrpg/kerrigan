pub mod log;
pub mod webhook;

use async_trait::async_trait;

#[derive(Debug, Clone)]
pub enum QueenEvent {
    HatcheryRegistered {
        name: String,
        id: String,
    },
    DroneSpawned {
        job_run_id: String,
        drone_type: String,
    },
    DroneCompleted {
        job_run_id: String,
        exit_code: i32,
    },
    DroneFailed {
        job_run_id: String,
        error: String,
    },
    DroneStalled {
        job_run_id: String,
        last_activity_secs: u64,
    },
    DroneTimedOut {
        job_run_id: String,
    },
    AuthRequested {
        job_run_id: String,
        url: String,
        message: String,
    },
    CreepStarted,
    CreepDied {
        restart_in_secs: u64,
    },
    ShuttingDown,
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, event: QueenEvent);
}
