# Plan 06: Drone Queen Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire everything together: implement `DroneRunner` for the native drone, bridge runtime events to Queen's protocol, extend the protocol with rich event types, and handle the full drone lifecycle (setup → health check → agent loop → exit conditions → teardown).

**Architecture:** `NativeDrone` implements `DroneRunner`. `DroneEventBridge` implements `EventSink` to map runtime events to `DroneMessage` variants. The extended protocol adds `DroneMessage::Event(DroneEvent)` alongside existing variants for backward compatibility.

**Tech Stack:** drone-sdk, runtime, tokio

**Spec:** `docs/specs/native-drone/06-drone-queen-integration.md`

---

### Task 1: Extended protocol types

**Files:**
- Modify: `src/drone-sdk/src/protocol.rs`

- [ ] **Step 1: Add DroneEvent enum to protocol**

In `src/drone-sdk/src/protocol.rs`, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
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

Add `Event(DroneEvent)` variant to the existing `DroneMessage` enum. Ensure the serde tagging is compatible with existing variants (the existing drone only sends `Progress`, `Result`, `Error`, `AuthRequest`).

Tests: serialization roundtrip for new Event variant, verify old variants still deserialize correctly.

- [ ] **Step 2: Run drone-sdk tests**

Run: `cd src/drone-sdk && cargo test`
Expected: all existing + new tests pass

- [ ] **Step 3: Commit**

```bash
git add src/drone-sdk/
git commit -m "extend drone protocol with rich DroneEvent types"
```

---

### Task 2: Event bridge

**Files:**
- Create: `src/drones/native/src/event_bridge.rs`

- [ ] **Step 1: Implement DroneEventBridge**

**Architecture note:** `QueenChannel` is sync (`send(&mut self, &DroneMessage)`) and requires `&mut self`. The `EventSink` trait is `Send + Sync` (used from multiple async contexts). Solution: the bridge holds an `mpsc::UnboundedSender<DroneMessage>`, and a separate forwarding task drains the receiver into the real `QueenChannel`.

```rust
use std::path::PathBuf;
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
        Self { sender, workspace, run_id }
    }
}

impl EventSink for DroneEventBridge {
    fn emit(&self, event: RuntimeEvent) {
        let msg = match event {
            RuntimeEvent::TurnStart { task } => {
                DroneMessage::Event(DroneEvent::TaskStarted {
                    task_id: "turn".into(),
                    description: task,
                })
            }
            RuntimeEvent::ToolUseStart { name, .. } => {
                DroneMessage::Progress(Progress {
                    status: "tool_use".into(),
                    detail: Some(name),
                })
            }
            RuntimeEvent::ToolUseEnd { name, duration_ms, .. } => {
                DroneMessage::Event(DroneEvent::ToolUse {
                    name,
                    duration_ms,
                    tokens_used: 0,
                })
            }
            RuntimeEvent::Usage(usage) => {
                DroneMessage::Event(DroneEvent::TokenUsage {
                    input: usage.input_tokens,
                    output: usage.output_tokens,
                    cache_read: usage.cache_read_tokens,
                    total_cost_usd: None,
                })
            }
            RuntimeEvent::Heartbeat => {
                DroneMessage::Progress(Progress {
                    status: "heartbeat".into(),
                    detail: Some("alive".into()),
                })
            }
            RuntimeEvent::CompactionTriggered { reason, .. } => {
                DroneMessage::Progress(Progress {
                    status: "compacting".into(),
                    detail: Some(reason),
                })
            }
            RuntimeEvent::CheckpointCreated { artifact_id } => {
                DroneMessage::Event(DroneEvent::Checkpoint {
                    artifact_id,
                    tokens_before: 0,
                    tokens_after: 0,
                })
            }
            RuntimeEvent::Error(e) => {
                DroneMessage::Progress(Progress {
                    status: "error".into(),
                    detail: Some(e),
                })
            }
            _ => return,
        };
        let _ = self.sender.send(msg); // non-blocking, fire-and-forget
    }

    async fn on_checkpoint(&self, session: &Session) -> CheckpointContext {
        let snapshot = serde_json::to_vec(session).unwrap_or_default();

        // Signal checkpoint via the channel
        let _ = self.sender.send(DroneMessage::Progress(Progress {
            status: "checkpoint_store".into(),
            detail: Some(format!("{} bytes", snapshot.len())),
        }));

        let git_state = capture_git_state(&self.workspace).await;

        CheckpointContext {
            artifact_id: Some(format!("checkpoint-{}", chrono::Utc::now().timestamp())),
            task_state: String::new(), // filled by orchestrator
            additional_context: format!(
                "Branch: {}, HEAD: {}",
                git_state.branch, git_state.head,
            ),
        }
    }
}

// In execute(), wire up the bridge and forwarding task:
//
// let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
// let bridge = Arc::new(DroneEventBridge::new(tx, env.workspace.clone(), job.job_run_id.clone()));
//
// // Forward messages to QueenChannel on a blocking thread (channel is sync)
// let forward_handle = tokio::task::spawn_blocking(move || {
//     while let Some(msg) = rx.blocking_recv() {
//         if let Err(e) = channel.send(&msg) {
//             tracing::warn!("failed to send to queen: {e}");
//             break;
//         }
//     }
// });

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
```

Tests: verify event mapping produces correct DroneMessage variants. Verify `Progress` uses the wrapped struct form `DroneMessage::Progress(Progress { ... })`, not inline fields.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add event bridge mapping runtime events to Queen protocol"
```

---

### Task 3: Full DroneRunner implementation

**Files:**
- Modify: `src/drones/native/src/drone.rs`
- Modify: `src/drones/native/src/main.rs`

- [ ] **Step 1: Implement setup phase**

Replace the placeholder `NativeDrone` with the real implementation:

```rust
impl DroneRunner for NativeDrone {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        // 1. Validate job run ID (alphanumeric, -, _ only)
        // 2. Read drone.toml (from DRONE_CONFIG env or default)
        // 3. Resolve config (drone.toml + job.config + stage)
        // 4. Create isolated home /tmp/drone-{id}/
        // 5. Setup environment (PATH, env vars from config)
        // 6. Clone/fetch repo via cache manager
        // 7. Configure git credentials (PAT from job secrets)
        // 8. Extract embedded tool binaries (if any)
        // 9. Connect MCP servers
        // 10. Return DroneEnvironment
    }
}
```

Each sub-step calls into existing modules (config.rs, cache.rs, etc.). Read the existing Claude drone's `environment.rs` for patterns on home creation, git credential setup, and job ID validation.

- [ ] **Step 2: Implement execute phase**

**Note:** `DroneEnvironment` has `home: PathBuf` and `workspace: PathBuf` (not `home_dir`/`workspace_dir`). `DroneOutput.conversation` is `serde_json::Value` (not `Option<String>`). Store resolved config and task as separate state (e.g., in a `DroneState` struct created during setup and passed around, or stored in a `OnceCell` on `NativeDrone`).

```rust
async fn execute(&self, env: &DroneEnvironment, channel: &mut QueenChannel) -> anyhow::Result<DroneOutput> {
    // The resolved config and task should be stored during setup
    // (e.g., in a Mutex<Option<DroneState>> on NativeDrone, or loaded from files in env.home)
    let config = load_resolved_config(&env.home)?;
    let stage = &config.stage_config;

    // 1. Create event bridge via mpsc channel
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let event_bridge = Arc::new(DroneEventBridge::new(tx, env.workspace.clone(), "run-id".into()));

    // Spawn forwarding task (QueenChannel is sync, needs blocking thread)
    // NOTE: channel is &mut, so we need to move it into the blocking task.
    // This means all Queen communication goes through the bridge after this point.
    let forward_handle = tokio::task::spawn_blocking(move || {
        while let Some(msg) = rx.blocking_recv() {
            if let Err(e) = channel.send(&msg) {
                tracing::warn!("failed to send to queen: {e}");
                break;
            }
        }
    });

    // 2. Run health checks
    let health_checks = health::checks_for_stage(&stage.stage);
    let report = health::run_health_checks(&health_checks).await;
    if !report.all_required_passed() {
        return Err(anyhow::anyhow!("health checks failed: {}", report.summary()));
    }

    // 3. Build tool registry
    let mut registry = runtime::tools::default_registry();
    // Register MCP tools, external tools from config

    // 4. Build prompt
    let workspace_context = read_claude_md(&env.workspace);
    let prompt = PromptBuilder::for_stage(&stage.stage, stage, &registry, &workspace_context, None, None);

    // 5. Create API client
    let api_client = runtime::api::create_client(&config.provider);

    // 6. Create conversation loop
    let mut conversation = ConversationLoop::new(
        api_client,
        registry,
        config.loop_config.clone(),
        event_bridge.clone(),
        prompt.build(),
    );

    // 7. Run the agent loop with the task
    let task = read_task_description(&env.home);
    let result = conversation.run_turn(&task).await?;

    // 8. Check exit conditions
    let conditions = exit_conditions::check_exit_conditions(&stage.exit_conditions, &env.workspace).await;
    let all_met = conditions.iter().all(|c| c.met);

    // 9. Handle git finalization
    if stage.git.pr_on_stage_complete {
        // Create PR if not already created
    }

    // 10. Build DroneOutput
    // Drop the bridge sender to signal the forwarding task to exit
    drop(event_bridge);
    let _ = forward_handle.await;

    Ok(DroneOutput {
        exit_code: if all_met { 0 } else { 1 },
        conversation: serde_json::to_value(&conversation.session())?,
        artifacts: vec![], // checkpoint artifact IDs
        git_refs: GitRefs {
            branch: stage.git.branch_name.clone(),
            pr_url: None, // filled by PR creation
            pr_required: stage.git.pr_on_stage_complete,
        },
        session_jsonl_gz: None,
    })
}
```

- [ ] **Step 3: Implement teardown phase**

```rust
async fn teardown(&self, env: &DroneEnvironment) {
    // 1. Disconnect MCP servers
    // 2. Cleanup git worktree via cache manager
    // 3. Remove /tmp/drone-{id}/ directory
}
```

- [ ] **Step 4: Verify build**

Run: `cd src/drones/native && cargo check`
Run: `buck2 build root//src/drones/native:native-drone`

- [ ] **Step 5: Commit**

```bash
git add src/drones/native/ Cargo.lock third-party/BUCK
git commit -m "implement full DroneRunner lifecycle for native drone"
```

---

### Task 4: End-to-end smoke test

**Files:**
- Create: `src/drones/native/tests/smoke.rs` (or inline test)

- [ ] **Step 1: Write smoke test with mock Queen channel**

Create a test that:
1. Creates a `NativeDrone`
2. Builds a minimal `JobSpec` with freeform stage
3. Calls setup (with a temp dir as workspace, skip real git clone)
4. Calls execute with a mock channel that collects messages
5. Verifies: health checks ran, events were emitted, DroneOutput has expected fields

This doesn't need a real LLM — use a mock `ApiClient` that returns a simple text response.

- [ ] **Step 2: Run test, commit**

Run: `cd src/drones/native && cargo test`

```bash
git add src/drones/native/
git commit -m "add end-to-end smoke test for native drone lifecycle"
```
