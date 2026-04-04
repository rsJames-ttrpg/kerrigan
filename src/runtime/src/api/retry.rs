use std::time::Duration;

use async_trait::async_trait;

use super::{ApiClient, ApiError, ApiRequest, EventStream};

pub struct RetryingClient {
    inner: Box<dyn ApiClient>,
    max_retries: u32,
}

impl RetryingClient {
    pub fn new(inner: Box<dyn ApiClient>, max_retries: u32) -> Self {
        Self { inner, max_retries }
    }
}

#[async_trait]
impl ApiClient for RetryingClient {
    async fn stream(&self, request: ApiRequest) -> Result<EventStream, ApiError> {
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            match self.inner.stream(request.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(ApiError::RateLimit { retry_after }) => {
                    let delay = retry_after.unwrap_or(Duration::from_secs(2u64.pow(attempt)));
                    tracing::warn!(attempt, ?delay, "rate limited, retrying");
                    tokio::time::sleep(delay).await;
                    last_error = Some(ApiError::RateLimit { retry_after });
                }
                Err(ApiError::NetworkError(msg)) if attempt < self.max_retries => {
                    tracing::warn!(attempt, %msg, "network error, retrying");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    last_error = Some(ApiError::NetworkError(msg));
                }
                Err(ApiError::ServerError { status, body })
                    if status >= 500 && attempt < self.max_retries =>
                {
                    tracing::warn!(attempt, status, "server error, retrying");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    last_error = Some(ApiError::ServerError { status, body });
                }
                Err(ApiError::StreamInterrupted) if attempt < self.max_retries => {
                    tracing::warn!(attempt, "stream interrupted, retrying full request");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    last_error = Some(ApiError::StreamInterrupted);
                }
                Err(e) => return Err(e), // Auth, model errors — fail immediately
            }
        }
        Err(last_error.unwrap())
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    fn supports_tool_use(&self) -> bool {
        self.inner.supports_tool_use()
    }

    fn max_tokens(&self) -> u32 {
        self.inner.max_tokens()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A mock client that returns errors for the first N calls, then succeeds.
    struct MockClient {
        errors: Vec<ApiError>,
        call_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl ApiClient for MockClient {
        async fn stream(&self, _request: ApiRequest) -> Result<EventStream, ApiError> {
            let count = self.call_count.fetch_add(1, Ordering::SeqCst) as usize;
            if count < self.errors.len() {
                // Return the error for this attempt
                let error = match &self.errors[count] {
                    ApiError::RateLimit { retry_after } => ApiError::RateLimit {
                        retry_after: *retry_after,
                    },
                    ApiError::NetworkError(msg) => ApiError::NetworkError(msg.clone()),
                    ApiError::ServerError { status, body } => ApiError::ServerError {
                        status: *status,
                        body: body.clone(),
                    },
                    ApiError::StreamInterrupted => ApiError::StreamInterrupted,
                    ApiError::AuthFailed => ApiError::AuthFailed,
                    ApiError::ModelNotFound => ApiError::ModelNotFound,
                    ApiError::ContextTooLong { max, requested } => ApiError::ContextTooLong {
                        max: *max,
                        requested: *requested,
                    },
                };
                Err(error)
            } else {
                // Return an empty stream
                Ok(Box::pin(tokio_stream::empty()))
            }
        }

        fn model(&self) -> &str {
            "test-model"
        }

        fn supports_tool_use(&self) -> bool {
            true
        }

        fn max_tokens(&self) -> u32 {
            4096
        }
    }

    fn test_request() -> ApiRequest {
        ApiRequest {
            system: vec![],
            messages: vec![],
            tools: vec![],
            max_tokens: 1024,
            temperature: None,
        }
    }

    #[tokio::test]
    async fn test_auth_error_not_retried() {
        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![ApiError::AuthFailed],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 3);
        let result = client.stream(test_request()).await;

        assert!(matches!(result, Err(ApiError::AuthFailed)));
        assert_eq!(call_count.load(Ordering::SeqCst), 1); // Only 1 attempt
    }

    #[tokio::test]
    async fn test_model_not_found_not_retried() {
        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![ApiError::ModelNotFound],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 3);
        let result = client.stream(test_request()).await;

        assert!(matches!(result, Err(ApiError::ModelNotFound)));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_network_error_retried_then_succeeds() {
        tokio::time::pause(); // Use virtual time to avoid real sleeps

        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![ApiError::NetworkError("timeout".to_string())],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 3);

        // Spawn the stream call so we can advance time
        let handle = tokio::spawn(async move { client.stream(test_request()).await });

        // Advance past the 5s sleep
        tokio::time::advance(Duration::from_secs(6)).await;

        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 2); // 1 failure + 1 success
    }

    #[tokio::test]
    async fn test_max_retries_respected() {
        tokio::time::pause();

        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![
                ApiError::NetworkError("err1".to_string()),
                ApiError::NetworkError("err2".to_string()),
                ApiError::NetworkError("err3".to_string()),
                ApiError::NetworkError("err4".to_string()), // max_retries=2, so 3 total attempts
            ],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 2);
        let handle = tokio::spawn(async move { client.stream(test_request()).await });

        // Advance time enough for all retries
        for _ in 0..3 {
            tokio::time::advance(Duration::from_secs(6)).await;
        }

        let result = handle.await.unwrap();
        assert!(matches!(result, Err(ApiError::NetworkError(_))));
        assert_eq!(call_count.load(Ordering::SeqCst), 3); // initial + 2 retries
    }

    #[tokio::test]
    async fn test_rate_limit_retried_with_provided_delay() {
        tokio::time::pause();

        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![ApiError::RateLimit {
                retry_after: Some(Duration::from_secs(10)),
            }],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 3);
        let handle = tokio::spawn(async move { client.stream(test_request()).await });

        // Advance past the 10s retry-after
        tokio::time::advance(Duration::from_secs(11)).await;

        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_server_error_retried() {
        tokio::time::pause();

        let call_count = Arc::new(AtomicU32::new(0));
        let mock = MockClient {
            errors: vec![ApiError::ServerError {
                status: 500,
                body: "internal error".to_string(),
            }],
            call_count: call_count.clone(),
        };

        let client = RetryingClient::new(Box::new(mock), 3);
        let handle = tokio::spawn(async move { client.stream(test_request()).await });

        // Advance past the 10s sleep
        tokio::time::advance(Duration::from_secs(11)).await;

        let result = handle.await.unwrap();
        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }
}
