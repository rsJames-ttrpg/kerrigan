use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(store_memory))
        .route("/search", get(search_memories))
        .route("/{id}", delete(delete_memory))
}

#[derive(Deserialize)]
struct StoreMemoryRequest {
    content: String,
    source: String,
    tags: Option<Vec<String>>,
    expires_at: Option<String>,
}

async fn store_memory(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreMemoryRequest>,
) -> Result<Json<Value>> {
    let tags = body.tags.unwrap_or_default();
    let result = state
        .memory
        .store(
            &body.content,
            &body.source,
            &tags,
            body.expires_at.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::CortexError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct SearchMemoriesQuery {
    q: String,
    tags: Option<String>,
    limit: Option<usize>,
}

async fn search_memories(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchMemoriesQuery>,
) -> Result<Json<Value>> {
    let limit = params.limit.unwrap_or(10);
    let tags: Option<Vec<String>> = params
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
    let results = state
        .memory
        .recall(&params.q, tags.as_deref(), limit)
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::CortexError::Internal(e.to_string())
    })?))
}

async fn delete_memory(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    state.memory.delete(&id).await?;
    Ok(Json(serde_json::json!({ "deleted": id })))
}
