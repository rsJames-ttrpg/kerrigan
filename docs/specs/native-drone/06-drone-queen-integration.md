# Drone: Queen Integration

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

Bridge between the runtime's event stream and Queen's JSON-line stdio protocol. Extends the drone-sdk protocol with rich event types while maintaining backward compatibility with the existing Claude drone.

## Event Bridge

The drone implements `EventSink` to map runtime events to `DroneMessage` variants:

```rust
pub struct DroneEventBridge {
    channel: QueenChannel,
    stage: Stage,
    run_id: String,
    workspace: PathBuf,
    checkpoint_store: Arc<CheckpointStore>,
}

impl EventSink for DroneEventBridge {
    fn emit(&self, event: RuntimeEvent) {
        let msg = match event {
            RuntimeEvent::TurnStart { task } =>
                DroneMessage::Event(DroneEvent::TaskStarted { task_id: "turn".into(), description: task }),
            RuntimeEvent::ToolUseStart { name, .. } =>
                DroneMessage::Progress { status: "tool_use".into(), detail: name },
            RuntimeEvent::ToolUseEnd { id: _, name, result: _, duration_ms } =>
                DroneMessage::Event(DroneEvent::ToolUse { name, duration_ms, tokens_used: 0 }),
            RuntimeEvent::Usage(usage) =>
                DroneMessage::Event(DroneEvent::TokenUsage {
                    input: usage.input_tokens,
                    output: usage.output_tokens,
                    cache_read: usage.cache_read_tokens,
                    total_cost_usd: None,
                }),
            RuntimeEvent::Heartbeat =>
                DroneMessage::Progress { status: "heartbeat".into(), detail: "alive".into() },
            RuntimeEvent::CompactionTriggered { reason, tokens_before } =>
                DroneMessage::Progress { status: "compacting".into(), detail: reason },
            RuntimeEvent::CheckpointCreated { artifact_id } =>
                DroneMessage::Event(DroneEvent::Checkpoint {
                    artifact_id,
                    tokens_before: 0,
                    tokens_after: 0,
                }),
            RuntimeEvent::Error(e) =>
                DroneMessage::Progress { status: "error".into(), detail: e },
            _ => return,
        };
        self.channel.send(msg);
    }

    async fn on_checkpoint(&self, session: &Session) -> CheckpointContext {
        let snapshot = serde_json::to_vec(session).unwrap();
        let artifact_id = self.checkpoint_store
            .store(&self.run_id, "session-checkpoint", &snapshot)
            .await;
        let git_state = git_current_state(&self.workspace);

        CheckpointContext {
            artifact_id: Some(artifact_id),
            task_state: self.task_summary(),
            additional_context: format!(
                "Branch: {}, HEAD: {}, Dirty: {}",
                git_state.branch, git_state.head, git_state.dirty_count
            ),
        }
    }
}
```

## Extended Protocol

New `DroneMessage::Event` variant alongside existing message types:

```rust
pub enum DroneMessage {
    // Existing (backward compat with Claude drone)
    Progress { status: String, detail: String },
    AuthRequest { url: String, message: String },
    Result(DroneOutput),
    Error { message: String },

    // New — rich structured events
    Event(DroneEvent),
}

pub enum DroneEvent {
    ToolUse { name: String, duration_ms: u64, tokens_used: u32 },
    Checkpoint { artifact_id: String, tokens_before: u32, tokens_after: u32 },
    TaskStarted { task_id: String, description: String },
    TaskCompleted { task_id: String, description: String },
    StageTransition { from: String, to: String },
    SubAgentSpawned { agent_id: String, task: String },
    SubAgentCompleted { agent_id: String, success: bool },
    GitCommit { sha: String, message: String },
    GitPrCreated { url: String },
    TestResults { passed: u32, failed: u32, skipped: u32 },
    TokenUsage { input: u32, output: u32, cache_read: u32, total_cost_usd: Option<f64> },
}
```

### Wire Format

JSON-line, same as today. The `Event` variant serializes as:
```json
{"Event":{"ToolUse":{"name":"edit_file","duration_ms":42,"tokens_used":0}}}
```

Queen's supervisor deserializes `DroneMessage` — existing variants work unchanged, new `Event` variant is handled if recognized, ignored if not. This allows rolling upgrades (new drone, old Queen still works).

### Queen-side Handling

Queen's supervisor processes events:

```rust
match message {
    DroneMessage::Progress { .. } => {
        // Existing: update last_activity, log
        handle.last_activity = Instant::now();
    }
    DroneMessage::Event(event) => {
        handle.last_activity = Instant::now();
        // Forward to Overseer as structured data
        overseer.forward_event(&run_id, &event).await;
        // Trigger notifications if configured
        notifier.on_event(&run_id, &event).await;
    }
    DroneMessage::Result(output) => { /* existing completion handling */ }
    DroneMessage::Error { message } => { /* existing error handling */ }
    DroneMessage::AuthRequest { .. } => { /* existing auth flow */ }
}
```

## Health & Liveness

No changes to Queen's health check model — it's already based on `last_activity` timestamps:

- Every `DroneMessage` (Progress, Event, etc.) resets `last_activity`
- Heartbeat events from the runtime (every 30s during API calls) prevent false stall alerts
- Timeout and stall thresholds work identically

The richer event stream means Queen naturally gets more frequent liveness signals. A native drone emitting `ToolUse` events every few seconds is clearly alive — no stderr scraping needed.

## DroneRunner Implementation

```rust
pub struct NativeDrone;

impl DroneRunner for NativeDrone {
    async fn setup(&self, job: &JobSpec) -> DroneEnvironment {
        // 1. Read drone.toml
        // 2. Resolve config (drone.toml + job spec + stage defaults)
        // 3. Setup isolated home (/tmp/drone-{id}/)
        // 4. Clone/fetch repo (using cache if available)
        // 5. Configure git credentials
        // 6. Extract embedded tool binaries (if any)
        // 7. Connect to MCP servers
        // 8. Build tool registry
        // 9. Return environment
    }

    async fn execute(&self, env: &DroneEnvironment, channel: &QueenChannel) -> DroneOutput {
        // 1. Create EventSink bridge (channel → DroneMessage)
        // 2. Build PromptBuilder for resolved stage
        // 3. Create ConversationLoop with API client, tools, config
        // 4. If structured stage (implement):
        //    a. Create Orchestrator
        //    b. Parse plan into tasks
        //    c. Run orchestrator (manages sub-agents, checkpoints)
        // 5. If freeform:
        //    a. Run single ConversationLoop with task as input
        // 6. Check exit conditions
        // 7. Handle git finalization (PR creation if required)
        // 8. Collect artifacts (conversation, session, git refs)
        // 9. Return DroneOutput
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        // 1. Disconnect MCP servers
        // 2. Remove git worktree
        // 3. Clean up /tmp/drone-{id}/
        // 4. (Cache persists across runs)
    }
}
```

## DroneOutput

Same structure as today — backward compatible:

```rust
pub struct DroneOutput {
    pub exit_code: i32,
    pub conversation: Option<String>,    // serialized session JSON
    pub artifacts: Vec<String>,          // artifact IDs stored during run
    pub git_refs: GitRefs,
    pub session_jsonl_gz: Option<String>, // not applicable for native drone
}

pub struct GitRefs {
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    pub pr_required: bool,
    pub commits: Vec<String>,           // new: list of commit SHAs
}
```

The `session_jsonl_gz` field is replaced by checkpoint artifacts stored during the run (referenced in `artifacts`). Queen's completion handler works with both formats.
