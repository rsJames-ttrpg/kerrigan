use async_trait::async_trait;

use crate::api::TokenUsage;
use crate::tools::ToolResult;

#[derive(Debug)]
pub enum RuntimeEvent {
    TurnStart {
        task: String,
    },
    TextDelta(String),
    ToolUseStart {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolUseEnd {
        id: String,
        name: String,
        result: ToolResult,
        duration_ms: u64,
    },
    Usage(TokenUsage),
    Heartbeat,
    CompactionTriggered {
        reason: String,
        tokens_before: u32,
    },
    CheckpointCreated {
        artifact_id: String,
    },
    TurnEnd {
        iterations: u32,
        total_usage: TokenUsage,
    },
    Error(String),
}

/// Context returned by checkpoint operations
pub struct CheckpointContext {
    pub artifact_id: Option<String>,
    pub task_state: String,
    pub additional_context: String,
}

impl Default for CheckpointContext {
    fn default() -> Self {
        Self {
            artifact_id: None,
            task_state: String::new(),
            additional_context: String::new(),
        }
    }
}

#[async_trait]
pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);

    /// Called during checkpoint compaction to store session state as an artifact.
    /// Returns context about the checkpoint for injection into the compacted session.
    async fn on_checkpoint(
        &self,
        _session: &crate::conversation::session::Session,
    ) -> CheckpointContext {
        CheckpointContext::default()
    }
}

/// No-op event sink for testing
pub struct NullEventSink;

#[async_trait]
impl EventSink for NullEventSink {
    fn emit(&self, _event: RuntimeEvent) {}
}
