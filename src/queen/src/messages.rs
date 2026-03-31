use serde_json::Value;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SpawnRequest {
    pub job_run_id: String,
    pub drone_type: String,
    pub job_config: Value,
}

#[derive(Debug)]
pub struct StatusQuery;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StatusResponse {
    pub active_drones: i32,
    pub queued_jobs: i32,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct DroneCompleted {
    pub job_run_id: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}
