# Plan 01: Runtime API Client

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the multi-provider LLM API client that abstracts over Anthropic Messages API and OpenAI-compatible endpoints (Ollama, OpenRouter) behind a single `ApiClient` trait with streaming support.

**Architecture:** Provider-agnostic types (`ApiRequest`, `StreamEvent`) with per-provider translation modules. Each provider handles its own SSE parsing and maps to shared types. The runtime never sees provider-specific types.

**Tech Stack:** reqwest (HTTP), tokio (async), serde (serialization), eventsource-stream or manual SSE parsing

**Spec:** `docs/specs/native-drone/01-runtime-api-client.md`

**Reference:** `rust/crates/api/src/` in Claw Code repo

---

### Task 1: Core API types and trait

**Files:**
- Create: `src/runtime/src/api/types.rs`
- Create: `src/runtime/src/api/error.rs`
- Modify: `src/runtime/src/api/mod.rs`

- [ ] **Step 1: Write tests for API types serialization**

Create `src/runtime/src/api/types.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

#[derive(Debug)]
pub enum StreamEvent {
    TextDelta(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Usage(TokenUsage),
    MessageStop,
    Error(ApiError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_block_text_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_content_block_tool_use_roundtrip() {
        let block = ContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "read_file".into(),
            input: serde_json::json!({"path": "/tmp/test.rs"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tool_1");
                assert_eq!(name, "read_file");
                assert_eq!(input["path"], "/tmp/test.rs");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_api_request_serialization() {
        let request = ApiRequest {
            system: vec![SystemBlock {
                text: "You are helpful.".into(),
            }],
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: 1024,
            temperature: Some(0.0),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["max_tokens"], 1024);
        assert_eq!(json["messages"][0]["role"], "user");
    }
}
```

- [ ] **Step 2: Create error types**

Create `src/runtime/src/api/error.rs`:

```rust
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
```

Add `thiserror` to `src/runtime/Cargo.toml`:
```toml
thiserror = "2"
```

- [ ] **Step 3: Create mod.rs with ApiClient trait**

Update `src/runtime/src/api/mod.rs`:

```rust
pub mod error;
pub mod types;

use std::pin::Pin;

use async_trait::async_trait;
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
```

Add `tokio-stream` to `src/runtime/Cargo.toml`:
```toml
tokio-stream = "0.1"
```

- [ ] **Step 4: Run tests, verify compilation**

Run: `cd src/runtime && cargo test`
Expected: all tests pass

- [ ] **Step 5: Run buckify and verify Buck2 build**

Run: `./tools/buckify.sh`
Run: `buck2 build root//src/runtime:runtime`
Expected: builds successfully

- [ ] **Step 6: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "add API types, error types, and ApiClient trait to runtime"
```

---

### Task 2: SSE parser

**Files:**
- Create: `src/runtime/src/api/sse.rs`
- Modify: `src/runtime/src/api/mod.rs`

- [ ] **Step 1: Write tests for SSE frame parsing**

Create `src/runtime/src/api/sse.rs`:

```rust
/// Parses Server-Sent Events from a byte stream.
/// SSE format: lines of `field: value`, events separated by blank lines.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Feed bytes into the parser, returning any complete events.
    pub fn feed(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();
        while let Some(event) = self.try_parse_event() {
            events.push(event);
        }
        events
    }

    fn try_parse_event(&mut self) -> Option<SseEvent> {
        // Look for double newline (event boundary)
        let boundary = self.buffer.find("\n\n")?;
        let raw = self.buffer[..boundary].to_string();
        self.buffer = self.buffer[boundary + 2..].to_string();

        let mut event_type = None;
        let mut data_lines = Vec::new();

        for line in raw.lines() {
            if let Some(value) = line.strip_prefix("event: ") {
                event_type = Some(value.to_string());
            } else if let Some(value) = line.strip_prefix("data: ") {
                data_lines.push(value);
            } else if line.starts_with("data:") {
                // "data:" with no space — empty data line
                data_lines.push(&line[5..]);
            }
            // Ignore other fields (id:, retry:, comments)
        }

        if data_lines.is_empty() && event_type.is_none() {
            return None;
        }

        Some(SseEvent {
            event_type,
            data: data_lines.join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_event() {
        let mut parser = SseParser::new();
        let events = parser.feed("event: message\ndata: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("message"));
        assert_eq!(events[0].data, "hello");
    }

    #[test]
    fn test_multi_line_data() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn test_chunked_delivery() {
        let mut parser = SseParser::new();
        assert!(parser.feed("event: msg\n").is_empty());
        assert!(parser.feed("data: part").is_empty());
        let events = parser.feed("ial\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "partial");
    }

    #[test]
    fn test_multiple_events() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: first\n\ndata: second\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "first");
        assert_eq!(events[1].data, "second");
    }

    #[test]
    fn test_event_without_type() {
        let mut parser = SseParser::new();
        let events = parser.feed("data: just data\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, None);
        assert_eq!(events[0].data, "just data");
    }
}
```

- [ ] **Step 2: Add to mod.rs**

Add `pub mod sse;` to `src/runtime/src/api/mod.rs`.

- [ ] **Step 3: Run tests**

Run: `cd src/runtime && cargo test api::sse`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/runtime/
git commit -m "add SSE frame parser for streaming API responses"
```

---

### Task 3: Anthropic provider

**Files:**
- Create: `src/runtime/src/api/anthropic.rs`
- Modify: `src/runtime/src/api/mod.rs`
- Modify: `src/runtime/Cargo.toml`

- [ ] **Step 1: Write tests for Anthropic SSE event parsing**

The Anthropic API sends SSE events like:
```
event: message_start
data: {"type":"message_start","message":{...}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"/tmp\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}

event: message_stop
data: {"type":"message_stop"}
```

Create `src/runtime/src/api/anthropic.rs` with the event types and parsing, plus an `AnthropicClient` struct implementing `ApiClient`. Include tests that parse sample SSE events into `StreamEvent` variants.

The key implementation details:
- Tool use inputs arrive incrementally as `input_json_delta` — accumulate partial JSON per content block index, parse on `content_block_stop`
- Usage arrives in `message_start` (input tokens) and `message_delta` (output tokens) — merge them
- Map `message_stop` → `StreamEvent::MessageStop`
- HTTP request: POST to `{base_url}/v1/messages` with `x-api-key` header, `anthropic-version: 2023-06-01`, `"stream": true`
- Handle error responses (429, 401, 404, 5xx) and map to `ApiError`

Include these tests (at minimum):
```rust
#[test]
fn test_parse_text_delta() { ... }

#[test]
fn test_parse_tool_use_accumulated() { ... }

#[test]
fn test_parse_usage_merged() { ... }

#[test]
fn test_error_response_rate_limit() { ... }

#[test]
fn test_error_response_auth() { ... }
```

- [ ] **Step 2: Run tests**

Run: `cd src/runtime && cargo test api::anthropic`
Expected: all parsing tests pass (no network calls in unit tests)

- [ ] **Step 3: Add reqwest dependency**

In `src/runtime/Cargo.toml`:
```toml
reqwest = { version = "0.12", features = ["stream"] }
```

Run: `./tools/buckify.sh`

- [ ] **Step 4: Implement AnthropicClient::stream**

The `stream` method:
1. Build HTTP request body from `ApiRequest` (translate to Anthropic format)
2. Send POST with streaming enabled
3. Return an `EventStream` that parses SSE frames and yields `StreamEvent`s

Use `reqwest::Response::bytes_stream()` piped through `SseParser`.

- [ ] **Step 5: Verify build**

Run: `cd src/runtime && cargo check`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 6: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "add Anthropic API provider with SSE streaming"
```

---

### Task 4: OpenAI-compatible provider

**Files:**
- Create: `src/runtime/src/api/openai_compat.rs`
- Modify: `src/runtime/src/api/mod.rs`

- [ ] **Step 1: Write tests for OpenAI chat completion chunk parsing**

OpenAI-compatible APIs (Ollama, OpenRouter) send:
```
data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"/tmp\"}"}}]},"finish_reason":null}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5}}

data: [DONE]
```

Create `src/runtime/src/api/openai_compat.rs` with `OpenAiCompatClient` implementing `ApiClient`. Key differences from Anthropic:
- Tool calls use `tool_calls` array in delta with incremental `arguments` strings
- Usage may come in final chunk or in a separate `usage` chunk (provider-dependent)
- `[DONE]` sentinel instead of `message_stop` event type
- Request format: `messages` array (not `content` blocks), `tools` array with `function` wrapper
- Auth: `Authorization: Bearer {key}` (optional for Ollama)
- System message is a `{"role": "system", "content": "..."}` message, not a separate field

Include tests:
```rust
#[test]
fn test_parse_text_chunk() { ... }

#[test]
fn test_parse_tool_call_incremental() { ... }

#[test]
fn test_parse_done_sentinel() { ... }

#[test]
fn test_translate_request_format() { ... }
```

- [ ] **Step 2: Run tests**

Run: `cd src/runtime && cargo test api::openai_compat`
Expected: all parsing tests pass

- [ ] **Step 3: Implement OpenAiCompatClient::stream**

Same pattern as Anthropic: translate request, POST with streaming, parse SSE, yield `StreamEvent`s. Handle the format differences (tool_calls accumulation, [DONE] sentinel).

- [ ] **Step 4: Verify build**

Run: `cd src/runtime && cargo check`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 5: Commit**

```bash
git add src/runtime/
git commit -m "add OpenAI-compatible API provider (Ollama, OpenRouter)"
```

---

### Task 5: Provider factory and retry logic

**Files:**
- Create: `src/runtime/src/api/retry.rs`
- Modify: `src/runtime/src/api/mod.rs`

- [ ] **Step 1: Implement provider factory**

In `src/runtime/src/api/mod.rs`, add:

```rust
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
```

- [ ] **Step 2: Write retry wrapper with tests**

Create `src/runtime/src/api/retry.rs`:

```rust
use std::time::Duration;
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

#[async_trait::async_trait]
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
                Err(ApiError::ServerError { status, body }) if status >= 500 && attempt < self.max_retries => {
                    tracing::warn!(attempt, status, "server error, retrying");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    last_error = Some(ApiError::ServerError { status, body });
                }
                Err(e) => return Err(e), // Auth, model errors — fail immediately
            }
        }
        Err(last_error.unwrap())
    }

    fn model(&self) -> &str { self.inner.model() }
    fn supports_tool_use(&self) -> bool { self.inner.supports_tool_use() }
    fn max_tokens(&self) -> u32 { self.inner.max_tokens() }
}

#[cfg(test)]
mod tests {
    // Test that non-retryable errors propagate immediately
    // Test that rate limits retry with backoff
    // Test that max retries is respected
}
```

- [ ] **Step 3: Run tests**

Run: `cd src/runtime && cargo test`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add src/runtime/
git commit -m "add provider factory and retry logic for API client"
```
