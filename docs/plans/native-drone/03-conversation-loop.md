# Plan 03: Runtime Conversation Loop

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the core agent loop: send messages to the LLM, execute tool calls, manage context with checkpoint-based compaction, and emit events. This is the engine that drives all drone stages.

**Architecture:** `ConversationLoop` holds an `ApiClient`, `ToolRegistry`, `Session`, and `EventSink`. The `run_turn` method drives the agentic loop. Compaction uses two strategies: `Summarize` (short tasks) and `Checkpoint` (long tasks with artifact storage via EventSink).

**Tech Stack:** tokio (async), serde_json (session serialization)

**Spec:** `docs/specs/native-drone/03-runtime-conversation-loop.md`

**Reference:** `rust/crates/runtime/src/conversation.rs`, `compact.rs` in Claw Code repo

---

### Task 1: Session model

**Files:**
- Create: `src/runtime/src/conversation/session.rs`
- Modify: `src/runtime/src/conversation/mod.rs`

- [ ] **Step 1: Define session types with serialization tests**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
    pub token_estimate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub total_tokens_estimate: u32,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
            total_tokens_estimate: 0,
        }
    }

    pub fn push(&mut self, message: Message) {
        self.total_tokens_estimate += message.token_estimate;
        self.messages.push(message);
    }

    /// Estimate tokens for a string (~4 chars per token for English)
    pub fn estimate_tokens(text: &str) -> u32 {
        (text.len() as u32 / 4).max(1)
    }
}
```

Add `uuid = { version = "1", features = ["v4"] }` to Cargo.toml.

Tests: session creation, push message, token estimation, JSON roundtrip.

- [ ] **Step 2: Run tests, commit**

Run: `cd src/runtime && cargo test conversation::session`

```bash
git add src/runtime/ Cargo.lock
git commit -m "add session model with token estimation"
```

---

### Task 2: Conversation loop core

**Files:**
- Create: `src/runtime/src/conversation/loop_core.rs`
- Modify: `src/runtime/src/conversation/mod.rs`

- [ ] **Step 1: Define ConversationLoop and LoopConfig**

**Important: Two Role enums exist.** `api::types::Role` has `{ User, Assistant }` (wire format). `conversation::session::Role` has `{ System, User, Assistant, Tool }` (internal). The `build_request()` method in step 2 MUST translate:
- `Session::Role::System` messages → `ApiRequest.system` blocks (NOT in messages array)
- `Session::Role::User` → `api::types::Message { role: api::Role::User, content: [...] }`
- `Session::Role::Assistant` → `api::types::Message { role: api::Role::Assistant, content: [...] }`
- `Session::Role::Tool` → `api::types::Message { role: api::Role::User, content: [ToolResult { ... }] }` (Anthropic puts tool results in user messages)

Also, `session::ContentBlock` uses field name `output` in `ToolResult`, while `api::types::ContentBlock` uses `content`. The `build_request` method translates between them.

```rust
use std::sync::Arc;
use crate::api::{ApiClient, ApiClientFactory, ApiRequest, StreamEvent};
use crate::event::EventSink;
use crate::tools::ToolRegistry;
use crate::permission::PermissionPolicy;
use super::session::{Session, Message, ContentBlock, Role};

pub struct LoopConfig {
    pub max_iterations: u32,
    pub max_context_tokens: u32,
    pub compaction_strategy: CompactionStrategy,
    pub checkpoint_on_compaction: bool,
}

pub enum CompactionStrategy {
    Summarize { preserve_recent: u32 },
    Checkpoint { preserve_recent: u32 },
}

pub struct TurnResult {
    pub iterations: u32,
    pub compacted: bool,
    pub usage: crate::api::TokenUsage,
}

pub struct ConversationLoop {
    api_client: Box<dyn ApiClient>,
    api_client_factory: Arc<dyn ApiClientFactory>,
    tool_registry: ToolRegistry,
    session: Session,
    config: LoopConfig,
    event_sink: Arc<dyn EventSink>,
    system_prompt: Vec<String>,
    permission_policy: PermissionPolicy,
}
```

- [ ] **Step 2: Implement run_turn — the agentic loop**

The core method. Pseudocode structure:

```rust
impl ConversationLoop {
    pub async fn run_turn(&mut self, task: &str) -> anyhow::Result<TurnResult> {
        // Push user message
        self.session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text { text: task.to_string() }],
            token_estimate: Session::estimate_tokens(task),
        });

        self.event_sink.emit(RuntimeEvent::TurnStart { task: task.to_string() });

        let mut iterations = 0;
        let mut compacted = false;

        for _ in 0..self.config.max_iterations {
            iterations += 1;

            // Check context pressure
            if self.session.total_tokens_estimate > self.config.max_context_tokens {
                self.compact().await?;
                compacted = true;
            }

            // Build API request
            let request = self.build_request();

            // Start heartbeat
            let heartbeat_sink = self.event_sink.clone();
            let heartbeat = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    heartbeat_sink.emit(RuntimeEvent::Heartbeat);
                }
            });

            // Stream response
            let mut stream = self.api_client.stream(request).await?;
            let (assistant_msg, tool_calls) = self.consume_stream(&mut stream).await?;
            heartbeat.abort();

            // Push assistant message
            self.session.push(assistant_msg);

            // If no tool calls, turn is complete
            if tool_calls.is_empty() {
                break;
            }

            // Execute each tool call
            for tc in &tool_calls {
                self.event_sink.emit(RuntimeEvent::ToolUseStart {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    input: tc.input.clone(),
                });

                let start = std::time::Instant::now();
                let result = self.tool_registry.execute(&tc.name, tc.input.clone(), &ctx).await;
                let duration_ms = start.elapsed().as_millis() as u64;

                self.event_sink.emit(RuntimeEvent::ToolUseEnd {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    result: result.clone(),
                    duration_ms,
                });

                // Push tool result to session
                self.session.push(Message {
                    role: Role::Tool,
                    blocks: vec![ContentBlock::ToolResult {
                        tool_use_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        output: result.output,
                        is_error: result.is_error,
                    }],
                    token_estimate: Session::estimate_tokens(&result.output),
                });
            }
        }

        self.event_sink.emit(RuntimeEvent::TurnEnd { iterations, total_usage: Default::default() });
        Ok(TurnResult { iterations, compacted })
    }
}
```

Implement `build_request()` — translates session messages + system prompt into `ApiRequest`.

Implement `consume_stream()` — reads `StreamEvent`s, accumulates text deltas and tool use blocks, returns constructed `Message` and list of tool calls.

- [ ] **Step 3: Write tests with mock ApiClient and tools**

Create a `MockApiClient` that returns predetermined responses. Create a `MockTool` that records calls. Test:
- Simple turn: user message → text response → turn ends
- Tool use turn: user message → tool call → tool result → text response
- Max iterations reached
- Multiple tool calls in one response

- [ ] **Step 4: Run tests**

Run: `cd src/runtime && cargo test conversation::loop_core`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/runtime/
git commit -m "implement core conversation loop with tool execution"
```

---

### Task 3: Context compaction

**Files:**
- Create: `src/runtime/src/conversation/compaction.rs`

- [ ] **Step 1: Implement Summarize strategy with tests**

```rust
impl ConversationLoop {
    async fn compact(&mut self) -> anyhow::Result<()> {
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

    async fn compact_summarize(&mut self, preserve_recent: u32) -> anyhow::Result<()> {
        let n = preserve_recent as usize;
        if self.session.messages.len() <= n {
            return Ok(()); // Nothing to compact
        }

        let to_summarize = &self.session.messages[..self.session.messages.len() - n];
        let summary = self.generate_summary(to_summarize).await?;

        let recent = self.session.messages.split_off(self.session.messages.len() - n);
        self.session.messages.clear();
        self.session.messages.push(Message {
            role: Role::System,
            blocks: vec![ContentBlock::Text { text: summary }],
            token_estimate: Session::estimate_tokens(&summary),
        });
        self.session.messages.extend(recent);
        self.session.recalculate_tokens();
        Ok(())
    }
}
```

`generate_summary` makes a short API call: "Summarize this conversation in ~200 tokens" with the messages to summarize.

Test: verify messages are truncated correctly, summary injected, recent messages preserved.

- [ ] **Step 2: Implement Checkpoint strategy**

Extends Summarize by calling `event_sink.on_checkpoint()` before summarizing:

```rust
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
            blocks: vec![ContentBlock::Text { text: checkpoint_msg.clone() }],
            token_estimate: Session::estimate_tokens(&checkpoint_msg),
        },
    );
    self.session.recalculate_tokens();
    Ok(())
}
```

Add `on_checkpoint` to `EventSink` trait:
```rust
pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
    async fn on_checkpoint(&self, session: &Session) -> CheckpointContext;
}

pub struct CheckpointContext {
    pub artifact_id: Option<String>,
    pub task_state: String,
    pub additional_context: String,
}
```

Test: verify checkpoint context is injected, summary includes artifact reference.

- [ ] **Step 3: Run tests, buckify, verify build**

Run: `cd src/runtime && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 4: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "add context compaction with summarize and checkpoint strategies"
```

---

### Task 4: Permission policy

**Files:**
- Modify: `src/runtime/src/permission.rs`

- [ ] **Step 1: Implement permission policy with tests**

```rust
use std::collections::HashMap;
use crate::tools::PermissionLevel;

#[derive(Debug, Clone)]
pub enum PermissionMode {
    AllowAll,
    DenyUnknown,
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub overrides: HashMap<String, PermissionLevel>,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            mode: PermissionMode::AllowAll,
            overrides: HashMap::new(),
        }
    }

    pub fn is_allowed(&self, tool_name: &str, tool_permission: PermissionLevel) -> bool {
        if let Some(override_level) = self.overrides.get(tool_name) {
            return *override_level >= tool_permission;
        }
        matches!(self.mode, PermissionMode::AllowAll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all_permits_everything() {
        let policy = PermissionPolicy::allow_all();
        assert!(policy.is_allowed("bash", PermissionLevel::FullAccess));
    }

    #[test]
    fn test_deny_unknown_blocks_unregistered() {
        let policy = PermissionPolicy {
            mode: PermissionMode::DenyUnknown,
            overrides: HashMap::new(),
        };
        assert!(!policy.is_allowed("bash", PermissionLevel::FullAccess));
    }

    #[test]
    fn test_override_allows_specific_tool() {
        let mut policy = PermissionPolicy {
            mode: PermissionMode::DenyUnknown,
            overrides: HashMap::new(),
        };
        policy.overrides.insert("bash".into(), PermissionLevel::FullAccess);
        assert!(policy.is_allowed("bash", PermissionLevel::FullAccess));
    }
}
```

- [ ] **Step 2: Run tests, commit**

```bash
git add src/runtime/
git commit -m "add permission policy for tool access control"
```

---

### Task 5: Sub-agent spawning

**Files:**
- Create: `src/runtime/src/tools/agent.rs`
- Modify: `src/runtime/src/tools/mod.rs`

- [ ] **Step 1: Implement agent tool**

`AgentTool`:
- Input: `{ "task": string, "tools": optional [string], "max_iterations": optional int }`
- Creates a child `ConversationLoop` with:
  - Fresh session
  - Scoped tool registry (if `tools` specified)
  - Shared event sink
  - Reduced max_iterations
- Runs `child.run_turn(task)`
- Returns the final text from the child's last assistant message
- The parent context gets only task + result, not the full sub-conversation

This requires `ConversationLoop` to expose a builder or factory method. The `ApiClientFactory` is used to create the child's API client.

- [ ] **Step 2: Write tests**

Test with mock API client that returns a simple text response. Verify the parent gets only the result text.

- [ ] **Step 3: Register agent tool, run all tests, commit**

Run: `cd src/runtime && cargo test`

```bash
git add src/runtime/
git commit -m "add sub-agent spawning tool"
```

---

### Task 6: Integration test — full conversation turn

**Files:**
- Create: `src/runtime/tests/integration.rs` (or inline in conversation module)

- [ ] **Step 1: Write end-to-end test with mock provider**

Create a mock `ApiClient` that simulates a multi-turn conversation:
1. User says "Create a file called test.txt with 'hello'"
2. Assistant calls `write_file` tool
3. Tool returns success
4. Assistant says "Done, I created test.txt"

Verify:
- Session has 4 messages (user, assistant+tool_use, tool_result, assistant+text)
- File was actually written to temp workspace
- Events were emitted (TurnStart, ToolUseStart, ToolUseEnd, TurnEnd)

- [ ] **Step 2: Run test, commit**

Run: `cd src/runtime && cargo test`

```bash
git add src/runtime/
git commit -m "add integration test for full conversation turn"
```
