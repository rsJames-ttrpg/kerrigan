use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(register_hatchery))
        .route("/", get(list_hatcheries))
        .route("/{id}", get(get_hatchery))
        .route("/{id}/heartbeat", post(heartbeat_hatchery))
        .route("/{id}", delete(deregister_hatchery))
        .route("/{id}/jobs", get(list_hatchery_jobs))
        .route("/{id}/jobs/{job_run_id}", post(assign_job))
}

// ── Request / response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct RegisterHatcheryRequest {
    name: String,
    capabilities: Option<Value>,
    max_concurrency: Option<i32>,
}

#[derive(Deserialize)]
struct HeartbeatRequest {
    status: String,
    active_drones: i32,
}

#[derive(Deserialize)]
struct ListHatcheriesQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
struct ListHatcheryJobsQuery {
    status: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

async fn register_hatchery(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterHatcheryRequest>,
) -> Result<Json<Value>> {
    let capabilities = body.capabilities.unwrap_or(serde_json::json!({}));
    let max_concurrency = body.max_concurrency.unwrap_or(1);
    let result = state
        .hatchery
        .register(&body.name, capabilities, max_concurrency)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn list_hatcheries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListHatcheriesQuery>,
) -> Result<Json<Value>> {
    let results = state.hatchery.list(params.status.as_deref()).await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn get_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let result = state.hatchery.get(&id).await?;
    match result {
        Some(h) => {
            Ok(Json(serde_json::to_value(h).map_err(|e| {
                crate::error::OverseerError::Internal(e.to_string())
            })?))
        }
        None => Err(crate::error::OverseerError::NotFound(format!(
            "hatchery {id} not found"
        ))),
    }
}

async fn heartbeat_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatRequest>,
) -> Result<Json<Value>> {
    let result = state
        .hatchery
        .heartbeat(&id, &body.status, body.active_drones)
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn deregister_hatchery(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    state.hatchery.deregister(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_hatchery_jobs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<ListHatcheryJobsQuery>,
) -> Result<Json<Value>> {
    let results = state
        .hatchery
        .list_job_runs(&id, params.status.as_deref())
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn assign_job(
    State(state): State<Arc<AppState>>,
    Path((id, job_run_id)): Path<(String, String)>,
) -> Result<Json<Value>> {
    let result = state.hatchery.assign_job(&job_run_id, &id).await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
