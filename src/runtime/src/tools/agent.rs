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

const MAX_AGENT_DEPTH: u32 = 3;

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
                    "description": "Optional list of tool names the sub-agent can use. If omitted, inherits all parent tools."
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

        if ctx.agent_depth >= MAX_AGENT_DEPTH {
            return ToolResult::error(format!(
                "Maximum sub-agent depth ({MAX_AGENT_DEPTH}) exceeded"
            ));
        }

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

        // Build child registry from parent's tools (shared via Arc)
        let child_registry = match &params.tools {
            Some(tool_names) => {
                let scoped = ctx.tool_registry.scoped(tool_names);
                let missing: Vec<_> = tool_names
                    .iter()
                    .filter(|n| scoped.get(n).is_none())
                    .collect();
                if !missing.is_empty() {
                    return ToolResult::error(format!(
                        "Requested tools not available: {missing:?}"
                    ));
                }
                scoped
            }
            None => ctx.tool_registry.clone_all(),
        };

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
        child.set_agent_depth(ctx.agent_depth + 1);

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
    use std::path::PathBuf;
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
            tool_registry: Arc::new(ToolRegistry::new()),
            agent_depth: 0,
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
            tool_registry: Arc::new(ToolRegistry::new()),
            agent_depth: 0,
        };

        let result = tool
            .execute(serde_json::json!({"wrong_field": "oops"}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("Invalid input"));
    }
}
