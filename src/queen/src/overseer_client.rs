use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct HatcheryResponse {
    pub id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Value,
    pub max_concurrency: i32,
    pub active_drones: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobRunResponse {
    pub id: String,
    pub definition_id: String,
    pub parent_id: Option<String>,
    pub status: String,
    pub triggered_by: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskResponse {
    pub id: String,
    pub run_id: Option<String>,
    pub subject: String,
    pub status: String,
    pub assigned_to: Option<String>,
    pub output: Option<Value>,
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RegisterRequest {
    pub name: String,
    pub capabilities: Value,
    pub max_concurrency: i32,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatRequest {
    pub status: String,
    pub active_drones: i32,
}

#[derive(Debug, Serialize)]
pub struct UpdateJobRunRequest {
    pub status: Option<String>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

// ── Client ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OverseerClient {
    base_url: String,
    client: reqwest::Client,
    hatchery_id: Arc<RwLock<Option<String>>>,
}

impl OverseerClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
            hatchery_id: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn hatchery_id(&self) -> Option<String> {
        self.hatchery_id.read().await.clone()
    }

    async fn require_hatchery_id(&self) -> Result<String> {
        self.hatchery_id
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("not registered with overseer"))
    }

    pub async fn register(
        &self,
        name: &str,
        capabilities: Value,
        max_concurrency: i32,
    ) -> Result<HatcheryResponse> {
        let body = RegisterRequest {
            name: name.to_string(),
            capabilities,
            max_concurrency,
        };
        let response = self
            .client
            .post(format!("{}/api/hatcheries", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<HatcheryResponse>()
            .await?;

        *self.hatchery_id.write().await = Some(response.id.clone());
        Ok(response)
    }

    pub async fn heartbeat(&self, status: &str, active_drones: i32) -> Result<HatcheryResponse> {
        let id = self.require_hatchery_id().await?;
        let body = HeartbeatRequest {
            status: status.to_string(),
            active_drones,
        };
        let response = self
            .client
            .post(format!("{}/api/hatcheries/{id}/heartbeat", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<HatcheryResponse>()
            .await?;
        Ok(response)
    }

    pub async fn poll_jobs(&self) -> Result<Vec<JobRunResponse>> {
        let id = self.require_hatchery_id().await?;
        let response = self
            .client
            .get(format!(
                "{}/api/hatcheries/{id}/jobs?status=pending",
                self.base_url
            ))
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<JobRunResponse>>()
            .await?;
        Ok(response)
    }

    pub async fn update_job_run(
        &self,
        id: &str,
        status: Option<&str>,
        result: Option<Value>,
        error: Option<&str>,
    ) -> Result<JobRunResponse> {
        let body = UpdateJobRunRequest {
            status: status.map(str::to_string),
            result,
            error: error.map(str::to_string),
        };
        let response = self
            .client
            .patch(format!("{}/api/jobs/runs/{id}", self.base_url))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<JobRunResponse>()
            .await?;
        Ok(response)
    }

    pub async fn get_tasks_for_run(&self, run_id: &str) -> Result<Vec<TaskResponse>> {
        let response = self
            .client
            .get(format!("{}/api/tasks?run_id={run_id}", self.base_url))
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<TaskResponse>>()
            .await?;
        Ok(response)
    }

    pub async fn deregister(&self) -> Result<()> {
        let id = self.require_hatchery_id().await?;
        self.client
            .delete(format!("{}/api/hatcheries/{id}", self.base_url))
            .send()
            .await?
            .error_for_status()?;
        *self.hatchery_id.write().await = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = OverseerClient::new("http://localhost:3100");
        assert_eq!(client.base_url, "http://localhost:3100");
    }

    #[tokio::test]
    async fn test_hatchery_id_initially_none() {
        let client = OverseerClient::new("http://localhost:3100");
        assert!(client.hatchery_id().await.is_none());
    }

    #[tokio::test]
    async fn test_require_hatchery_id_fails_before_register() {
        let client = OverseerClient::new("http://localhost:3100");
        let result = client.require_hatchery_id().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not registered with overseer")
        );
    }
}
