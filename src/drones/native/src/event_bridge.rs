use std::path::PathBuf;

use async_trait::async_trait;
use drone_sdk::protocol::{DroneEvent, DroneMessage, Progress};
use runtime::conversation::session::Session;
use runtime::event::{CheckpointContext, EventSink, RuntimeEvent};

pub struct DroneEventBridge {
    sender: tokio::sync::mpsc::UnboundedSender<DroneMessage>,
    workspace: PathBuf,
    run_id: String,
}

impl DroneEventBridge {
    pub fn new(
        sender: tokio::sync::mpsc::UnboundedSender<DroneMessage>,
        workspace: PathBuf,
        run_id: String,
    ) -> Self {
        Self {
            sender,
            workspace,
            run_id,
        }
    }
}

#[async_trait]
impl EventSink for DroneEventBridge {
    fn emit(&self, event: RuntimeEvent) {
        let msg = match event {
            RuntimeEvent::TurnStart { task } => DroneMessage::Event(DroneEvent::TaskStarted {
                task_id: "turn".into(),
                description: task,
            }),
            RuntimeEvent::ToolUseStart { name, .. } => DroneMessage::Progress(Progress {
                status: "tool_use".into(),
                detail: Some(name),
            }),
            RuntimeEvent::ToolUseEnd {
                name, duration_ms, ..
            } => DroneMessage::Event(DroneEvent::ToolUse {
                name,
                duration_ms,
                tokens_used: 0,
            }),
            RuntimeEvent::Usage(usage) => DroneMessage::Event(DroneEvent::TokenUsage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cache_read: usage.cache_read_tokens,
                total_cost_usd: None,
            }),
            RuntimeEvent::Heartbeat => DroneMessage::Progress(Progress {
                status: "heartbeat".into(),
                detail: Some("alive".into()),
            }),
            RuntimeEvent::CompactionTriggered { reason, .. } => DroneMessage::Progress(Progress {
                status: "compacting".into(),
                detail: Some(reason),
            }),
            RuntimeEvent::CheckpointCreated { artifact_id } => {
                DroneMessage::Event(DroneEvent::Checkpoint {
                    artifact_id,
                    tokens_before: 0,
                    tokens_after: 0,
                })
            }
            RuntimeEvent::Error(e) => DroneMessage::Progress(Progress {
                status: "error".into(),
                detail: Some(e),
            }),
            _ => return,
        };
        let _ = self.sender.send(msg);
    }

    async fn on_checkpoint(&self, session: &Session) -> CheckpointContext {
        let snapshot = serde_json::to_vec(session).unwrap_or_default();

        let _ = self.sender.send(DroneMessage::Progress(Progress {
            status: "checkpoint_store".into(),
            detail: Some(format!("{} bytes", snapshot.len())),
        }));

        let git_state = capture_git_state(&self.workspace).await;

        CheckpointContext {
            artifact_id: Some(format!(
                "checkpoint-{}-{}",
                self.run_id,
                chrono::Utc::now().timestamp()
            )),
            task_state: String::new(),
            additional_context: format!("Branch: {}, HEAD: {}", git_state.branch, git_state.head),
        }
    }
}

struct GitState {
    branch: String,
    head: String,
}

async fn capture_git_state(workspace: &std::path::Path) -> GitState {
    let branch = tokio::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workspace)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let head = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    GitState { branch, head }
}

#[cfg(test)]
mod tests {
    use runtime::api::TokenUsage;
    use runtime::tools::ToolResult;

    use super::*;

    fn make_bridge() -> (
        DroneEventBridge,
        tokio::sync::mpsc::UnboundedReceiver<DroneMessage>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = DroneEventBridge::new(tx, PathBuf::from("/tmp/test"), "run-1".into());
        (bridge, rx)
    }

    #[test]
    fn test_turn_start_maps_to_task_started() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::TurnStart {
            task: "fix the bug".into(),
        });
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Event(DroneEvent::TaskStarted {
                task_id,
                description,
            }) => {
                assert_eq!(task_id, "turn");
                assert_eq!(description, "fix the bug");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_tool_use_start_maps_to_progress() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::ToolUseStart {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({}),
        });
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Progress(p) => {
                assert_eq!(p.status, "tool_use");
                assert_eq!(p.detail.as_deref(), Some("bash"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_tool_use_end_maps_to_event() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::ToolUseEnd {
            id: "t1".into(),
            name: "read".into(),
            result: ToolResult::success("ok".into()),
            duration_ms: 250,
        });
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Event(DroneEvent::ToolUse {
                name,
                duration_ms,
                tokens_used,
            }) => {
                assert_eq!(name, "read");
                assert_eq!(duration_ms, 250);
                assert_eq!(tokens_used, 0);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_usage_maps_to_token_usage_event() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::Usage(TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 20,
            cache_creation_tokens: 0,
        }));
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Event(DroneEvent::TokenUsage {
                input,
                output,
                cache_read,
                total_cost_usd,
            }) => {
                assert_eq!(input, 100);
                assert_eq!(output, 50);
                assert_eq!(cache_read, 20);
                assert!(total_cost_usd.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_heartbeat_maps_to_progress() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::Heartbeat);
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Progress(p) => {
                assert_eq!(p.status, "heartbeat");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_compaction_maps_to_progress() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::CompactionTriggered {
            reason: "token limit".into(),
            tokens_before: 50000,
        });
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Progress(p) => {
                assert_eq!(p.status, "compacting");
                assert_eq!(p.detail.as_deref(), Some("token limit"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_checkpoint_created_maps_to_event() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::CheckpointCreated {
            artifact_id: "chk-42".into(),
        });
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Event(DroneEvent::Checkpoint { artifact_id, .. }) => {
                assert_eq!(artifact_id, "chk-42");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_error_maps_to_progress() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::Error("something broke".into()));
        let msg = rx.try_recv().unwrap();
        match msg {
            DroneMessage::Progress(p) => {
                assert_eq!(p.status, "error");
                assert_eq!(p.detail.as_deref(), Some("something broke"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_text_delta_is_ignored() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::TextDelta("hello".into()));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_turn_end_is_ignored() {
        let (bridge, mut rx) = make_bridge();
        bridge.emit(RuntimeEvent::TurnEnd {
            iterations: 5,
            total_usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
        });
        assert!(rx.try_recv().is_err());
    }
}
