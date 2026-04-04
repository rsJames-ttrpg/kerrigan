use std::time::Duration;

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
