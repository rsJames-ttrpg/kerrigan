use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum OverseerError {
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

    #[error("object store error: {0}")]
    ObjectStore(String),
}

impl From<object_store::Error> for OverseerError {
    fn from(e: object_store::Error) -> Self {
        OverseerError::ObjectStore(e.to_string())
    }
}

impl IntoResponse for OverseerError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            OverseerError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            OverseerError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            OverseerError::Storage(e) => {
                tracing::error!(error = %e, "storage error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal storage error".to_string(),
                )
            }
            OverseerError::Embedding(msg) => {
                tracing::error!(error = %msg, "embedding error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            OverseerError::Io(e) => {
                tracing::error!(error = %e, "io error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            OverseerError::ObjectStore(msg) => {
                tracing::error!(error = %msg, "object store error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
            OverseerError::Internal(msg) => {
                tracing::error!(error = %msg, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        let body = axum::Json(json!({ "error": message }));
        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, OverseerError>;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn status_and_body(err: OverseerError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn test_not_found_is_404() {
        let (status, body) = status_and_body(OverseerError::NotFound("thing".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "thing");
    }

    #[tokio::test]
    async fn test_validation_is_400() {
        let (status, body) = status_and_body(OverseerError::Validation("bad input".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "bad input");
    }

    #[tokio::test]
    async fn test_internal_is_500() {
        let (status, body) = status_and_body(OverseerError::Internal("boom".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal error");
    }

    #[tokio::test]
    async fn test_embedding_is_500() {
        let (status, body) = status_and_body(OverseerError::Embedding("fail".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal error");
    }

    #[tokio::test]
    async fn test_io_is_500() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let (status, body) = status_and_body(OverseerError::Io(io_err)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal error");
    }

    #[tokio::test]
    async fn test_object_store_is_500() {
        let (status, body) =
            status_and_body(OverseerError::ObjectStore("store failed".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal error");
    }

    #[tokio::test]
    async fn test_storage_is_500() {
        let db_err = sqlx::Error::RowNotFound;
        let (status, body) = status_and_body(OverseerError::Storage(db_err)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal storage error");
    }
}
