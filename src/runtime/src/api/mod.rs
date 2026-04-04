pub mod anthropic;
pub mod error;
pub mod openai_compat;
pub mod retry;
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

/// Create an ApiClient from a ProviderConfig.
pub fn create_client(config: &ProviderConfig) -> Box<dyn ApiClient> {
    match config {
        ProviderConfig::Anthropic {
            api_key,
            model,
            base_url,
        } => Box::new(anthropic::AnthropicClient::new(
            api_key.clone(),
            model.clone(),
            base_url.clone(),
        )),
        ProviderConfig::OpenAiCompat {
            base_url,
            api_key,
            model,
        } => Box::new(openai_compat::OpenAiCompatClient::new(
            base_url.clone(),
            api_key.clone(),
            model.clone(),
        )),
    }
}

pub struct DefaultApiClientFactory {
    config: ProviderConfig,
}

impl DefaultApiClientFactory {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

impl ApiClientFactory for DefaultApiClientFactory {
    fn create(&self) -> Box<dyn ApiClient> {
        create_client(&self.config)
    }
}
