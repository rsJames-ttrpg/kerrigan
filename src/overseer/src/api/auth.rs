use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;

use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{job_run_id}/auth", post(submit_auth_code))
        .route("/{job_run_id}/auth", get(poll_auth_code))
}

#[derive(Deserialize)]
struct SubmitAuthCodeRequest {
    code: String,
}

/// POST /api/jobs/runs/{job_run_id}/auth — user submits the OAuth code
async fn submit_auth_code(
    State(state): State<Arc<AppState>>,
    Path(job_run_id): Path<String>,
    Json(body): Json<SubmitAuthCodeRequest>,
) -> StatusCode {
    state.auth.submit_code(&job_run_id, &body.code);
    tracing::info!(job_run_id = %job_run_id, "auth code submitted");
    StatusCode::NO_CONTENT
}

/// GET /api/jobs/runs/{job_run_id}/auth — Queen polls for the code
async fn poll_auth_code(
    State(state): State<Arc<AppState>>,
    Path(job_run_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.auth.take_code(&job_run_id) {
        Some(code) => (StatusCode::OK, Json(serde_json::json!({ "code": code }))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "status": "pending" })),
        ),
    }
}
