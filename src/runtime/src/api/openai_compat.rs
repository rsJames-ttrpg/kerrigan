use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use super::error::ApiError;
use super::sse::SseParser;
use super::types::*;
use super::{ApiClient, EventStream};

pub struct OpenAiCompatClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
    max_tokens: u32,
}

impl OpenAiCompatClient {
    pub fn new(base_url: String, api_key: Option<String>, model: String) -> Self {
        Self {
            base_url,
            api_key,
            model,
            http: reqwest::Client::new(),
            max_tokens: 16384,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

// --- OpenAI request types ---

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OpenAiTool>,
    max_tokens: u32,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize)]
struct OpenAiToolCallMessage {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

fn translate_request(request: &ApiRequest, model: &str) -> OpenAiRequest {
    let mut messages = Vec::new();

    // System messages
    for sys in &request.system {
        messages.push(OpenAiMessage {
            role: "system".to_string(),
            content: Some(serde_json::Value::String(sys.text.clone())),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Conversation messages
    for msg in &request.messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        // Check if this message contains tool results (they become separate messages)
        let mut tool_results: Vec<&ContentBlock> = Vec::new();
        let mut other_content: Vec<&ContentBlock> = Vec::new();
        let mut tool_calls: Vec<&ContentBlock> = Vec::new();

        for block in &msg.content {
            match block {
                ContentBlock::ToolResult { .. } => tool_results.push(block),
                ContentBlock::ToolUse { .. } => tool_calls.push(block),
                _ => other_content.push(block),
            }
        }

        // Emit text content
        if !other_content.is_empty() || !tool_calls.is_empty() {
            let content = if other_content.len() == 1 {
                if let ContentBlock::Text { text } = other_content[0] {
                    Some(serde_json::Value::String(text.clone()))
                } else {
                    None
                }
            } else if other_content.is_empty() {
                None
            } else {
                let parts: Vec<serde_json::Value> = other_content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => {
                            Some(serde_json::json!({"type": "text", "text": text}))
                        }
                        _ => None,
                    })
                    .collect();
                Some(serde_json::Value::Array(parts))
            };

            let tc = if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => {
                                Some(OpenAiToolCallMessage {
                                    id: id.clone(),
                                    call_type: "function".to_string(),
                                    function: OpenAiToolCallFunction {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input).unwrap_or_default(),
                                    },
                                })
                            }
                            _ => None,
                        })
                        .collect(),
                )
            };

            messages.push(OpenAiMessage {
                role: role.to_string(),
                content,
                tool_calls: tc,
                tool_call_id: None,
            });
        }

        // Emit tool results as separate "tool" role messages
        for tr in tool_results {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = tr
            {
                messages.push(OpenAiMessage {
                    role: "tool".to_string(),
                    content: Some(serde_json::Value::String(content.clone())),
                    tool_calls: None,
                    tool_call_id: Some(tool_use_id.clone()),
                });
            }
        }
    }

    let tools = request
        .tools
        .iter()
        .map(|t| OpenAiTool {
            tool_type: "function".to_string(),
            function: OpenAiFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect();

    OpenAiRequest {
        model: model.to_string(),
        messages,
        tools,
        max_tokens: request.max_tokens,
        stream: true,
        temperature: request.temperature,
        stream_options: Some(StreamOptions {
            include_usage: true,
        }),
    }
}

// --- OpenAI SSE event types ---

#[derive(Deserialize, Debug)]
struct OpenAiChunk {
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize, Debug)]
struct OpenAiChoice {
    #[serde(default)]
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Deserialize, Debug)]
struct OpenAiToolCallDelta {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<OpenAiFunctionDelta>,
}

#[derive(Deserialize, Debug)]
struct OpenAiFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// State machine for parsing OpenAI chat completion chunks into StreamEvents.
pub(crate) struct OpenAiEventTranslator {
    /// Accumulated tool call arguments per index
    tool_arg_buffers: HashMap<usize, String>,
    /// Tool call metadata per index (id, name)
    tool_call_meta: HashMap<usize, (String, String)>,
}

impl OpenAiEventTranslator {
    pub fn new() -> Self {
        Self {
            tool_arg_buffers: HashMap::new(),
            tool_call_meta: HashMap::new(),
        }
    }

    /// Process an SSE data payload and return zero or more StreamEvents.
    pub fn translate(&mut self, data: &str) -> Vec<StreamEvent> {
        // Handle [DONE] sentinel
        if data.trim() == "[DONE]" {
            return self.flush_tool_calls();
        }

        let chunk: OpenAiChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(err) => {
                return vec![StreamEvent::Error(ApiError::NetworkError(format!(
                    "failed to parse chunk: {err}"
                )))];
            }
        };

        let mut events = Vec::new();

        // Process usage if present (some providers send it in the final chunk)
        if let Some(usage) = &chunk.usage {
            events.push(StreamEvent::Usage(TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            }));
        }

        for choice in &chunk.choices {
            // Text content delta
            if let Some(content) = &choice.delta.content {
                if !content.is_empty() {
                    events.push(StreamEvent::TextDelta(content.clone()));
                }
            }

            // Tool call deltas
            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    let idx = tc.index;

                    // New tool call — store metadata
                    if let Some(id) = &tc.id {
                        let name = tc
                            .function
                            .as_ref()
                            .and_then(|f| f.name.clone())
                            .unwrap_or_default();
                        self.tool_call_meta.insert(idx, (id.clone(), name));
                        self.tool_arg_buffers.insert(idx, String::new());
                    }

                    // Accumulate arguments
                    if let Some(func) = &tc.function {
                        if let Some(args) = &func.arguments {
                            if let Some(buf) = self.tool_arg_buffers.get_mut(&idx) {
                                buf.push_str(args);
                            }
                        }
                    }
                }
            }

            // finish_reason = "stop" or "tool_calls" — emit completed tool calls and stop
            if let Some(reason) = &choice.finish_reason {
                let mut tool_events = self.flush_tool_calls();
                events.append(&mut tool_events);
                if reason == "stop" || reason == "tool_calls" {
                    events.push(StreamEvent::MessageStop);
                }
            }
        }

        events
    }

    fn flush_tool_calls(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let mut indices: Vec<usize> = self.tool_call_meta.keys().copied().collect();
        indices.sort();

        for idx in indices {
            if let Some((id, name)) = self.tool_call_meta.remove(&idx) {
                let raw_args = self.tool_arg_buffers.remove(&idx).unwrap_or_default();
                let input = if raw_args.is_empty() {
                    serde_json::Value::Object(serde_json::Map::new())
                } else {
                    serde_json::from_str(&raw_args)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
                };
                events.push(StreamEvent::ToolUse { id, name, input });
            }
        }

        events
    }
}

/// Parse an HTTP error response into an ApiError.
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

fn build_openai_stream(
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
            OpenAiEventTranslator::new(),
            std::collections::VecDeque::<StreamEvent>::new(),
        ),
        |(mut byte_stream, mut parser, mut translator, mut pending)| async move {
            loop {
                if let Some(event) = pending.pop_front() {
                    return Some((event, (byte_stream, parser, translator, pending)));
                }

                match byte_stream.next().await {
                    Some(Ok(chunk)) => {
                        let text = String::from_utf8_lossy(&chunk);
                        let sse_events = parser.feed(&text);
                        for sse_event in sse_events {
                            let stream_events = translator.translate(&sse_event.data);
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
impl ApiClient for OpenAiCompatClient {
    async fn stream(&self, request: ApiRequest) -> Result<EventStream, ApiError> {
        let body = translate_request(&request, &self.model);
        let url = format!("{}/v1/chat/completions", self.base_url);

        let mut req = self
            .http
            .post(&url)
            .header("content-type", "application/json");

        if let Some(key) = &self.api_key {
            req = req.header("authorization", format!("Bearer {key}"));
        }

        let response = req
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

        let stream = build_openai_stream(response);
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
    fn test_parse_text_chunk() {
        let mut translator = OpenAiEventTranslator::new();
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let events = translator.translate(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::TextDelta(text) => assert_eq!(text, "Hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_tool_call_incremental() {
        let mut translator = OpenAiEventTranslator::new();

        // First chunk: tool call start with id and name
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}]}"#;
        let events = translator.translate(data);
        assert!(events.is_empty());

        // Second chunk: arguments fragment
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"/tmp\"}"}}]},"finish_reason":null}]}"#;
        let events = translator.translate(data);
        assert!(events.is_empty());

        // Final chunk: finish_reason triggers flush
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
        let events = translator.translate(data);
        assert_eq!(events.len(), 2); // ToolUse + MessageStop
        match &events[0] {
            StreamEvent::ToolUse { id, name, input } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read_file");
                assert_eq!(input["path"], "/tmp");
            }
            other => panic!("expected ToolUse, got {other:?}"),
        }
        assert!(matches!(&events[1], StreamEvent::MessageStop));
    }

    #[test]
    fn test_parse_done_sentinel() {
        let mut translator = OpenAiEventTranslator::new();
        let events = translator.translate("[DONE]");
        assert!(events.is_empty()); // No pending tool calls, so empty
    }

    #[test]
    fn test_parse_usage_in_chunk() {
        let mut translator = OpenAiEventTranslator::new();
        let data = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let events = translator.translate(data);
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Usage(usage) => {
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 5);
            }
            other => panic!("expected Usage, got {other:?}"),
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
                input_schema: serde_json::json!({"type": "object"}),
            }],
            max_tokens: 4096,
            temperature: Some(0.5),
        };

        let translated = translate_request(&request, "gpt-4");
        assert_eq!(translated.model, "gpt-4");
        assert_eq!(translated.max_tokens, 4096);
        assert!(translated.stream);
        assert_eq!(translated.temperature, Some(0.5));
        // System message + user message = 2
        assert_eq!(translated.messages.len(), 2);
        assert_eq!(translated.messages[0].role, "system");
        assert_eq!(translated.messages[1].role, "user");
        assert_eq!(translated.tools.len(), 1);
        assert_eq!(translated.tools[0].function.name, "read_file");
    }

    #[test]
    fn test_translate_request_with_tool_results() {
        let request = ApiRequest {
            system: vec![],
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::ToolUse {
                        id: "call_1".to_string(),
                        name: "read_file".to_string(),
                        input: serde_json::json!({"path": "/tmp"}),
                    }],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "call_1".to_string(),
                        content: "file contents".to_string(),
                        is_error: false,
                    }],
                },
            ],
            tools: vec![],
            max_tokens: 1024,
            temperature: None,
        };

        let translated = translate_request(&request, "gpt-4");
        // Assistant message with tool_calls + tool result message
        assert_eq!(translated.messages.len(), 2);
        assert_eq!(translated.messages[0].role, "assistant");
        assert!(translated.messages[0].tool_calls.is_some());
        assert_eq!(translated.messages[1].role, "tool");
        assert_eq!(
            translated.messages[1].tool_call_id.as_deref(),
            Some("call_1")
        );
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
}
