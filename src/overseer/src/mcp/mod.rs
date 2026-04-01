use std::sync::Arc;

use base64::Engine;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};

use crate::services::AppState;

// ──────────────────────────── parameter structs ────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StoreMemoryParams {
    #[schemars(description = "The text content to store")]
    pub content: String,
    #[schemars(description = "Identifies the agent or component that produced this memory")]
    pub source: String,
    #[serde(default)]
    #[schemars(description = "Optional list of tags for filtering")]
    pub tags: Vec<String>,
    #[schemars(description = "Optional ISO-8601 expiry timestamp")]
    pub expires_at: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RecallMemoryParams {
    #[schemars(description = "Natural-language search query")]
    pub query: String,
    #[serde(default)]
    #[schemars(description = "Filter results to these tags (empty = no filter)")]
    pub tags: Vec<String>,
    #[serde(default = "default_limit")]
    #[schemars(description = "Maximum number of results to return (default 10)")]
    pub limit: usize,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteMemoryParams {
    #[schemars(description = "ID of the memory to delete")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LogDecisionParams {
    #[schemars(description = "Agent name that made the decision")]
    pub agent: String,
    #[schemars(description = "Context in which the decision was made")]
    pub context: String,
    #[schemars(description = "The decision that was made")]
    pub decision: String,
    #[serde(default)]
    #[schemars(description = "Reasoning behind the decision")]
    pub reasoning: String,
    #[serde(default)]
    #[schemars(description = "Tags to categorise this decision")]
    pub tags: Vec<String>,
    #[schemars(description = "Optional job run ID to associate the decision with")]
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QueryDecisionsParams {
    #[schemars(description = "Filter by agent name")]
    pub agent: Option<String>,
    #[serde(default)]
    #[schemars(description = "Filter by tags (empty = no filter)")]
    pub tags: Vec<String>,
    #[serde(default = "default_limit_i64")]
    #[schemars(description = "Maximum number of results to return (default 20)")]
    pub limit: i64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateJobDefinitionParams {
    #[schemars(description = "Unique name for the job definition")]
    pub name: String,
    #[serde(default)]
    #[schemars(description = "Human-readable description")]
    pub description: String,
    #[serde(default = "serde_json::Value::default")]
    #[schemars(description = "Arbitrary JSON configuration for the job")]
    pub config: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StartJobParams {
    #[schemars(description = "ID of the job definition to run")]
    pub definition_id: String,
    #[schemars(description = "Agent or entity that triggered this run")]
    pub triggered_by: String,
    #[schemars(description = "Optional parent job run ID")]
    pub parent_id: Option<String>,
    #[schemars(description = "Optional config overrides (shallow merge with definition config)")]
    pub config_overrides: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateJobRunParams {
    #[schemars(description = "Job run ID to update")]
    pub id: String,
    #[schemars(description = "New status (e.g. completed, failed)")]
    pub status: Option<String>,
    #[schemars(description = "JSON result payload")]
    pub result: Option<serde_json::Value>,
    #[schemars(description = "Error message if the run failed")]
    pub error: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    #[schemars(description = "Short description of the task")]
    pub subject: String,
    #[schemars(description = "Optional job run ID this task belongs to")]
    pub run_id: Option<String>,
    #[schemars(description = "Agent assigned to this task")]
    pub assigned_to: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskParams {
    #[schemars(description = "Task ID to update")]
    pub id: String,
    #[schemars(description = "New task status")]
    pub status: Option<String>,
    #[schemars(description = "Reassign to a different agent")]
    pub assigned_to: Option<String>,
    #[schemars(description = "JSON output from the task")]
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    #[schemars(description = "Filter by task status")]
    pub status: Option<String>,
    #[schemars(description = "Filter by assigned agent")]
    pub assigned_to: Option<String>,
    #[schemars(description = "Filter by job run ID")]
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StoreArtifactParams {
    #[schemars(description = "Filename / display name for the artifact")]
    pub name: String,
    #[schemars(description = "MIME content type (e.g. application/json)")]
    pub content_type: String,
    #[schemars(description = "Base64-encoded artifact data")]
    pub data: String,
    #[schemars(description = "Optional job run ID to associate with")]
    pub run_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetArtifactParams {
    #[schemars(description = "Artifact ID to retrieve")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RegisterHatcheryParams {
    #[schemars(description = "Unique name for this hatchery instance")]
    pub name: String,
    #[serde(default = "serde_json::Value::default")]
    #[schemars(description = "JSON describing available architectures, drone types, etc.")]
    pub capabilities: serde_json::Value,
    #[serde(default = "default_max_concurrency")]
    #[schemars(description = "Maximum number of concurrent drones (default 1)")]
    pub max_concurrency: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct HeartbeatHatcheryParams {
    #[schemars(description = "Hatchery ID")]
    pub id: String,
    #[schemars(description = "Current status: online, degraded, or offline")]
    pub status: String,
    #[schemars(description = "Number of currently running drone sessions")]
    pub active_drones: i32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListHatcheriesParams {
    #[schemars(description = "Filter by status")]
    pub status: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeregisterHatcheryParams {
    #[schemars(description = "Hatchery ID to deregister")]
    pub id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignJobToHatcheryParams {
    #[schemars(description = "Job run ID to assign")]
    pub job_run_id: String,
    #[schemars(description = "Hatchery ID to assign the job to")]
    pub hatchery_id: String,
}

// ──────────────────────────────── defaults ─────────────────────────────────

fn default_limit() -> usize {
    10
}

fn default_limit_i64() -> i64 {
    20
}

fn default_max_concurrency() -> i32 {
    1
}

// ──────────────────────────── MCP server struct ────────────────────────────

#[derive(Clone)]
pub struct OverseerMcp {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

// ──────────────────────────────── tool impl ────────────────────────────────

#[tool_router]
impl OverseerMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    // ── memory ──────────────────────────────────────────────────────────────

    #[tool(description = "Store a memory fragment for later retrieval by semantic search")]
    async fn store_memory(
        &self,
        Parameters(p): Parameters<StoreMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .memory
            .store(&p.content, &p.source, &p.tags, p.expires_at.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Recall memories using semantic similarity search")]
    async fn recall_memory(
        &self,
        Parameters(p): Parameters<RecallMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let tags_filter = if p.tags.is_empty() {
            None
        } else {
            Some(p.tags.as_slice())
        };
        let results = self
            .state
            .memory
            .recall(&p.query, tags_filter, p.limit)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Delete a memory by ID")]
    async fn delete_memory(
        &self,
        Parameters(p): Parameters<DeleteMemoryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.state
            .memory
            .delete(&p.id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Memory {} deleted",
            p.id
        ))]))
    }

    // ── decisions ───────────────────────────────────────────────────────────

    #[tool(description = "Log an agent decision for future reference and auditing")]
    async fn log_decision(
        &self,
        Parameters(p): Parameters<LogDecisionParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .decisions
            .log(
                &p.agent,
                &p.context,
                &p.decision,
                &p.reasoning,
                &p.tags,
                p.run_id.as_deref(),
            )
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Query previously logged decisions, optionally filtered by agent or tags")]
    async fn query_decisions(
        &self,
        Parameters(p): Parameters<QueryDecisionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let tags_filter = if p.tags.is_empty() {
            None
        } else {
            Some(p.tags.as_slice())
        };
        let results = self
            .state
            .decisions
            .query(p.agent.as_deref(), tags_filter, p.limit)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // ── jobs ────────────────────────────────────────────────────────────────

    #[tool(description = "Create a new job definition (a reusable template for job runs)")]
    async fn create_job_definition(
        &self,
        Parameters(p): Parameters<CreateJobDefinitionParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .jobs
            .create_job_definition(&p.name, &p.description, p.config)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Start a new job run from an existing job definition")]
    async fn start_job(
        &self,
        Parameters(p): Parameters<StartJobParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .jobs
            .start_job_run(
                &p.definition_id,
                &p.triggered_by,
                p.parent_id.as_deref(),
                p.config_overrides,
            )
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Update the status, result, or error of a job run")]
    async fn update_job_run(
        &self,
        Parameters(p): Parameters<UpdateJobRunParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .jobs
            .update_job_run(&p.id, p.status.as_deref(), p.result, p.error.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // ── tasks ───────────────────────────────────────────────────────────────

    #[tool(description = "Create a new task, optionally linked to a job run")]
    async fn create_task(
        &self,
        Parameters(p): Parameters<CreateTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .jobs
            .create_task(&p.subject, p.run_id.as_deref(), p.assigned_to.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Update a task's status, assignment, or output")]
    async fn update_task(
        &self,
        Parameters(p): Parameters<UpdateTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .jobs
            .update_task(
                &p.id,
                p.status.as_deref(),
                p.assigned_to.as_deref(),
                p.output,
            )
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List tasks, optionally filtered by status, assignee, or job run")]
    async fn list_tasks(
        &self,
        Parameters(p): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self
            .state
            .jobs
            .list_tasks(
                p.status.as_deref(),
                p.assigned_to.as_deref(),
                p.run_id.as_deref(),
            )
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // ── hatcheries ──────────────────────────────────────────────────────────

    #[tool(description = "Register a new hatchery instance with Overseer")]
    async fn register_hatchery(
        &self,
        Parameters(p): Parameters<RegisterHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .register(&p.name, p.capabilities, p.max_concurrency)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Send a heartbeat from a hatchery, updating its status and active drone count"
    )]
    async fn heartbeat_hatchery(
        &self,
        Parameters(p): Parameters<HeartbeatHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .heartbeat(&p.id, &p.status, p.active_drones)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "List registered hatcheries, optionally filtered by status")]
    async fn list_hatcheries(
        &self,
        Parameters(p): Parameters<ListHatcheriesParams>,
    ) -> Result<CallToolResult, McpError> {
        let results = self
            .state
            .hatchery
            .list(p.status.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&results).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(description = "Deregister a hatchery instance")]
    async fn deregister_hatchery(
        &self,
        Parameters(p): Parameters<DeregisterHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        self.state
            .hatchery
            .deregister(&p.id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Hatchery {} deregistered",
            p.id
        ))]))
    }

    #[tool(description = "Assign a job run to a specific hatchery for execution")]
    async fn assign_job_to_hatchery(
        &self,
        Parameters(p): Parameters<AssignJobToHatcheryParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .state
            .hatchery
            .assign_job(&p.job_run_id, &p.hatchery_id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    // ── artifacts ───────────────────────────────────────────────────────────

    #[tool(description = "Store a binary artifact (base64-encoded). Returns artifact metadata.")]
    async fn store_artifact(
        &self,
        Parameters(p): Parameters<StoreArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&p.data)
            .map_err(|e| McpError::invalid_params(format!("invalid base64: {e}"), None))?;
        let result = self
            .state
            .artifacts
            .store(&p.name, &p.content_type, &data, p.run_id.as_deref())
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&result).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Retrieve artifact metadata by ID. The blob itself is available via the HTTP API."
    )]
    async fn get_artifact(
        &self,
        Parameters(p): Parameters<GetArtifactParams>,
    ) -> Result<CallToolResult, McpError> {
        let (metadata, _blob) = self
            .state
            .artifacts
            .get(&p.id)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let json = serde_json::to_string_pretty(&metadata).unwrap_or_else(|e| e.to_string());
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

// ──────────────────────────── ServerHandler impl ───────────────────────────

#[tool_handler]
impl ServerHandler for OverseerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("Overseer — persistent memory, jobs, decisions, artifacts".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
