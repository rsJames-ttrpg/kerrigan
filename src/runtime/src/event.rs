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

pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
}

/// No-op event sink for testing
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _event: RuntimeEvent) {}
}
