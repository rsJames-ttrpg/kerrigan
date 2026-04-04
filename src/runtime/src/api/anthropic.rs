use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use super::error::ApiError;
use super::sse::SseParser;
use super::types::*;
use super::{ApiClient, EventStream};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    api_key: String,
    model: String,
    base_url: String,
    http: reqwest::Client,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            http: reqwest::Client::new(),
            max_tokens: 16384,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

// --- Anthropic API request types ---

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    system: Vec<AnthropicSystemBlock>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

fn translate_request(request: &ApiRequest, model: &str) -> AnthropicRequest {
    let system = request
        .system
        .iter()
        .map(|s| AnthropicSystemBlock {
            block_type: "text".to_string(),
            text: s.text.clone(),
        })
        .collect();

    let messages = request
        .messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let content = msg
                .content
                .iter()
                .map(|block| match block {
                    ContentBlock::Text { text } => {
                        serde_json::json!({"type": "text", "text": text})
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input})
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        serde_json::json!({"type": "tool_result", "tool_use_id": tool_use_id, "content": content, "is_error": is_error})
                    }
                })
                .collect();
            AnthropicMessage {
                role: role.to_string(),
                content,
            }
        })
        .collect();

    let tools = request
        .tools
        .iter()
        .map(|t| AnthropicTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect();

    AnthropicRequest {
        model: model.to_string(),
        max_tokens: request.max_tokens,
        system,
        messages,
        tools,
        stream: true,
        temperature: request.temperature,
    }
}

// --- Anthropic SSE event types ---

#[derive(Deserialize, Debug)]
struct AnthropicSseEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    message: Option<AnthropicSseMessage>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    content_block: Option<AnthropicContentBlock>,
    #[serde(default)]
    delta: Option<serde_json::Value>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Debug)]
struct AnthropicSseMessage {
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Debug)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

/// State machine for parsing Anthropic SSE events into StreamEvents.
/// Handles tool use input accumulation across multiple deltas.
pub(crate) struct AnthropicEventTranslator {
    /// Accumulated partial JSON per content block index (for tool use)
    tool_input_buffers: HashMap<usize, String>,
    /// Tool use metadata per content block index
    tool_use_meta: HashMap<usize, (String, String)>, // (id, name)
    /// Accumulated usage
    usage: TokenUsage,
}

impl AnthropicEventTranslator {
    pub fn new() -> Self {
        Self {
            tool_input_buffers: HashMap::new(),
            tool_use_meta: HashMap::new(),
            usage: TokenUsage::default(),
        }
    }

    /// Process a parsed SSE event and return zero or more StreamEvents.
    pub fn translate(&mut self, event_type: Option<&str>, data: &str) -> Vec<StreamEvent> {
        let parsed: AnthropicSseEvent = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(err) => {
                return vec![StreamEvent::Error(ApiError::NetworkError(format!(
                    "failed to parse SSE data: {err}"
                )))];
            }
        };

        match parsed.event_type.as_str() {
            "message_start" => {
                if let Some(msg) = &parsed.message {
                    if let Some(u) = &msg.usage {
                        self.usage.input_tokens = u.input_tokens;
                        self.usage.cache_read_tokens = u.cache_read_input_tokens;
                        self.usage.cache_creation_tokens = u.cache_creation_input_tokens;
                    }
                }
                vec![]
            }
            "content_block_start" => {
                if let (Some(idx), Some(block)) = (parsed.index, &parsed.content_block) {
                    if block.block_type == "tool_use" {
                        let id = block.id.clone().unwrap_or_default();
                        let name = block.name.clone().unwrap_or_default();
                        self.tool_use_meta.insert(idx, (id, name));
                        self.tool_input_buffers.insert(idx, String::new());
                    }
                }
                vec![]
            }
            "content_block_delta" => {
                let idx = parsed.index.unwrap_or(0);
                if let Some(delta) = &parsed.delta {
                    let delta_type = delta.get("type").and_then(|t| t.as_str());
                    match delta_type {
                        Some("text_delta") => {
                            let text = delta
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            vec![StreamEvent::TextDelta(text)]
                        }
                        Some("input_json_delta") => {
                            let partial = delta
                                .get("partial_json")
                                .and_then(|t| t.as_str())
                                .unwrap_or("");
                            if let Some(buf) = self.tool_input_buffers.get_mut(&idx) {
                                buf.push_str(partial);
                            }
                            vec![]
                        }
                        _ => vec![],
                    }
                } else {
                    vec![]
                }
            }
            "content_block_stop" => {
                let idx = parsed.index.unwrap_or(0);
                if let Some((id, name)) = self.tool_use_meta.remove(&idx) {
                    let raw_input = self.tool_input_buffers.remove(&idx).unwrap_or_default();
                    let input = if raw_input.is_empty() {
                        serde_json::Value::Object(serde_json::Map::new())
                    } else {
                        serde_json::from_str(&raw_input)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
                    };
                    vec![StreamEvent::ToolUse { id, name, input }]
                } else {
                    vec![]
                }
            }
            "message_delta" => {
                if let Some(u) = &parsed.usage {
                    self.usage.output_tokens = u.output_tokens;
                }
                vec![]
            }
            "message_stop" => {
                let usage = std::mem::take(&mut self.usage);
                vec![StreamEvent::Usage(usage), StreamEvent::MessageStop]
            }
            _ => {
                // ping, error, or unknown event types — ignore
                if let Some("error") = event_type {
                    let error_msg = parsed
                        .delta
                        .as_ref()
                        .and_then(|d| d.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string();
                    vec![StreamEvent::Error(ApiError::ServerError {
                        status: 0,
                        body: error_msg,
                    })]
                } else {
                    vec![]
                }
            }
        }
    }
}

/// Parse an HTTP error response into an ApiError.
pub(crate) fn parse_error_response(status: u16, body: &str) -> ApiError {
    match status {
        429 => {
            // Try to extract retry-after from the response body
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

fn build_anthropic_stream(
    response: reqwest::Response,
) -> impl futures_util::Stream<Item = StreamEvent> + Send {
    futures_util::stream::unfold(
        (
            Box::pin(response.bytes_stream())
                as std::pin::Pin<
                    Box<
                        dyn futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>>
                            + Send,
                    >,
                >,
            SseParser::new(),
            AnthropicEventTranslator::new(),
            std::collections::VecDeque::<StreamEvent>::new(),
        ),
        |(mut byte_stream, mut parser, mut translator, mut pending)| async move {
            loop {
                // Drain pending events first
                if let Some(event) = pending.pop_front() {
                    return Some((event, (byte_stream, parser, translator, pending)));
                }

                // Read next chunk from byte stream
                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        let text = String::from_utf8_lossy(&chunk);
                        let sse_events = parser.feed(&text);
                        for sse_event in sse_events {
                            let stream_events = translator
                                .translate(sse_event.event_type.as_deref(), &sse_event.data);
                            pending.extend(stream_events);
                        }
                    }
                    Some(Err(_)) => {
                        return Some((
                            StreamEvent::Error(ApiError::StreamInterrupted),
                            (byte_stream, parser, translator, pending),
                        ));
                    }
                    None => return None,
                }
            }
        },
    )
}

#[async_trait]
impl ApiClient for AnthropicClient {
    async fn stream(&self, request: ApiRequest) -> Result<EventStream, ApiError> {
        let body = translate_request(&request, &self.model);
        let url = format!("{}/v1/messages", self.base_url);

        let response = self
            .http
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::NetworkError(e.to_string()))?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "failed to read body".to_string());
            return Err(parse_error_response(status, &body));
        }

        let stream = build_anthropic_stream(response);
        Ok(Box::pin(stream))
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn supports_tool_use(&self) -> bool {
        true
    }

    fn max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_delta() {
        let mut translator = AnthropicEventTranslator::new();
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let events = translator.translate(Some("content_block_delta"), data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::TextDelta(text) => assert_eq!(text, "Hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_tool_use_accumulated() {
        let mut translator = AnthropicEventTranslator::new();

        // Start tool use block
        let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"read_file","input":{}}}"#;
        let events = translator.translate(Some("content_block_start"), data);
        assert!(events.is_empty());

        // Accumulate partial JSON
        let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\""}}"#;
        let events = translator.translate(Some("content_block_delta"), data);
        assert!(events.is_empty());

        let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":":\"/tmp\"}"}}"#;
        let events = translator.translate(Some("content_block_delta"), data);
        assert!(events.is_empty());

        // Stop — should emit completed tool use
        let data = r#"{"type":"content_block_stop","index":1}"#;
        let events = translator.translate(Some("content_block_stop"), data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(name, "read_file");
                assert_eq!(input["path"], "/tmp");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_usage_merged() {
        let mut translator = AnthropicEventTranslator::new();

        // message_start with input usage
        let data = r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"cache_read_input_tokens":50,"cache_creation_input_tokens":25}}}"#;
        let events = translator.translate(Some("message_start"), data);
        assert!(events.is_empty());

        // message_delta with output usage
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        let events = translator.translate(Some("message_delta"), data);
        assert!(events.is_empty());

        // message_stop — should emit merged usage
        let data = r#"{"type":"message_stop"}"#;
        let events = translator.translate(Some("message_stop"), data);
        assert_eq!(events.len(), 2);
        match &events[0] {
            StreamEvent::Usage(usage) => {
                assert_eq!(usage.input_tokens, 100);
                assert_eq!(usage.output_tokens, 42);
                assert_eq!(usage.cache_read_tokens, 50);
                assert_eq!(usage.cache_creation_tokens, 25);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
        assert!(matches!(&events[1], StreamEvent::MessageStop));
    }

    #[test]
    fn test_error_response_rate_limit() {
        let error = parse_error_response(429, r#"{"error":{"message":"rate limited"}}"#);
        assert!(matches!(error, ApiError::RateLimit { .. }));
    }

    #[test]
    fn test_error_response_auth() {
        let error = parse_error_response(401, r#"{"error":{"message":"invalid key"}}"#);
        assert!(matches!(error, ApiError::AuthFailed));
    }

    #[test]
    fn test_error_response_not_found() {
        let error = parse_error_response(404, r#"{"error":{"message":"model not found"}}"#);
        assert!(matches!(error, ApiError::ModelNotFound));
    }

    #[test]
    fn test_error_response_server() {
        let error = parse_error_response(500, "internal server error");
        match error {
            ApiError::ServerError { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "internal server error");
            }
            other => panic!("expected ServerError, got {other:?}"),
        }
    }

    #[test]
    fn test_translate_request_format() {
        let request = ApiRequest {
            system: vec![SystemBlock {
                text: "You are helpful.".to_string(),
            }],
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "hello".to_string(),
                }],
            }],
            tools: vec![ToolDefinition {
                name: "read_file".to_string(),
                description: "Read a file".to_string(),
                input_schema: serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            }],
            max_tokens: 4096,
            temperature: Some(0.5),
        };

        let translated = translate_request(&request, "claude-sonnet-4-20250514");
        assert_eq!(translated.model, "claude-sonnet-4-20250514");
        assert_eq!(translated.max_tokens, 4096);
        assert!(translated.stream);
        assert_eq!(translated.system.len(), 1);
        assert_eq!(translated.messages.len(), 1);
        assert_eq!(translated.tools.len(), 1);
        assert_eq!(translated.tools[0].name, "read_file");
        assert_eq!(translated.temperature, Some(0.5));
    }
}
