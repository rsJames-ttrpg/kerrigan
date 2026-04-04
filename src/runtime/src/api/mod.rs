pub mod error;
pub mod sse;
pub mod types;

use std::pin::Pin;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;

pub use error::ApiError;
pub use types::*;

pub type EventStream = Pin<Box<dyn Stream<Item = StreamEvent> + Send>>;

#[async_trait]
pub trait ApiClient: Send + Sync {
    async fn stream(&self, request: ApiRequest) -> Result<EventStream, ApiError>;
    fn model(&self) -> &str;
    fn supports_tool_use(&self) -> bool;
    fn max_tokens(&self) -> u32;
}

/// Factory for creating ApiClient instances (used for sub-agent spawning)
pub trait ApiClientFactory: Send + Sync {
    fn create(&self) -> Box<dyn ApiClient>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProviderConfig {
    Anthropic {
        api_key: String,
        model: String,
        base_url: Option<String>,
    },
    OpenAiCompat {
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
}
