use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub job_run_id: String,
    pub drone_type: String,
    pub job_config: Value,
}

#[derive(Debug)]
pub struct StatusQuery;

#[derive(Debug, Clone)]
pub struct StatusResponse {
    pub active_drones: i32,
    pub queued_jobs: i32,
}
