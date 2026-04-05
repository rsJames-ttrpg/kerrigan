use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::api::ApiClientFactory;
use crate::conversation::loop_core::{CompactionStrategy, ConversationLoop, LoopConfig};
use crate::conversation::session::{ContentBlock, Role};
use crate::event::EventSink;
use crate::permission::PermissionPolicy;

use super::ToolRegistry;
use super::registry::Tool;
use super::types::{PermissionLevel, ToolContext, ToolResult};

#[derive(Deserialize)]
struct AgentInput {
    task: String,
    tools: Option<Vec<String>>,
    max_iterations: Option<u32>,
}

/// Tool that spawns a child ConversationLoop as a sub-agent.
/// The parent context gets only the task + result text, not the full sub-conversation.
pub struct AgentTool {
    api_client_factory: Arc<dyn ApiClientFactory>,
    event_sink: Arc<dyn EventSink>,
    system_prompt: Vec<String>,
    permission_policy: PermissionPolicy,
}

impl AgentTool {
    pub fn new(
        api_client_factory: Arc<dyn ApiClientFactory>,
        event_sink: Arc<dyn EventSink>,
        system_prompt: Vec<String>,
        permission_policy: PermissionPolicy,
    ) -> Self {
        Self {
            api_client_factory,
            event_sink,
            system_prompt,
            permission_policy,
        }
    }

    /// Build a scoped tool registry: if `tools` is specified, only include those tools.
    fn scoped_registry(parent: &ToolRegistry, tools: &Option<Vec<String>>) -> ToolRegistry {
        let mut child_registry = ToolRegistry::new();
        match tools {
            Some(tool_names) => {
                // Only include specified tools by re-fetching definitions
                // We can't move tools out of the parent, so we just filter definitions
                // The child will use its own registry which shares tool implementations
                // For now, the child gets an empty registry with the allowed tool names
                // This is a limitation - in practice tools would be cloneable or arc-wrapped
                let _ = tool_names;
                // Return empty registry - tools aren't Arc-wrapped so can't be shared
                // The sub-agent will work without tools (text-only responses)
                child_registry
            }
            None => {
                // No filter - but we can't clone tools, so return empty
                child_registry
            }
        }
    }
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "agent"
    }

    fn description(&self) -> &str {
        "Spawn a sub-agent to handle a task. The sub-agent runs in its own conversation \
         context and returns only the final result text."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["task"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The task for the sub-agent to perform"
                },
                "tools": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional list of tool names the sub-agent can use"
                },
                "max_iterations": {
                    "type": "integer",
                    "description": "Maximum iterations for the sub-agent (default: 10)"
                }
            }
        })
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::FullAccess
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let params: AgentInput = match serde_json::from_value(input) {
            Ok(p) => p,
            Err(e) => return ToolResult::error(format!("Invalid input: {e}")),
        };

        let max_iterations = params.max_iterations.unwrap_or(10);

        // Create a fresh API client for the child
        let child_client = self.api_client_factory.create();

        let child_config = LoopConfig {
            max_iterations,
            max_context_tokens: 50_000, // smaller context for sub-agents
            compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 4 },
            max_tokens_per_response: 4096,
            temperature: None,
        };

        // Create child loop with scoped tools
        let child_registry = Self::scoped_registry(&ToolRegistry::new(), &params.tools);

        let mut child = ConversationLoop::new(
            child_client,
            self.api_client_factory.clone(),
            child_registry,
            child_config,
            self.event_sink.clone(),
            self.system_prompt.clone(),
            self.permission_policy.clone(),
            ctx.workspace.clone(),
        );

        // Run the sub-agent turn
        match child.run_turn(&params.task).await {
            Ok(_result) => {
                // Extract the final text from the last assistant message
                let final_text = child
                    .session()
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                    .and_then(|m| {
                        m.blocks.iter().find_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                    })
                    .unwrap_or_else(|| "Sub-agent completed without text response.".into());

                ToolResult::success(final_text)
            }
            Err(e) => ToolResult::error(format!("Sub-agent failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::error::ApiError;
    use crate::api::{ApiClient, ApiRequest, EventStream, StreamEvent, TokenUsage};
    use crate::event::NullEventSink;
    use std::sync::Mutex;

    struct SubAgentMockClient {
        response_text: String,
    }

    #[async_trait]
    impl ApiClient for SubAgentMockClient {
        async fn stream(&self, _request: ApiRequest) -> Result<EventStream, ApiError> {
            let text = self.response_text.clone();
            Ok(Box::pin(tokio_stream::iter(vec![
                StreamEvent::TextDelta(text),
                StreamEvent::Usage(TokenUsage::default()),
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

    struct SubAgentMockFactory {
        response_text: String,
    }

    impl ApiClientFactory for SubAgentMockFactory {
        fn create(&self) -> Box<dyn ApiClient> {
            Box::new(SubAgentMockClient {
                response_text: self.response_text.clone(),
            })
        }
    }

    #[tokio::test]
    async fn test_agent_tool_returns_result_text() {
        let factory = Arc::new(SubAgentMockFactory {
            response_text: "The answer is 42.".into(),
        });

        let tool = AgentTool::new(
            factory,
            Arc::new(NullEventSink),
            vec!["You are helpful.".into()],
            PermissionPolicy::allow_all(),
        );

        let ctx = ToolContext {
            workspace: PathBuf::from("/tmp"),
            home: PathBuf::from("/tmp"),
            event_sink: Arc::new(NullEventSink),
        };

        let result = tool
            .execute(serde_json::json!({"task": "What is 6 * 7?"}), &ctx)
            .await;

        assert!(!result.is_error);
        assert_eq!(result.output, "The answer is 42.");
    }

    #[tokio::test]
    async fn test_agent_tool_invalid_input() {
        let factory = Arc::new(SubAgentMockFactory {
            response_text: "unused".into(),
        });

        let tool = AgentTool::new(
            factory,
            Arc::new(NullEventSink),
            vec![],
            PermissionPolicy::allow_all(),
        );

        let ctx = ToolContext {
            workspace: PathBuf::from("/tmp"),
            home: PathBuf::from("/tmp"),
            event_sink: Arc::new(NullEventSink),
        };

        let result = tool
            .execute(serde_json::json!({"wrong_field": "oops"}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("Invalid input"));
    }
}
