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

use crate::db::models::ArtifactType;
use crate::error::{OverseerError, Result};
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
    artifact_type: Option<String>,
}

async fn store_artifact(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreArtifactRequest>,
) -> Result<Json<Value>> {
    let data = base64::engine::general_purpose::STANDARD
        .decode(&body.data)
        .map_err(|e| OverseerError::Validation(format!("invalid base64: {e}")))?;
    let result = state
        .artifacts
        .store(
            &body.name,
            &body.content_type,
            &data,
            body.run_id.as_deref(),
            body.artifact_type
                .as_deref()
                .map(|s| s.parse::<ArtifactType>())
                .transpose()
                .map_err(OverseerError::Validation)?,
        )
        .await?;
    Ok(Json(
        serde_json::to_value(result).map_err(|e| OverseerError::Internal(e.to_string()))?,
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
    artifact_type: Option<String>,
    since: Option<String>,
}

async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListArtifactsQuery>,
) -> Result<Json<Value>> {
    let since = params
        .since
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| OverseerError::Validation(format!("invalid since timestamp: {e}")))
        })
        .transpose()?;
    let artifact_type = params
        .artifact_type
        .as_deref()
        .map(|s| s.parse::<ArtifactType>())
        .transpose()
        .map_err(OverseerError::Validation)?;
    let filter = crate::db::ArtifactFilter {
        run_id: params.run_id,
        artifact_type,
        since,
    };
    let results = state.artifacts.list(&filter).await?;
    Ok(Json(
        serde_json::to_value(results).map_err(|e| OverseerError::Internal(e.to_string()))?,
    ))
}
