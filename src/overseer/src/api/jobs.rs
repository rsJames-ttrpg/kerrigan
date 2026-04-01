use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, patch, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/definitions", post(create_job_definition))
        .route("/definitions", get(list_job_definitions))
        .route("/definitions/{id}", get(get_job_definition))
        .route("/runs", post(start_job_run))
        .route("/runs", get(list_job_runs))
        .route("/runs/{id}", patch(update_job_run))
        .route("/runs/{id}/advance", post(advance_job_run))
}

pub fn task_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(create_task))
        .route("/", get(list_tasks))
        .route("/{id}", patch(update_task))
}

// --- Job definitions ---

#[derive(Deserialize)]
struct CreateJobDefinitionRequest {
    name: String,
    description: Option<String>,
    config: Option<Value>,
}

async fn create_job_definition(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateJobDefinitionRequest>,
) -> Result<Json<Value>> {
    let description = body.description.unwrap_or_default();
    let config = body.config.unwrap_or(serde_json::json!({}));
    let result = state
        .jobs
        .create_job_definition(&body.name, &description, config)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn list_job_definitions(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    let results = state.jobs.list_job_definitions().await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn get_job_definition(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .get_job_definition(&id)
        .await?
        .ok_or_else(|| crate::error::OverseerError::NotFound(format!("job_definition {id}")))?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

// --- Job runs ---

#[derive(Deserialize)]
struct StartJobRunRequest {
    definition_id: String,
    triggered_by: String,
    parent_id: Option<String>,
    config_overrides: Option<Value>,
}

async fn start_job_run(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StartJobRunRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .start_job_run(
            &body.definition_id,
            &body.triggered_by,
            body.parent_id.as_deref(),
            body.config_overrides,
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct ListJobRunsQuery {
    status: Option<String>,
}

async fn list_job_runs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListJobRunsQuery>,
) -> Result<Json<Value>> {
    let results = state.jobs.list_job_runs(params.status.as_deref()).await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct UpdateJobRunRequest {
    status: Option<String>,
    result: Option<Value>,
    error: Option<String>,
}

async fn advance_job_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let new_run = state.pipeline.advance(&id).await?;

    // Try to assign to an available hatchery
    let hatcheries = state.hatchery.list(Some("online")).await?;
    if let Some(hatchery) = hatcheries
        .iter()
        .find(|h| h.active_drones < h.max_concurrency)
    {
        let _ = state.hatchery.assign_job(&new_run.id, &hatchery.id).await;
        tracing::info!(
            run_id = %new_run.id,
            hatchery_id = %hatchery.id,
            "auto-assigned advanced run to hatchery"
        );
    }

    Ok(Json(serde_json::to_value(new_run).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn update_job_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateJobRunRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .update_job_run(
            &id,
            body.status.as_deref(),
            body.result,
            body.error.as_deref(),
        )
        .await?;

    // Check if pipeline should auto-advance
    if let Ok(Some(next_run)) = state
        .jobs
        .check_pipeline_after_completion(&result, &state.pipeline)
        .await
    {
        let hatcheries = state
            .hatchery
            .list(Some("online"))
            .await
            .unwrap_or_default();
        if let Some(hatchery) = hatcheries
            .iter()
            .find(|h| h.active_drones < h.max_concurrency)
        {
            let _ = state.hatchery.assign_job(&next_run.id, &hatchery.id).await;
            tracing::info!(
                next_run_id = %next_run.id,
                hatchery_id = %hatchery.id,
                "auto-assigned pipeline run to hatchery"
            );
        }
    }

    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

// --- Tasks ---

#[derive(Deserialize)]
struct CreateTaskRequest {
    subject: String,
    run_id: Option<String>,
    assigned_to: Option<String>,
}

async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateTaskRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .create_task(
            &body.subject,
            body.run_id.as_deref(),
            body.assigned_to.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct ListTasksQuery {
    status: Option<String>,
    assigned_to: Option<String>,
    run_id: Option<String>,
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTasksQuery>,
) -> Result<Json<Value>> {
    let results = state
        .jobs
        .list_tasks(
            params.status.as_deref(),
            params.assigned_to.as_deref(),
            params.run_id.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct UpdateTaskRequest {
    status: Option<String>,
    assigned_to: Option<String>,
    output: Option<Value>,
}

async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<Value>> {
    let result = state
        .jobs
        .update_task(
            &id,
            body.status.as_deref(),
            body.assigned_to.as_deref(),
            body.output,
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
