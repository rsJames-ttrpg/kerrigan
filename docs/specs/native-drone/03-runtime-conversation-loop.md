# Runtime: Conversation Loop

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

The core agent loop: send messages to the LLM, execute tool calls, manage context, emit events. Handles compaction with checkpoint support so long-running tasks don't hit context limits.

## Core Types

```rust
pub struct ConversationLoop {
    api_client: Box<dyn ApiClient>,
    api_client_factory: Arc<dyn ApiClientFactory>,  // for spawning sub-agents
    tool_registry: ToolRegistry,
    session: Session,
    config: LoopConfig,
    event_sink: Arc<dyn EventSink>,
    prompt_builder: PromptBuilder,
}

pub struct LoopConfig {
    pub max_iterations: u32,
    pub max_context_tokens: u32,
    pub compaction_strategy: CompactionStrategy,
    pub checkpoint_on_compaction: bool,
}

pub struct TurnResult {
    pub messages: Vec<Message>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub usage: TokenUsage,
    pub iterations: u32,
    pub compacted: bool,
}
```

## The Agent Loop

`run_turn(task: &str)` drives a single turn of the agent:

```
1. Push user message (task) to session
2. Loop (up to max_iterations):
   a. Check context pressure
      - If over threshold: run compaction (with checkpoint if configured)
   b. Build ApiRequest:
      - System prompt from prompt_builder
      - All session messages
      - Tool definitions from registry (filtered by stage allowlist)
   c. Start heartbeat task (emit Heartbeat every 30s during API call)
   d. Stream response from API client
      - Emit TextDelta events as they arrive
      - Collect assistant message (text blocks + tool use blocks)
   e. Stop heartbeat
   f. Push assistant message to session
   g. If no tool calls → break (turn complete)
   h. For each tool call:
      - Emit ToolUseStart event
      - Check permission policy
      - Execute tool via registry
      - Emit ToolUseEnd event with result + duration
      - Push ToolResult message to session
   i. Continue loop (LLM sees tool results)
3. Emit TurnEnd event
4. Return TurnResult
```

## Session Model

```rust
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub total_tokens_estimate: u32,
}

pub struct Message {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
    pub token_estimate: u32,
}

pub enum Role { System, User, Assistant, Tool }

pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, tool_name: String, output: String, is_error: bool },
}
```

Token estimation uses a fast character-based heuristic (~4 chars per token for English, adjusted per provider). Updated on every message insertion. Used for compaction decisions only — actual usage comes from API responses.

Sessions serialize to JSON for checkpoint storage.

## Context Compaction

```rust
pub enum CompactionStrategy {
    Summarize { preserve_recent: u32 },
    Checkpoint { preserve_recent: u32 },
}
```

### Summarize Strategy

Basic compaction for short-lived tasks:
1. Take all messages except the most recent N
2. Summarize them into a single text block (via a short API call with a "summarize this conversation" prompt)
3. Replace old messages with the summary as a System message
4. Keep recent N messages verbatim

### Checkpoint Strategy

For long-running implement stages — the primary mode:

```
1. Context tokens exceed threshold
2. Serialize full session to JSON
3. Call event_sink.on_checkpoint(session):
   - Drone stores session as Overseer artifact
   - Drone captures git state (branch, HEAD, dirty files)
   - Drone captures task progress (completed/remaining)
   - Returns CheckpointContext with artifact_id + state summary
4. Generate a brief recap of old messages (short API call, ~200 tokens target — not a full summary, just enough for the agent to know what happened without re-reading the artifact)
5. Rebuild session:
   - System prompt (unchanged)
   - Checkpoint context message:
     "Previous work checkpointed as artifact {id}.
      Git: branch {name}, HEAD {sha}.
      Completed: {tasks}.
      Remaining: {tasks}."
   - Summary of old conversation
   - Recent N messages (verbatim)
6. Continue loop with reduced context
```

### EventSink Checkpoint Interface

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

The runtime doesn't know how checkpoints are stored — it delegates to the event sink (implemented by the drone). This keeps the runtime generic.

## Event Stream

```rust
pub enum RuntimeEvent {
    TurnStart { task: String },
    TextDelta(String),
    ToolUseStart { id: String, name: String, input: serde_json::Value },
    ToolUseEnd { id: String, name: String, result: ToolResult, duration_ms: u64 },
    Usage(TokenUsage),
    Heartbeat,
    CompactionTriggered { reason: String, tokens_before: u32 },
    CheckpointCreated { artifact_id: String },
    TurnEnd { iterations: u32, total_usage: TokenUsage },
    Error(String),
}
```

Events are emitted synchronously during the loop. The event sink implementation decides whether to buffer, forward, or drop them. The drone's event bridge maps these to `DroneMessage` variants for Queen.

## Permission Policy

```rust
pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub overrides: HashMap<String, PermissionLevel>,
}

pub enum PermissionMode {
    AllowAll,
    DenyUnknown,
}
```

In drone context, the default is `AllowAll` (the drone is trusted). Stage configs can deny specific tools (e.g., review stage denies `write_file`). The policy is checked before every tool execution.

## Sub-agent Spawning

When the `agent` tool is called, the conversation loop creates a child:

```rust
pub fn spawn_sub_agent(&self, request: AgentRequest) -> SubAgentHandle {
    let child_loop = ConversationLoop {
        api_client: self.api_client_factory.create(),  // factory creates new client instances
        tool_registry: self.tool_registry.scoped(&request.tools),
        session: Session::new(),
        config: LoopConfig {
            max_iterations: request.max_iterations.unwrap_or(self.config.max_iterations / 2),
            ..self.config.clone()
        },
        event_sink: self.event_sink.clone(),
        prompt_builder: self.prompt_builder.for_sub_agent(&request),
    };

    // Run in a spawned tokio task
    SubAgentHandle::spawn(child_loop, request.task)
}
```

Sub-agents share the workspace, cache, and event sink. They get their own session and (optionally) scoped tool set. Events from sub-agents are tagged with the agent ID so Queen can track them.
