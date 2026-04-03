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
        .route("/", post(create_credential))
        .route("/", get(list_credentials))
        .route("/match", get(match_credentials))
        .route("/{id}", get(get_credential))
        .route("/{id}", delete(delete_credential))
}

#[derive(Deserialize)]
struct CreateCredentialRequest {
    pattern: String,
    credential_type: String,
    secret: String,
}

async fn create_credential(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateCredentialRequest>,
) -> Result<Json<Value>> {
    let cred = state
        .credentials
        .create_credential(&body.pattern, &body.credential_type, &body.secret)
        .await?;
    Ok(Json(redacted_credential(&cred)))
}

async fn list_credentials(State(state): State<Arc<AppState>>) -> Result<Json<Value>> {
    let creds = state.credentials.list_credentials().await?;
    let redacted: Vec<Value> = creds.iter().map(redacted_credential).collect();
    Ok(Json(serde_json::to_value(redacted).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

async fn get_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let cred = state
        .credentials
        .get_credential(&id)
        .await?
        .ok_or_else(|| crate::error::OverseerError::NotFound(format!("credential {id}")))?;
    Ok(Json(redacted_credential(&cred)))
}

async fn delete_credential(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    state.credentials.delete_credential(&id).await?;
    Ok(Json(serde_json::json!({"deleted": true})))
}

#[derive(Deserialize)]
struct MatchQuery {
    repo_url: String,
}

async fn match_credentials(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MatchQuery>,
) -> Result<Json<Value>> {
    let matches = state
        .credentials
        .match_credentials(&params.repo_url)
        .await?;
    // SECURITY: Returns full secrets — intended for internal Queen consumption only.
    // This endpoint must not be exposed to untrusted clients. Requires auth before
    // any non-localhost deployment.
    let result: Vec<Value> = matches
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id,
                "pattern": c.pattern,
                "credential_type": c.credential_type,
                "secret": c.secret,
            })
        })
        .collect();
    Ok(Json(serde_json::to_value(result).map_err(|e| {
        crate::error::OverseerError::Internal(e.to_string())
    })?))
}

fn redacted_credential(cred: &crate::db::models::Credential) -> Value {
    serde_json::json!({
        "id": cred.id,
        "pattern": cred.pattern,
        "credential_type": cred.credential_type,
        "created_at": cred.created_at,
        "updated_at": cred.updated_at,
    })
}
