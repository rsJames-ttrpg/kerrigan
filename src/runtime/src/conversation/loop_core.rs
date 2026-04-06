use std::path::PathBuf;
use std::sync::Arc;

use tokio_stream::StreamExt;

use crate::api::{
    ApiClient, ApiClientFactory, ApiRequest, EventStream, StreamEvent, SystemBlock, TokenUsage,
    ToolDefinition,
};
use crate::event::{EventSink, RuntimeEvent};
use crate::permission::PermissionPolicy;
use crate::tools::{ToolContext, ToolRegistry};

use super::session::{ContentBlock, Message, Role, Session};

/// Configuration for the conversation loop
#[derive(Clone)]
pub struct LoopConfig {
    pub max_iterations: u32,
    pub max_context_tokens: u32,
    pub compaction_strategy: CompactionStrategy,
    pub max_tokens_per_response: u32,
    pub temperature: Option<f32>,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            max_context_tokens: 100_000,
            compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 4 },
            max_tokens_per_response: 4096,
            temperature: None,
        }
    }
}

/// Strategy for context compaction when token limits are approached
#[derive(Clone)]
pub enum CompactionStrategy {
    Summarize { preserve_recent: u32 },
    Checkpoint { preserve_recent: u32 },
}

/// Result of a single turn (one user message → potentially many iterations)
pub struct TurnResult {
    pub iterations: u32,
    pub compacted: bool,
    pub usage: TokenUsage,
}

/// Tracks tool calls extracted from streaming responses
struct ToolCall {
    id: String,
    name: String,
    input: serde_json::Value,
}

/// The core conversation loop that drives the agentic workflow
pub struct ConversationLoop {
    pub(super) api_client: Box<dyn ApiClient>,
    pub(super) api_client_factory: Arc<dyn ApiClientFactory>,
    pub(super) tool_registry: Arc<ToolRegistry>,
    pub(super) session: Session,
    pub(super) config: LoopConfig,
    pub(super) event_sink: Arc<dyn EventSink>,
    pub(super) system_prompt: Vec<String>,
    pub(super) permission_policy: PermissionPolicy,
    pub(super) workspace: PathBuf,
    pub(super) agent_depth: u32,
}

impl ConversationLoop {
    pub fn new(
        api_client: Box<dyn ApiClient>,
        api_client_factory: Arc<dyn ApiClientFactory>,
        tool_registry: ToolRegistry,
        config: LoopConfig,
        event_sink: Arc<dyn EventSink>,
        system_prompt: Vec<String>,
        permission_policy: PermissionPolicy,
        workspace: PathBuf,
    ) -> Self {
        Self {
            api_client,
            api_client_factory,
            tool_registry: Arc::new(tool_registry),
            session: Session::new(),
            config,
            event_sink,
            system_prompt,
            permission_policy,
            workspace,
            agent_depth: 0,
        }
    }

    /// Access the session (for sub-agent result extraction, testing, etc.)
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Access the tool registry
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Set the agent nesting depth (used by AgentTool for sub-agent spawning)
    pub fn set_agent_depth(&mut self, depth: u32) {
        self.agent_depth = depth;
    }

    /// Access the API client factory (for sub-agent spawning)
    pub fn api_client_factory(&self) -> &Arc<dyn ApiClientFactory> {
        &self.api_client_factory
    }

    /// Run a single turn: push user message, iterate API calls until no tool calls or max iterations
    pub async fn run_turn(&mut self, task: &str) -> anyhow::Result<TurnResult> {
        // Push user message
        self.session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: task.to_string(),
            }],
            token_estimate: Session::estimate_tokens(task),
        });

        self.event_sink.emit(RuntimeEvent::TurnStart {
            task: task.to_string(),
        });

        let mut iterations = 0u32;
        let mut compacted = false;
        let mut total_usage = TokenUsage::default();

        for _ in 0..self.config.max_iterations {
            iterations += 1;

            // Check context pressure
            if self.session.total_tokens_estimate > self.config.max_context_tokens {
                self.compact().await?;
                compacted = true;
            }

            // Build API request with role translation
            let request = self.build_request();

            // Start heartbeat task
            let heartbeat_sink = self.event_sink.clone();
            let heartbeat = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    heartbeat_sink.emit(RuntimeEvent::Heartbeat);
                }
            });

            // Stream response — abort heartbeat before propagating errors
            let stream_result = self.api_client.stream(request).await;
            heartbeat.abort();
            let stream = stream_result?;
            let (assistant_msg, tool_calls, usage) = self.consume_stream(stream).await?;

            // Accumulate usage
            total_usage.input_tokens += usage.input_tokens;
            total_usage.output_tokens += usage.output_tokens;
            total_usage.cache_read_tokens += usage.cache_read_tokens;
            total_usage.cache_creation_tokens += usage.cache_creation_tokens;
            self.event_sink.emit(RuntimeEvent::Usage(usage));

            // Push assistant message
            self.session.push(assistant_msg);

            // If no tool calls, turn is complete
            if tool_calls.is_empty() {
                break;
            }

            // Execute each tool call
            let ctx = ToolContext {
                workspace: self.workspace.clone(),
                home: self.workspace.clone(),
                event_sink: self.event_sink.clone(),
                tool_registry: Arc::clone(&self.tool_registry),
                agent_depth: self.agent_depth,
            };

            for tc in &tool_calls {
                // Check tool exists and permission
                let Some(tool) = self.tool_registry.get(&tc.name) else {
                    let result_output = format!("Tool not found: '{}'", tc.name);
                    self.session.push(Message {
                        role: Role::Tool,
                        blocks: vec![ContentBlock::ToolResult {
                            tool_use_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: result_output.clone(),
                            is_error: true,
                        }],
                        token_estimate: Session::estimate_tokens(&result_output),
                    });
                    continue;
                };

                if !self
                    .permission_policy
                    .is_allowed(&tc.name, tool.permission())
                {
                    let result_output =
                        format!("Permission denied: tool '{}' is not allowed", tc.name);
                    self.session.push(Message {
                        role: Role::Tool,
                        blocks: vec![ContentBlock::ToolResult {
                            tool_use_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: result_output.clone(),
                            is_error: true,
                        }],
                        token_estimate: Session::estimate_tokens(&result_output),
                    });
                    continue;
                }

                self.event_sink.emit(RuntimeEvent::ToolUseStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    input: tc.input.clone(),
                });

                let start = std::time::Instant::now();
                let result = self
                    .tool_registry
                    .execute(&tc.name, tc.input.clone(), &ctx)
                    .await;
                let duration_ms = start.elapsed().as_millis() as u64;

                self.event_sink.emit(RuntimeEvent::ToolUseEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    result: result.clone(),
                    duration_ms,
                });

                // Push tool result to session
                let token_estimate = Session::estimate_tokens(&result.output);
                self.session.push(Message {
                    role: Role::Tool,
                    blocks: vec![ContentBlock::ToolResult {
                        tool_use_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        output: result.output,
                        is_error: result.is_error,
                    }],
                    token_estimate,
                });
            }
        }

        self.event_sink.emit(RuntimeEvent::TurnEnd {
            iterations,
            total_usage: total_usage.clone(),
        });

        Ok(TurnResult {
            iterations,
            compacted,
            usage: total_usage,
        })
    }

    /// Build an API request from the session, translating between session and API role models.
    ///
    /// Key translations:
    /// - Session::Role::System → ApiRequest.system blocks (NOT in messages array)
    /// - Session::Role::User → api::Message { role: User, content: [Text...] }
    /// - Session::Role::Assistant → api::Message { role: Assistant, content: [Text/ToolUse...] }
    /// - Session::Role::Tool → api::Message { role: User, content: [ToolResult...] }
    ///   (Anthropic puts tool results in user messages)
    /// - Session ContentBlock::ToolResult.output → api ContentBlock::ToolResult.content
    fn build_request(&self) -> ApiRequest {
        let mut system_blocks: Vec<SystemBlock> = self
            .system_prompt
            .iter()
            .map(|s| SystemBlock { text: s.clone() })
            .collect();

        let mut api_messages: Vec<crate::api::types::Message> = Vec::new();

        for msg in &self.session.messages {
            match msg.role {
                Role::System => {
                    // System messages go into the system blocks, not messages array
                    for block in &msg.blocks {
                        if let ContentBlock::Text { text } = block {
                            system_blocks.push(SystemBlock { text: text.clone() });
                        }
                    }
                }
                Role::User => {
                    let content = msg
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => {
                                Some(crate::api::types::ContentBlock::Text { text: text.clone() })
                            }
                            _ => None,
                        })
                        .collect();
                    api_messages.push(crate::api::types::Message {
                        role: crate::api::types::Role::User,
                        content,
                    });
                }
                Role::Assistant => {
                    let content = msg
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => {
                                Some(crate::api::types::ContentBlock::Text { text: text.clone() })
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                Some(crate::api::types::ContentBlock::ToolUse {
                                    id: id.clone(),
                                    name: name.clone(),
                                    input: input.clone(),
                                })
                            }
                            _ => None,
                        })
                        .collect();
                    api_messages.push(crate::api::types::Message {
                        role: crate::api::types::Role::Assistant,
                        content,
                    });
                }
                Role::Tool => {
                    // Tool results go in user messages (Anthropic API convention)
                    let content = msg
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolResult {
                                tool_use_id,
                                output,
                                is_error,
                                ..
                            } => Some(crate::api::types::ContentBlock::ToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: output.clone(), // session "output" → api "content"
                                is_error: *is_error,
                            }),
                            _ => None,
                        })
                        .collect();
                    api_messages.push(crate::api::types::Message {
                        role: crate::api::types::Role::User,
                        content,
                    });
                }
            }
        }

        // Get tool definitions
        let tools: Vec<ToolDefinition> = self.tool_registry.definitions(&[], &[]);

        ApiRequest {
            system: system_blocks,
            messages: api_messages,
            tools,
            max_tokens: self.config.max_tokens_per_response,
            temperature: self.config.temperature,
        }
    }

    /// Consume a stream of events, building the assistant message and extracting tool calls
    async fn consume_stream(
        &self,
        mut stream: EventStream,
    ) -> anyhow::Result<(Message, Vec<ToolCall>, TokenUsage)> {
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut usage = TokenUsage::default();

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::TextDelta(text) => {
                    self.event_sink.emit(RuntimeEvent::TextDelta(text.clone()));
                    text_parts.push(text);
                }
                StreamEvent::ToolUse { id, name, input } => {
                    tool_calls.push(ToolCall { id, name, input });
                }
                StreamEvent::Usage(u) => {
                    usage = u;
                }
                StreamEvent::MessageStop => {
                    break;
                }
                StreamEvent::Error(e) => {
                    return Err(anyhow::anyhow!("Stream error: {e:?}"));
                }
            }
        }

        // Build assistant message with all content blocks
        let mut blocks: Vec<ContentBlock> = Vec::new();

        let combined_text = text_parts.join("");
        if !combined_text.is_empty() {
            blocks.push(ContentBlock::Text {
                text: combined_text.clone(),
            });
        }

        for tc in &tool_calls {
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            });
        }

        let token_estimate =
            Session::estimate_tokens(&combined_text) + tool_calls.len() as u32 * 20; // rough estimate for tool use blocks

        let assistant_msg = Message {
            role: Role::Assistant,
            blocks,
            token_estimate,
        };

        Ok((assistant_msg, tool_calls, usage))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::error::ApiError;
    use crate::event::NullEventSink;
    use crate::tools::{PermissionLevel, Tool, ToolResult as TToolResult};
    use async_trait::async_trait;
    use std::sync::Mutex;

    // --- Mock API Client ---

    /// A mock API client that returns predetermined responses in sequence
    struct MockApiClient {
        responses: Mutex<Vec<Vec<StreamEvent>>>,
    }

    impl MockApiClient {
        fn new(responses: Vec<Vec<StreamEvent>>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl ApiClient for MockApiClient {
        async fn stream(&self, _request: ApiRequest) -> Result<EventStream, ApiError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                panic!("MockApiClient: no more responses available");
            }
            let events = responses.remove(0);
            Ok(Box::pin(tokio_stream::iter(events)))
        }

        fn model(&self) -> &str {
            "mock-model"
        }

        fn supports_tool_use(&self) -> bool {
            true
        }

        fn max_tokens(&self) -> u32 {
            4096
        }
    }

    struct MockApiClientFactory;

    impl ApiClientFactory for MockApiClientFactory {
        fn create(&self) -> Box<dyn ApiClient> {
            Box::new(MockApiClient::new(vec![]))
        }
    }

    // --- Mock Tool ---

    struct MockTool {
        name: String,
        calls: Arc<Mutex<Vec<serde_json::Value>>>,
        result: String,
    }

    impl MockTool {
        fn new(name: &str, result: &str) -> (Self, Arc<Mutex<Vec<serde_json::Value>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    name: name.to_string(),
                    calls: calls.clone(),
                    result: result.to_string(),
                },
                calls,
            )
        }
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "a mock tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::ReadOnly
        }
        async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> TToolResult {
            self.calls.lock().unwrap().push(input);
            TToolResult::success(self.result.clone())
        }
    }

    fn make_loop(api_client: Box<dyn ApiClient>, tool_registry: ToolRegistry) -> ConversationLoop {
        ConversationLoop::new(
            api_client,
            Arc::new(MockApiClientFactory),
            tool_registry,
            LoopConfig::default(),
            Arc::new(NullEventSink),
            vec!["You are a helpful assistant.".into()],
            PermissionPolicy::allow_all(),
            PathBuf::from("/tmp/test-workspace"),
        )
    }

    // --- Tests ---

    #[tokio::test]
    async fn test_simple_text_response() {
        let client = MockApiClient::new(vec![vec![
            StreamEvent::TextDelta("Hello, world!".into()),
            StreamEvent::Usage(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            }),
            StreamEvent::MessageStop,
        ]]);

        let mut conv = make_loop(Box::new(client), ToolRegistry::new());
        let result = conv.run_turn("Hi there").await.unwrap();

        assert_eq!(result.iterations, 1);
        assert!(!result.compacted);
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);

        // Session should have: user message + assistant message
        assert_eq!(conv.session().messages.len(), 2);
        assert_eq!(conv.session().messages[0].role, Role::User);
        assert_eq!(conv.session().messages[1].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_tool_use_turn() {
        // Response 1: tool call
        // Response 2: text after tool result
        let client = MockApiClient::new(vec![
            vec![
                StreamEvent::ToolUse {
                    id: "tool_1".into(),
                    name: "mock_tool".into(),
                    input: serde_json::json!({"arg": "value"}),
                },
                StreamEvent::Usage(TokenUsage::default()),
                StreamEvent::MessageStop,
            ],
            vec![
                StreamEvent::TextDelta("Done!".into()),
                StreamEvent::Usage(TokenUsage::default()),
                StreamEvent::MessageStop,
            ],
        ]);

        let mut registry = ToolRegistry::new();
        let (tool, calls) = MockTool::new("mock_tool", "tool output");
        registry.register(Arc::new(tool));

        let mut conv = make_loop(Box::new(client), registry);
        let result = conv.run_turn("Do something").await.unwrap();

        assert_eq!(result.iterations, 2);

        // Verify tool was called
        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["arg"], "value");

        // Session: user, assistant(tool_use), tool_result, assistant(text)
        assert_eq!(conv.session().messages.len(), 4);
        assert_eq!(conv.session().messages[0].role, Role::User);
        assert_eq!(conv.session().messages[1].role, Role::Assistant);
        assert_eq!(conv.session().messages[2].role, Role::Tool);
        assert_eq!(conv.session().messages[3].role, Role::Assistant);
    }

    #[tokio::test]
    async fn test_max_iterations_reached() {
        // Every response triggers a tool call, never returns text-only
        let responses: Vec<Vec<StreamEvent>> = (0..20)
            .map(|i| {
                vec![
                    StreamEvent::ToolUse {
                        id: format!("tool_{i}"),
                        name: "mock_tool".into(),
                        input: serde_json::json!({}),
                    },
                    StreamEvent::Usage(TokenUsage::default()),
                    StreamEvent::MessageStop,
                ]
            })
            .collect();

        let client = MockApiClient::new(responses);

        let mut registry = ToolRegistry::new();
        let (tool, _calls) = MockTool::new("mock_tool", "ok");
        registry.register(Arc::new(tool));

        let mut conv = make_loop(Box::new(client), registry);
        let result = conv.run_turn("Loop forever").await.unwrap();

        assert_eq!(result.iterations, 20); // hit max_iterations
    }

    #[tokio::test]
    async fn test_multiple_tool_calls_in_one_response() {
        let client = MockApiClient::new(vec![
            vec![
                StreamEvent::ToolUse {
                    id: "t1".into(),
                    name: "tool_a".into(),
                    input: serde_json::json!({"x": 1}),
                },
                StreamEvent::ToolUse {
                    id: "t2".into(),
                    name: "tool_b".into(),
                    input: serde_json::json!({"y": 2}),
                },
                StreamEvent::Usage(TokenUsage::default()),
                StreamEvent::MessageStop,
            ],
            vec![
                StreamEvent::TextDelta("All done".into()),
                StreamEvent::Usage(TokenUsage::default()),
                StreamEvent::MessageStop,
            ],
        ]);

        let mut registry = ToolRegistry::new();
        let (tool_a, calls_a) = MockTool::new("tool_a", "result_a");
        let (tool_b, calls_b) = MockTool::new("tool_b", "result_b");
        registry.register(Arc::new(tool_a));
        registry.register(Arc::new(tool_b));

        let mut conv = make_loop(Box::new(client), registry);
        let result = conv.run_turn("Use both tools").await.unwrap();

        assert_eq!(result.iterations, 2);

        // Both tools called
        assert_eq!(calls_a.lock().unwrap().len(), 1);
        assert_eq!(calls_b.lock().unwrap().len(), 1);

        // Session: user, assistant(2 tool_use), tool_result_a, tool_result_b, assistant(text)
        assert_eq!(conv.session().messages.len(), 5);
    }

    #[tokio::test]
    async fn test_build_request_role_translation() {
        let client = MockApiClient::new(vec![]);
        let mut conv = make_loop(Box::new(client), ToolRegistry::new());

        // Manually build a session with all role types
        conv.session.push(Message {
            role: Role::System,
            blocks: vec![ContentBlock::Text {
                text: "System context".into(),
            }],
            token_estimate: 5,
        });
        conv.session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "Hello".into(),
            }],
            token_estimate: 2,
        });
        conv.session.push(Message {
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "Let me check".into(),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "read".into(),
                    input: serde_json::json!({}),
                },
            ],
            token_estimate: 10,
        });
        conv.session.push(Message {
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                tool_name: "read".into(),
                output: "file contents".into(),
                is_error: false,
            }],
            token_estimate: 4,
        });

        let request = conv.build_request();

        // System prompt + session system message = 2 system blocks
        assert_eq!(request.system.len(), 2);
        assert_eq!(request.system[0].text, "You are a helpful assistant.");
        assert_eq!(request.system[1].text, "System context");

        // Messages: user, assistant, tool_result (as user)
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[0].role, crate::api::types::Role::User);
        assert_eq!(request.messages[1].role, crate::api::types::Role::Assistant);
        assert_eq!(request.messages[2].role, crate::api::types::Role::User); // Tool → User

        // Verify tool result field translation: output → content
        match &request.messages[2].content[0] {
            crate::api::types::ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(content, "file contents"); // "output" mapped to "content"
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    #[tokio::test]
    async fn test_text_deltas_accumulated() {
        let client = MockApiClient::new(vec![vec![
            StreamEvent::TextDelta("Hello".into()),
            StreamEvent::TextDelta(", ".into()),
            StreamEvent::TextDelta("world!".into()),
            StreamEvent::Usage(TokenUsage::default()),
            StreamEvent::MessageStop,
        ]]);

        let mut conv = make_loop(Box::new(client), ToolRegistry::new());
        conv.run_turn("Hi").await.unwrap();

        // Check the assistant message has combined text
        let assistant_msg = &conv.session().messages[1];
        match &assistant_msg.blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello, world!"),
            _ => panic!("expected text block"),
        }
    }
}
