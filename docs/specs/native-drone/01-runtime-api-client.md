# Runtime: API Client

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

Multi-provider LLM API client with streaming support. Abstracts over Anthropic Messages API and OpenAI-compatible endpoints (Ollama, OpenRouter, etc.) behind a single trait.

## Core Trait

```rust
#[trait_variant::make(Send)]
pub trait ApiClient {
    async fn stream(&self, request: ApiRequest) -> Result<EventStream, ApiError>;
    fn model(&self) -> &str;
    fn supports_tool_use(&self) -> bool;
    fn max_tokens(&self) -> u32;
}
```

The runtime is generic over `ApiClient`. Provider selection happens at construction time, not per-request.

## Request/Response Types

```rust
pub struct ApiRequest {
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

pub enum StreamEvent {
    TextDelta(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    Usage(TokenUsage),
    MessageStop,
    Error(ApiError),
}

pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}
```

`EventStream` is a `Pin<Box<dyn Stream<Item = StreamEvent> + Send>>`. The conversation loop consumes events as they arrive for real-time forwarding to the event sink.

## Providers

### Anthropic

- Endpoint: `https://api.anthropic.com/v1/messages`
- Auth: `x-api-key` header
- Streaming: SSE with `message_start`, `content_block_delta`, `message_stop` events
- Tool use: native `tool_use` content blocks
- Translates Anthropic SSE events → `StreamEvent` enum

### OpenAI-Compatible

- Endpoint: configurable `base_url` (e.g., `http://localhost:11434/v1` for Ollama)
- Auth: `Bearer` token, optional (Ollama needs none)
- Streaming: SSE with `chat.completion.chunk` events
- Tool use: `tool_calls` array in response delta
- Translates OpenAI chat completion chunks → `StreamEvent` enum
- Handles provider quirks: some return tool call arguments incrementally (streamed JSON), some return them complete

### Translation Layer

Each provider has a private module that handles:
- Building the provider-specific HTTP request body
- Parsing SSE frames from the response stream
- Mapping provider-specific types to `StreamEvent`

The runtime never sees provider-specific types. Message format translation (Anthropic's `content` blocks vs OpenAI's `messages` array) happens inside the client.

## Configuration

```rust
pub enum ProviderConfig {
    Anthropic {
        api_key: String,
        model: String,
        base_url: Option<String>,   // override for proxies
    },
    OpenAiCompat {
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
}
```

Provider is selected explicitly via config — no env var auto-detection. The drone resolves this from `drone.toml` + job spec overrides.

## Error Handling

```rust
pub enum ApiError {
    RateLimit { retry_after: Option<Duration> },
    AuthFailed,
    ModelNotFound,
    ContextTooLong { max: u32, requested: u32 },
    ServerError { status: u16, body: String },
    NetworkError(String),
    StreamInterrupted,
}
```

The conversation loop handles `RateLimit` with exponential backoff. `ContextTooLong` triggers compaction. Other errors propagate to the drone for reporting via Queen.

## Retry Policy

- Rate limit: exponential backoff, respect `retry_after` header, max 3 retries
- Network errors: retry once after 5s
- Server errors (5xx): retry once after 10s
- Auth/model errors: fail immediately
- Stream interruptions: retry the full request once (idempotent — same messages)
