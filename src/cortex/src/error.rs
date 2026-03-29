use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum CortexError {
    #[error("storage error: {0}")]
    Storage(#[from] sqlx::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for CortexError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            CortexError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            CortexError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            CortexError::Storage(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            CortexError::Embedding(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            CortexError::Io(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            CortexError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };
        let body = axum::Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, CortexError>;
