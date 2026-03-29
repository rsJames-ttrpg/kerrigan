use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::header,
    routing::{get, post},
};
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::{CortexError, Result};
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(store_artifact))
        .route("/", get(list_artifacts))
        .route("/{id}", get(get_artifact))
}

#[derive(Deserialize)]
struct StoreArtifactRequest {
    name: String,
    content_type: String,
    data: String, // base64-encoded
    run_id: Option<String>,
}

async fn store_artifact(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreArtifactRequest>,
) -> Result<Json<Value>> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(&body.data)
        .map_err(|e| CortexError::Validation(format!("invalid base64: {e}")))?;
    let result = state
        .artifacts
        .store(
            &body.name,
            &body.content_type,
            &data,
            body.run_id.as_deref(),
        )
        .await?;
    Ok(Json(
        serde_json::to_value(result).map_err(|e| CortexError::Internal(e.to_string()))?,
    ))
}

async fn get_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<([(header::HeaderName, String); 1], Bytes)> {
    let (meta, data) = state.artifacts.get(&id).await?;
    Ok((
        [(header::CONTENT_TYPE, meta.content_type)],
        Bytes::from(data),
    ))
}

#[derive(Deserialize)]
struct ListArtifactsQuery {
    run_id: Option<String>,
}

async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListArtifactsQuery>,
) -> Result<Json<Value>> {
    let results = state.artifacts.list(params.run_id.as_deref()).await?;
    Ok(Json(
        serde_json::to_value(results).map_err(|e| CortexError::Internal(e.to_string()))?,
    ))
}
