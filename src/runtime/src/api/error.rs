use std::time::Duration;

/// Parse an HTTP error response into an ApiError.
/// Shared by Anthropic and OpenAI-compatible providers.
pub(crate) fn parse_error_response(status: u16, body: &str) -> ApiError {
    match status {
        429 => {
            let retry_after = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("error")?.get("retry_after")?.as_f64())
                .map(|secs| Duration::from_secs_f64(secs));
            ApiError::RateLimit { retry_after }
        }
        401 => ApiError::AuthFailed,
        404 => ApiError::ModelNotFound,
        status if status >= 500 => ApiError::ServerError {
            status,
            body: body.to_string(),
        },
        _ => ApiError::ServerError {
            status,
            body: body.to_string(),
        },
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("rate limited (retry after {retry_after:?})")]
    RateLimit { retry_after: Option<Duration> },

    #[error("authentication failed")]
    AuthFailed,

    #[error("model not found")]
    ModelNotFound,

    #[error("context too long (max {max}, requested {requested})")]
    ContextTooLong { max: u32, requested: u32 },

    #[error("server error ({status}): {body}")]
    ServerError { status: u16, body: String },

    #[error("network error: {0}")]
    NetworkError(String),

    #[error("stream interrupted")]
    StreamInterrupted,
}
