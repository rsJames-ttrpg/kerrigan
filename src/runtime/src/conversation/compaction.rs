use crate::api::{ApiRequest, StreamEvent, SystemBlock, TokenUsage};
use crate::event::RuntimeEvent;

use super::loop_core::{CompactionStrategy, ConversationLoop};
use super::session::{ContentBlock, Message, Role, Session};

use tokio_stream::StreamExt;

impl ConversationLoop {
    /// Compact the session to reduce token count
    pub(super) async fn compact(&mut self) -> anyhow::Result<()> {
        let tokens_before = self.session.total_tokens_estimate;
        self.event_sink.emit(RuntimeEvent::CompactionTriggered {
            reason: "context pressure".into(),
            tokens_before,
        });

        match &self.config.compaction_strategy {
            CompactionStrategy::Summarize { preserve_recent } => {
                self.compact_summarize(*preserve_recent).await?;
            }
            CompactionStrategy::Checkpoint { preserve_recent } => {
                self.compact_checkpoint(*preserve_recent).await?;
            }
        }
        Ok(())
    }

    /// Summarize old messages, keeping only the most recent N
    async fn compact_summarize(&mut self, preserve_recent: u32) -> anyhow::Result<()> {
        let n = preserve_recent as usize;
        if self.session.messages.len() <= n {
            return Ok(()); // Nothing to compact
        }

        let split_point = self.session.messages.len() - n;
        let to_summarize = &self.session.messages[..split_point];
        let summary = self.generate_summary(to_summarize).await?;

        let recent = self.session.messages.split_off(split_point);
        self.session.messages.clear();
        self.session.messages.push(Message {
            role: Role::System,
            blocks: vec![ContentBlock::Text {
                text: summary.clone(),
            }],
            token_estimate: Session::estimate_tokens(&summary),
        });
        self.session.messages.extend(recent);
        self.session.recalculate_tokens();
        Ok(())
    }

    /// Checkpoint strategy: store artifact via EventSink, then summarize
    async fn compact_checkpoint(&mut self, preserve_recent: u32) -> anyhow::Result<()> {
        // Call EventSink for checkpoint (drone stores artifact)
        let checkpoint_ctx = self.event_sink.on_checkpoint(&self.session).await;

        if let Some(artifact_id) = &checkpoint_ctx.artifact_id {
            self.event_sink.emit(RuntimeEvent::CheckpointCreated {
                artifact_id: artifact_id.clone(),
            });
        }

        // Summarize old messages
        self.compact_summarize(preserve_recent).await?;

        // Inject checkpoint context after summary
        let checkpoint_msg = format!(
            "Previous work checkpointed as artifact {}.\n{}\n{}",
            checkpoint_ctx.artifact_id.as_deref().unwrap_or("unknown"),
            checkpoint_ctx.task_state,
            checkpoint_ctx.additional_context,
        );
        self.session.messages.insert(
            1, // After summary, before recent messages
            Message {
                role: Role::System,
                blocks: vec![ContentBlock::Text {
                    text: checkpoint_msg.clone(),
                }],
                token_estimate: Session::estimate_tokens(&checkpoint_msg),
            },
        );
        self.session.recalculate_tokens();
        Ok(())
    }

    /// Generate a summary of messages by making a short API call
    async fn generate_summary(&self, messages: &[Message]) -> anyhow::Result<String> {
        // Build a compact text representation of messages to summarize
        let mut conversation_text = String::new();
        for msg in messages {
            let role_str = match msg.role {
                Role::System => "System",
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
            };
            for block in &msg.blocks {
                match block {
                    ContentBlock::Text { text } => {
                        conversation_text.push_str(&format!("{role_str}: {text}\n"));
                    }
                    ContentBlock::ToolUse { name, .. } => {
                        conversation_text.push_str(&format!("{role_str}: [called {name}]\n"));
                    }
                    ContentBlock::ToolResult {
                        tool_name, output, ..
                    } => {
                        // Truncate long tool outputs in summary
                        let truncated = if output.len() > 200 {
                            format!("{}...", &output[..200])
                        } else {
                            output.clone()
                        };
                        conversation_text
                            .push_str(&format!("{role_str} ({tool_name}): {truncated}\n"));
                    }
                }
            }
        }

        let summary_request = ApiRequest {
            system: vec![SystemBlock {
                text: "Summarize this conversation concisely in ~200 tokens. \
                       Focus on key decisions, results, and current state."
                    .into(),
            }],
            messages: vec![crate::api::types::Message {
                role: crate::api::types::Role::User,
                content: vec![crate::api::types::ContentBlock::Text {
                    text: conversation_text,
                }],
            }],
            tools: vec![],
            max_tokens: 300,
            temperature: Some(0.0),
        };

        let mut stream = self.api_client.stream(summary_request).await?;
        let mut summary_parts: Vec<String> = Vec::new();

        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::TextDelta(text) => {
                    summary_parts.push(text);
                }
                StreamEvent::MessageStop => break,
                StreamEvent::Error(e) => {
                    return Err(anyhow::anyhow!("Summary generation failed: {e:?}"));
                }
                _ => {}
            }
        }

        Ok(summary_parts.join(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::error::ApiError;
    use crate::api::{ApiClient, ApiClientFactory, EventStream};
    use crate::event::{CheckpointContext, EventSink, NullEventSink};
    use crate::permission::PermissionPolicy;
    use crate::tools::ToolRegistry;
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use super::super::loop_core::LoopConfig;

    // Mock that returns summary text for first call, then panics
    struct SummaryMockApiClient {
        summary_text: String,
        call_count: Mutex<u32>,
    }

    impl SummaryMockApiClient {
        fn new(summary: &str) -> Self {
            Self {
                summary_text: summary.to_string(),
                call_count: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl ApiClient for SummaryMockApiClient {
        async fn stream(&self, _request: ApiRequest) -> Result<EventStream, crate::api::ApiError> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let text = self.summary_text.clone();
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

    struct MockFactory;
    impl ApiClientFactory for MockFactory {
        fn create(&self) -> Box<dyn ApiClient> {
            Box::new(SummaryMockApiClient::new("default"))
        }
    }

    fn make_conv(api_client: Box<dyn ApiClient>) -> ConversationLoop {
        ConversationLoop::new(
            api_client,
            Arc::new(MockFactory),
            ToolRegistry::new(),
            LoopConfig {
                max_context_tokens: 100,
                compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 2 },
                ..LoopConfig::default()
            },
            Arc::new(NullEventSink),
            vec![],
            PermissionPolicy::allow_all(),
            PathBuf::from("/tmp"),
        )
    }

    #[tokio::test]
    async fn test_compact_summarize_preserves_recent() {
        let client = SummaryMockApiClient::new("Summary of earlier conversation.");
        let mut conv = make_conv(Box::new(client));

        // Add 5 messages
        for i in 0..5 {
            conv.session.push(Message {
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: format!("message {i}"),
                }],
                token_estimate: 10,
            });
        }

        conv.compact_summarize(2).await.unwrap();

        // Should have: 1 summary + 2 recent = 3 messages
        assert_eq!(conv.session.messages.len(), 3);
        assert_eq!(conv.session.messages[0].role, Role::System);
        match &conv.session.messages[0].blocks[0] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "Summary of earlier conversation.");
            }
            _ => panic!("expected text"),
        }

        // Recent messages preserved
        match &conv.session.messages[1].blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "message 3"),
            _ => panic!("expected text"),
        }
        match &conv.session.messages[2].blocks[0] {
            ContentBlock::Text { text } => assert_eq!(text, "message 4"),
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn test_compact_summarize_nothing_to_compact() {
        let client = SummaryMockApiClient::new("should not be called");
        let mut conv = make_conv(Box::new(client));

        // Add only 2 messages (equal to preserve_recent)
        conv.session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text { text: "a".into() }],
            token_estimate: 1,
        });
        conv.session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text { text: "b".into() }],
            token_estimate: 1,
        });

        conv.compact_summarize(2).await.unwrap();

        // Nothing changed
        assert_eq!(conv.session.messages.len(), 2);
    }

    // Event sink that records checkpoint calls
    struct CheckpointRecorder {
        checkpoint_called: Mutex<bool>,
        context: CheckpointContext,
    }

    impl CheckpointRecorder {
        fn new(ctx: CheckpointContext) -> Self {
            Self {
                checkpoint_called: Mutex::new(false),
                context: ctx,
            }
        }
    }

    #[async_trait]
    impl EventSink for CheckpointRecorder {
        fn emit(&self, _event: RuntimeEvent) {}

        async fn on_checkpoint(&self, _session: &Session) -> CheckpointContext {
            *self.checkpoint_called.lock().unwrap() = true;
            CheckpointContext {
                artifact_id: self.context.artifact_id.clone(),
                task_state: self.context.task_state.clone(),
                additional_context: self.context.additional_context.clone(),
            }
        }
    }

    #[tokio::test]
    async fn test_compact_checkpoint_injects_context() {
        let client = SummaryMockApiClient::new("Summary text.");
        let recorder = Arc::new(CheckpointRecorder::new(CheckpointContext {
            artifact_id: Some("art-123".into()),
            task_state: "Implementing feature X".into(),
            additional_context: "3 of 5 files done".into(),
        }));

        let mut conv = ConversationLoop::new(
            Box::new(client),
            Arc::new(MockFactory),
            ToolRegistry::new(),
            LoopConfig {
                max_context_tokens: 100,
                compaction_strategy: CompactionStrategy::Checkpoint { preserve_recent: 2 },
                ..LoopConfig::default()
            },
            recorder.clone(),
            vec![],
            PermissionPolicy::allow_all(),
            PathBuf::from("/tmp"),
        );

        // Add 5 messages
        for i in 0..5 {
            conv.session.push(Message {
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: format!("msg {i}"),
                }],
                token_estimate: 10,
            });
        }

        conv.compact_checkpoint(2).await.unwrap();

        // Checkpoint was called
        assert!(*recorder.checkpoint_called.lock().unwrap());

        // Should have: summary + checkpoint_context + 2 recent = 4 messages
        assert_eq!(conv.session.messages.len(), 4);
        assert_eq!(conv.session.messages[0].role, Role::System); // summary
        assert_eq!(conv.session.messages[1].role, Role::System); // checkpoint context

        match &conv.session.messages[1].blocks[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("art-123"));
                assert!(text.contains("Implementing feature X"));
                assert!(text.contains("3 of 5 files done"));
            }
            _ => panic!("expected text"),
        }
    }

    #[tokio::test]
    async fn test_compact_recalculates_tokens() {
        let client = SummaryMockApiClient::new("Short summary.");
        let mut conv = make_conv(Box::new(client));

        for _ in 0..10 {
            conv.session.push(Message {
                role: Role::User,
                blocks: vec![ContentBlock::Text {
                    text: "a long message that takes many tokens".into(),
                }],
                token_estimate: 50,
            });
        }
        assert_eq!(conv.session.total_tokens_estimate, 500);

        conv.compact_summarize(2).await.unwrap();

        // Tokens should be recalculated and much lower
        assert!(conv.session.total_tokens_estimate < 500);
    }
}
