use axum::{
    Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::error::Result;
use crate::services::AppState;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", post(log_decision))
        .route("/", get(query_decisions))
}

#[derive(Deserialize)]
struct LogDecisionRequest {
    agent: String,
    context: String,
    decision: String,
    reasoning: Option<String>,
    tags: Option<Vec<String>>,
    run_id: Option<String>,
}

async fn log_decision(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LogDecisionRequest>,
) -> Result<Json<Value>> {
    let tags = body.tags.unwrap_or_default();
    let reasoning = body.reasoning.unwrap_or_default();
    let result = state
        .decisions
        .log(
            &body.agent,
            &body.context,
            &body.decision,
            &reasoning,
            &tags,
            body.run_id.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

#[derive(Deserialize)]
struct QueryDecisionsQuery {
    agent: Option<String>,
    tags: Option<String>,
    limit: Option<i64>,
}

async fn query_decisions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryDecisionsQuery>,
) -> Result<Json<Value>> {
    let limit = params.limit.unwrap_or(20);
    let tags: Option<Vec<String>> = params
        .tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
    let results = state
        .decisions
        .query(params.agent.as_deref(), tags.as_deref(), limit)
        .await?;
    Ok(Json(serde_json::to_value(results).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}
