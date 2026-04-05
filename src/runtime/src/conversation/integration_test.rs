//! End-to-end integration test: multi-turn conversation with tool use.
//!
//! Simulates:
//! 1. User says "Create a file called test.txt with 'hello'"
//! 2. Assistant calls `write_file` tool
//! 3. Tool returns success
//! 4. Assistant says "Done, I created test.txt"
//!
//! Verifies session state, file creation, and event emission.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::api::error::ApiError;
use crate::api::types::{StreamEvent, TokenUsage};
use crate::api::{ApiClient, ApiClientFactory, ApiRequest, EventStream};
use crate::conversation::loop_core::{CompactionStrategy, ConversationLoop, LoopConfig};
use crate::conversation::session::Role;
use crate::event::{EventSink, NullEventSink, RuntimeEvent};
use crate::permission::PermissionPolicy;
use crate::tools::ToolRegistry;

// --- Event recording sink ---

struct RecordingEventSink {
    events: Mutex<Vec<String>>,
}

impl RecordingEventSink {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventSink for RecordingEventSink {
    fn emit(&self, event: RuntimeEvent) {
        let tag = match &event {
            RuntimeEvent::TurnStart { .. } => "TurnStart",
            RuntimeEvent::TextDelta(_) => "TextDelta",
            RuntimeEvent::ToolUseStart { .. } => "ToolUseStart",
            RuntimeEvent::ToolUseEnd { .. } => "ToolUseEnd",
            RuntimeEvent::Usage(_) => "Usage",
            RuntimeEvent::Heartbeat => "Heartbeat",
            RuntimeEvent::TurnEnd { .. } => "TurnEnd",
            RuntimeEvent::Error(_) => "Error",
            RuntimeEvent::CompactionTriggered { .. } => "CompactionTriggered",
            RuntimeEvent::CheckpointCreated { .. } => "CheckpointCreated",
        };
        self.events.lock().unwrap().push(tag.to_string());
    }
}

// --- Mock API client that simulates a multi-turn conversation ---

struct MultiTurnMockApiClient {
    call_count: Mutex<u32>,
}

impl MultiTurnMockApiClient {
    fn new() -> Self {
        Self {
            call_count: Mutex::new(0),
        }
    }
}

#[async_trait]
impl ApiClient for MultiTurnMockApiClient {
    async fn stream(&self, _request: ApiRequest) -> Result<EventStream, ApiError> {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        let call_num = *count;

        let events = match call_num {
            1 => {
                // First call: assistant calls write_file tool
                vec![
                    StreamEvent::TextDelta("I'll create that file for you.".into()),
                    StreamEvent::ToolUse {
                        id: "tool_call_1".into(),
                        name: "write_file".into(),
                        input: serde_json::json!({
                            "file_path": "test.txt",
                            "content": "hello"
                        }),
                    },
                    StreamEvent::Usage(TokenUsage {
                        input_tokens: 50,
                        output_tokens: 30,
                        cache_read_tokens: 10,
                        cache_creation_tokens: 0,
                    }),
                    StreamEvent::MessageStop,
                ]
            }
            2 => {
                // Second call: assistant gives final response after tool result
                vec![
                    StreamEvent::TextDelta(
                        "Done, I created test.txt with the content 'hello'.".into(),
                    ),
                    StreamEvent::Usage(TokenUsage {
                        input_tokens: 80,
                        output_tokens: 20,
                        cache_read_tokens: 30,
                        cache_creation_tokens: 0,
                    }),
                    StreamEvent::MessageStop,
                ]
            }
            _ => panic!("Unexpected call number: {call_num}"),
        };

        Ok(Box::pin(tokio_stream::iter(events)))
    }

    fn model(&self) -> &str {
        "mock-integration"
    }

    fn supports_tool_use(&self) -> bool {
        true
    }

    fn max_tokens(&self) -> u32 {
        4096
    }
}

struct MockFactory;
impl ApiClientFactory for MockFactory {
    fn create(&self) -> Box<dyn ApiClient> {
        Box::new(MultiTurnMockApiClient::new())
    }
}

#[tokio::test]
async fn test_full_conversation_turn_with_tool_use() {
    let workspace = tempfile::tempdir().unwrap();
    let workspace_path = workspace.path().to_path_buf();

    let event_sink = Arc::new(RecordingEventSink::new());

    // Set up tool registry with write_file tool
    let registry = crate::tools::default_registry();

    let config = LoopConfig {
        max_iterations: 10,
        max_context_tokens: 100_000,
        compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 4 },
        max_tokens_per_response: 4096,
        temperature: None,
    };

    let mut conv = ConversationLoop::new(
        Box::new(MultiTurnMockApiClient::new()),
        Arc::new(MockFactory),
        registry,
        config,
        event_sink.clone(),
        vec!["You are a helpful coding assistant.".into()],
        PermissionPolicy::allow_all(),
        workspace_path.clone(),
    );

    // Run the turn
    let result = conv
        .run_turn("Create a file called test.txt with 'hello'")
        .await
        .unwrap();

    // Verify turn result
    assert_eq!(result.iterations, 2); // tool call + final response
    assert!(!result.compacted);
    assert_eq!(result.usage.input_tokens, 130); // 50 + 80
    assert_eq!(result.usage.output_tokens, 50); // 30 + 20

    // Verify session messages
    let session = conv.session();
    assert_eq!(session.messages.len(), 4);
    // Message 0: User
    assert_eq!(session.messages[0].role, Role::User);
    // Message 1: Assistant (text + tool_use)
    assert_eq!(session.messages[1].role, Role::Assistant);
    assert_eq!(session.messages[1].blocks.len(), 2); // text + tool_use
    // Message 2: Tool result
    assert_eq!(session.messages[2].role, Role::Tool);
    // Message 3: Final assistant text
    assert_eq!(session.messages[3].role, Role::Assistant);

    // Verify the file was actually written
    let file_path = workspace_path.join("test.txt");
    assert!(file_path.exists(), "test.txt should have been created");
    let contents = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(contents, "hello");

    // Verify events were emitted
    let events = event_sink.events();
    assert!(events.contains(&"TurnStart".to_string()));
    assert!(events.contains(&"ToolUseStart".to_string()));
    assert!(events.contains(&"ToolUseEnd".to_string()));
    assert!(events.contains(&"TurnEnd".to_string()));
    assert!(events.contains(&"TextDelta".to_string()));
    assert!(events.contains(&"Usage".to_string()));
}

#[tokio::test]
async fn test_text_only_conversation() {
    // Simple case: no tool calls, just text response
    struct TextOnlyClient;

    #[async_trait]
    impl ApiClient for TextOnlyClient {
        async fn stream(&self, _request: ApiRequest) -> Result<EventStream, ApiError> {
            Ok(Box::pin(tokio_stream::iter(vec![
                StreamEvent::TextDelta("Hello! How can I help?".into()),
                StreamEvent::Usage(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 8,
                    ..Default::default()
                }),
                StreamEvent::MessageStop,
            ])))
        }
        fn model(&self) -> &str {
            "mock"
        }
        fn supports_tool_use(&self) -> bool {
            true
        }
        fn max_tokens(&self) -> u32 {
            4096
        }
    }

    let mut conv = ConversationLoop::new(
        Box::new(TextOnlyClient),
        Arc::new(MockFactory),
        ToolRegistry::new(),
        LoopConfig::default(),
        Arc::new(NullEventSink),
        vec![],
        PermissionPolicy::allow_all(),
        PathBuf::from("/tmp"),
    );

    let result = conv.run_turn("Hello").await.unwrap();

    assert_eq!(result.iterations, 1);
    assert_eq!(conv.session().messages.len(), 2); // user + assistant
}
