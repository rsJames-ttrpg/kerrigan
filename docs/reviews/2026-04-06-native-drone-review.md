# Native Drone Comprehensive Spec Compliance Review

**Date:** 2026-04-06
**Reviewer:** Automated drone review
**Scope:** All native drone specs (00-06) plus orchestrator integration spec

---

## Executive Summary

| Spec | Title | Compliance | Critical Gaps |
|------|-------|-----------|---------------|
| 00 | Overview & Architecture | Partial | Checkpoint artifacts not stored in Overseer; PR creation bypasses GitWorkflow |
| 01 | Runtime API Client | Full | Minor: retry count generalization, no Retry-After header parsing |
| 02 | Runtime Tool System | Partial | No namespace sandboxing; git tool lacks policy enforcement; tool cache missing |
| 03 | Runtime Conversation Loop | Partial | No PromptBuilder; tool filtering unused; no sub-agent spawning method |
| 04 | Drone Pipeline | Partial | `orchestrated.rs` not implemented; ArtifactStored is stub; no default-branch guard |
| 05 | Config and Prompts | Partial | Tool cache unimplemented; no sub-agent prompt; environment section minimal |
| 06 | Queen Integration | Partial | 5 DroneEvent variants defined but never emitted; checkpoint store is synthetic |

**Overall:** The implementation covers the core architecture faithfully — crate structure, conversation loop, provider abstraction, tool registry, event bridge, and config hierarchy are all present and well-tested. The primary gaps are in advanced features (orchestrator integration, namespace sandboxing, tool caching, sub-agent prompt scoping) and in wiring between layers (GitWorkflow not connected to main execute path, DroneEvent variants defined but unused).

---

## Spec 00: Overview & Architecture

**Compliance: Partial**

### Crate Structure

Both required crates are present and correctly located:

- `src/runtime/` exists as a `rust_library` target (`//src/runtime:runtime`) in Buck2, with no dependencies on `drone-sdk`, Queen, or Overseer — fully generic as specified.
- `src/drones/native/` exists as a `rust_binary` target (`//src/drones/native:native-drone`) in Buck2.

Dependency direction is correct: `native-drone`'s `Cargo.toml` and BUCK both list `runtime` and `drone-sdk` as dependencies. The `runtime` BUCK file has no reference to either `drone-sdk` or `native-drone`. The inversion constraint is satisfied.

Both crates have corresponding `rust_test` targets (`native-drone-test` and `runtime-test`), and both have substantial test suites.

One minor deviation: the spec's architecture diagram shows `runtime` as a library with no knowledge of drones/Queen/Overseer, which holds — but `runtime/Cargo.toml` includes `tokio` with `test-util` feature unconditionally (not in dev-dependencies). This is a minor dependency hygiene issue, not a structural violation.

### Six Problems Addressed

| # | Problem | Spec Requirement | Implementation Status | Notes |
|---|---------|-----------------|----------------------|-------|
| 1 | Black box agent loop | Native conversation loop with full tool control | ✅ | `ConversationLoop` implements a full agentic loop with explicit tool dispatch via typed `ToolRegistry` |
| 2 | Brittle observability | Structured `RuntimeEvent` stream via `EventSink` trait | ✅ | `RuntimeEvent` enum covers all specified variants. `DroneEventBridge` maps to typed `DroneMessage` events. Heartbeats on 30s timer |
| 3 | No context management | Checkpoint-based compaction | ✅ | Both `Summarize` and `Checkpoint` strategies implemented. However, `on_checkpoint` synthesizes an ID without calling Overseer MCP — partial gap |
| 4 | Git workflow by prayer | Enforced git workflow via `GitWorkflow` | ✅ | Force push denied, operation allow-lists, protected paths, branch name enforcement. Note: `drone.rs::create_pr_if_needed` bypasses policy |
| 5 | Vendor lock-in | Multi-provider `ApiClient` trait | ⚠️ | `Anthropic` and `OpenAiCompat` providers exist. No named Ollama variant — works via `OpenAiCompat` but implicit |
| 6 | Distribution burden | No embedded CLI binary | ✅ | Pure Rust binary with no vendored executables |

### Gaps

- **Checkpoint artifacts not stored in Overseer**: `DroneEventBridge::on_checkpoint` serializes the session to bytes and returns a synthetic `artifact_id` string (`checkpoint-{run_id}-{timestamp}`) but does not call the Overseer MCP `store_artifact` tool.
- **No named Ollama provider**: The spec explicitly targets Ollama for local model testing. `OpenAiCompat` will work but requires operator knowledge.
- **PR creation in `drone.rs` bypasses `GitWorkflow`**: `create_pr_if_needed` calls `gh pr create` directly without going through the `GitWorkflow` policy layer.
- **`GitWorkflow` not wired into the main `drone.rs::execute` path**: The loop's built-in `GitTool` is a separate implementation from `GitWorkflow` that does not enforce stage-based policies. `GitWorkflow` is used only in the orchestrator executor.

---

## Spec 01: Runtime API Client

**Compliance: Full**

### Provider Abstraction

The `ApiClient` trait in `src/runtime/src/api/mod.rs` matches the spec exactly — `stream`, `model`, `supports_tool_use`, and `max_tokens` methods are all present. Both specified providers are implemented:

- `AnthropicClient` in `anthropic.rs` — targets `https://api.anthropic.com/v1/messages` with `x-api-key` auth
- `OpenAiCompatClient` in `openai_compat.rs` — configurable `base_url`, optional `Bearer` auth

`ProviderConfig` enum matches the spec definition verbatim, including `base_url: Option<String>` override for the Anthropic variant. A `create_client` factory function and a `DefaultApiClientFactory` struct are provided — the factory is a reasonable addition for sub-agent spawning.

No Ollama-specific provider exists, but the spec lists it only as an example of an OpenAI-compatible endpoint. The `OpenAiCompatClient` covers it fully.

### Streaming Support

Streaming is fully implemented via `EventStream = Pin<Box<dyn Stream<Item = StreamEvent> + Send>>`. Both providers set `stream: true` and parse SSE frames through the shared `SseParser` in `sse.rs`. All `StreamEvent` variants from the spec are present: `TextDelta`, `ToolUse`, `Usage`, `MessageStop`, `Error`.

### Token Tracking

`TokenUsage` matches the spec exactly with `input_tokens`, `output_tokens`, `cache_read_tokens`, and `cache_creation_tokens` (all `u32`). Anthropic token collection merges tokens from `message_start` and `message_delta`. OpenAI tracking reads from the usage object in the final chunk.

### Retry & Rate Limiting

`RetryingClient` in `retry.rs` is a decorator wrapping any `ApiClient`. The retry policy aligns closely with the spec:

- **Rate limit (429):** Retried with exponential backoff or `retry_after` duration. Respects `max_retries`.
- **Network errors:** Retried after a fixed 5s delay.
- **Server errors (5xx):** Retried after a fixed 10s delay.
- **Stream interruptions:** Retried after a 2s delay.
- **Auth/model errors:** Fail immediately.

### Gaps

- `async_trait` is used instead of `trait_variant::make(Send)` from the spec. Functionally equivalent.
- Network errors and stream interruptions retry up to `max_retries` times rather than exactly once as stated in the spec's prose. Deliberate generalization.
- No `Retry-After` HTTP header parsing — the `retry_after` value is extracted from the JSON body only.

---

## Spec 02: Runtime Tool System

**Compliance: Partial**

### Tool Registry

The `ToolRegistry` in `registry.rs` closely matches the spec. The `Tool` trait signature matches exactly. The registry stores tools in a single `HashMap<String, Arc<dyn Tool>>` rather than the spec's three-map structure, which is a pragmatic simplification.

The `ToolContext` struct differs from spec: the spec lists `cache: Arc<ToolCache>` and `sandbox_config: SandboxConfig` fields; the implementation has `tool_registry: Arc<ToolRegistry>` and `agent_depth: u32` instead. No `ToolCache` or `SandboxConfig` type exists.

### Built-in Tools

| Tool | Spec Required | Implemented | Sandboxed | Notes |
|------|--------------|-------------|-----------|-------|
| `bash` | Yes | Yes | Partial | Path escape check on `working_dir`; no Linux namespace isolation |
| `read_file` | Yes | Yes | Yes | Workspace path check via `validate_path` |
| `write_file` | Yes | Yes | Yes | Workspace path check, creates parent dirs |
| `edit_file` | Yes | Yes | Yes | Exact string match with uniqueness enforcement |
| `glob_search` | Yes | Yes | Yes | Workspace-scoped; sorts by mtime |
| `grep_search` | Yes | Yes | Yes | Workspace-scoped; regex; context lines |
| `git` | Yes | Yes | Partial | All operations implemented; no branch-naming policy, no force-push guard |
| `test_runner` | Yes | Yes | Partial | `cargo test` parsing; missing `location` and `duration_ms` fields |
| `agent` | Yes | Yes | Yes | Depth cap at 3; tool scoping via `scoped()` |

### MCP Integration

Fully implemented and exceeds spec. `McpClient` supports both `stdio` and `http` transports. Tools registered with `mcp__{server}__{tool}` namespacing. Graceful shutdown implemented. Minor deviation: `Http` variant omits `headers` field for auth.

### External Binary Tools

Implemented in `external.rs`. JSON-on-stdin/stdout protocol matches spec. Missing: `embedded` binary feature (include_bytes extraction), `input_schema_path`, `output_format`/`output_template`, and per-tool `permission` field.

### Allow/Deny Lists

The `definitions(allowed, denied)` filtering method is implemented exactly as spec shows. `PermissionPolicy` provides complementary per-tool permission gate but is not wired into `ToolRegistry.execute()`.

### Gaps

- No Linux namespace sandboxing for `bash`
- Git tool lacks policy enforcement (no `GitWorkflow` in tools layer)
- `ToolContext` missing `cache` and `sandbox_config` from spec
- `AgentRequest.files` field absent
- `TestResult` missing `duration_ms` and `TestFailure.location`
- External tool `embedded`, `input_schema_path`, `output_format`/`output_template` not implemented
- MCP `Http` transport missing `headers`

---

## Spec 03: Runtime Conversation Loop

**Compliance: Partial**

### Conversation Loop

The turn-based loop is implemented faithfully in `loop_core.rs`. The `run_turn` method pushes the user message, iterates up to `max_iterations`, calls the API client via `stream()`, collects tool calls, dispatches each sequentially, and continues until no tool calls remain.

One structural deviation: the spec defines a `prompt_builder: PromptBuilder` field, but the implementation uses a plain `system_prompt: Vec<String>`. No `PromptBuilder` type exists in the runtime crate. The spec's `PromptBuilder::for_sub_agent()` capability is absent.

Tool definitions are passed to the API with empty allowed/denied slices (`definitions(&[], &[])`) — no stage-based filtering is applied at the call site.

### Stop Conditions

- **Max iterations** — implemented correctly with configurable limit.
- **End turn (no tool calls)** — implemented correctly.
- **Timeout** — not implemented. No wall-clock timeout guards the total turn duration.

### Event Emission

All event variants specified in the spec are defined in `event.rs` and match exactly. Events are emitted synchronously inline. Sub-agent event tagging with agent ID is not implemented.

### Checkpoint & Compaction

Both `Summarize` and `Checkpoint` strategies are correctly implemented:
- Summarize: splits messages, generates summary via short API call, replaces old messages.
- Checkpoint: delegates to `event_sink.on_checkpoint`, injects checkpoint context message.

The spec field `checkpoint_on_compaction: bool` is replaced by the `CompactionStrategy` enum, which is cleaner.

### Session Management

`Session` matches the spec model closely with all required fields. Token estimation uses the ~4 chars/token heuristic. Session derives `Serialize`/`Deserialize` for checkpoint storage.

### Gaps

- No `PromptBuilder` — system prompt is a `Vec<String>` with no stage-scoped or sub-agent-scoped prompt construction
- Tool definitions not filtered by stage allowlist
- No `spawn_sub_agent` method on `ConversationLoop`
- Sub-agent events not tagged with agent ID
- `RuntimeEvent::Error` defined but never emitted (errors propagate as `anyhow::Result`)
- `config.rs` is empty — no config loading or runtime config types implemented

---

## Spec 04: Drone Pipeline

**Compliance: Partial**

### Stage State Machine

All 6 stages are implemented (`Spec`, `Plan`, `Implement`, `Review`, `Evolve`, `Freeform`) with `Stage::resolve()` matching the spec's fallback-to-Freeform logic. `StageConfig` matches field-for-field. Default configs per stage are present and largely match spec intent.

The implementation adds `allowed_operations: Option<Vec<GitOperationKind>>` to `StageGitConfig` — an improvement over the spec giving typed per-operation allow-listing.

### Health Checks

All spec-mandated checks are implemented faithfully: base checks (cargo, rustc, git) for all stages, with stage-specific additions (build, tests, gh for implement; build, gh for review). Creep check is optional.

**Gap:** Custom health checks from `drone.toml` are not loaded in `checks_for_stage()`. Optional check warning injection into system prompt not visible in `health.rs`.

### Orchestrator

Plan parsing, DAG scheduling, and parallel execution are all implemented and well-tested:
- **Plan Parser:** Fully matches spec format with checkbox items, `Files:` and `Depends:` sub-lines.
- **Scheduler:** DAG validation via Kahn's algorithm detects cycles and unknown dependencies.
- **Executor:** `tokio::spawn` + `select_all` for parallel execution up to `max_parallel`.

**Critical Gap:** `orchestrated.rs` is **not implemented**. The `run_orchestrated()` function, test-fix loop, summarisation sub-loop, and `DroneOutput` assembly described in the orchestrator integration spec are entirely absent. The orchestrator module exists but is not wired into the drone's Implement stage execution path.

### Git Workflow Enforcement

| Policy | Spec Required | Implemented | Notes |
|--------|---------------|-------------|-------|
| Branch naming | Enforced from config | ✅ | `BranchNameMismatch` error |
| Force push denial | Rejected unconditionally | ✅ | Always returns `ForcePushDenied` |
| No default branch commits | Reject on main/master | ❌ | Not implemented |
| Protected paths | Reject writes to matching globs | Partial | Checked at commit time, not at file write time |
| Commit message validation | Non-empty, reasonable length | ❌ | Not implemented |
| PR creation | Via `gh pr create` | ✅ | Working |
| Operation allow-lists | Per-stage allow-listing | ✅ | Typed `GitOperationKind` enum |
| Git serialization | Mutex-serialized atomic commits | ✅ | `Mutex<()>` in `GitWorkflow` |

### Exit Conditions

| Condition | Spec Required | Implemented | Notes |
|-----------|---------------|-------------|-------|
| `FileCreated { glob }` | ✅ | ✅ | Uses `globset` + `ignore::WalkBuilder` |
| `TestsPassing` | ✅ | ✅ | Runs `cargo test`, checks exit code |
| `PrCreated` | ✅ | ✅ | Runs `gh pr view --json url` |
| `ArtifactStored { kind }` | ✅ | Stub | Returns `met: false` with `"requires MCP check"` |
| `Custom(String)` | ✅ | ✅ | Runs via `sh -c` |

### Gaps

1. **`orchestrated.rs` not implemented** — largest gap; orchestrator exists but is not wired into drone
2. **`ArtifactStored` exit condition is a stub**
3. **No default-branch commit guard**
4. **Commit message validation absent**
5. **Protected paths checked at commit time, not file write time**
6. **Evolve stage exit conditions missing** (spec says "issues created")

---

## Spec 05: Config and Prompts

**Compliance: Partial**

### Config Hierarchy

The four-layer merge chain is correctly implemented in `resolve.rs`: compiled defaults → drone.toml → job spec config → stage defaults. Override precedence is sound.

One deviation: the spec lists `timeout_secs` and `system_prompt` as job-spec-overrideable fields, but neither is implemented as a job-spec override in `resolve.rs`.

### drone.toml Structure

`DroneConfig` in `config.rs` faithfully mirrors the spec's TOML schema with all sections present and default values matching exactly. The implementation adds `[environment]` and `[[health_checks]]` sections not in the spec — reasonable extensions.

**Gap:** `DRONE_CONFIG` env var discovery and `/etc/kerrigan/drone.toml` fallback path are not implemented; callers must resolve the path externally.

### Cache Strategy

Bare repo caching is correctly implemented: blake3 URL hash, `git clone --bare`, `git fetch origin`, `git worktree add/remove`.

**Gap:** Tool cache is entirely unimplemented. No `ToolCache` struct, no input hashing, no `.result` file read/write, no LRU eviction. The `tool_cache` bool and `max_size_mb` config fields are dead config.

### Prompt Construction

`prompt.rs` correctly implements `PromptSection` and `PromptBuilder` with exact priority values from the spec. `build_within_budget` drops lowest priority sections first using char/4 token estimate.

**Gaps:**
- Sub-agent prompt reduction (`for_subagent`) not implemented
- Environment section only includes cwd and date, omitting git branch, model name, tools summary

---

## Spec 06: Queen Integration

**Compliance: Partial**

### Event Bridge

`DroneEventBridge` correctly maps all explicitly specified `RuntimeEvent` variants to `DroneMessage` types matching the spec pseudocode. Uses `mpsc::UnboundedSender<DroneMessage>` instead of `QueenChannel` — reasonable decoupling.

### DroneEvent Variants

| Variant | Spec Required | Implemented | Emitted | Notes |
|---------|--------------|-------------|---------|-------|
| `ToolUse` | Yes | Yes | Yes | `tokens_used` hardcoded to 0 |
| `Checkpoint` | Yes | Yes | Yes | `tokens_before`/`tokens_after` hardcoded to 0 |
| `TaskStarted` | Yes | Yes | Yes | `task_id` hardcoded to `"turn"` |
| `TaskCompleted` | Yes | Yes | **No** | Defined but never emitted |
| `StageTransition` | Yes | Yes | **No** | Defined but never emitted |
| `SubAgentSpawned` | Yes | Yes | **No** | Defined but never emitted |
| `SubAgentCompleted` | Yes | Yes | **No** | Defined but never emitted |
| `GitCommit` | Yes | Yes | **No** | Defined but never emitted |
| `GitPrCreated` | Yes | Yes | Yes | Emitted by `drone.rs` on PR creation |
| `TestResults` | Yes | Yes | Yes | Repurposed for health check results |
| `TokenUsage` | Yes | Yes | Yes | Fully implemented |

### DroneRunner Lifecycle

`NativeDrone` implements `DroneRunner` with all three lifecycle methods:
- **setup:** Validates job, loads config, resolves stage, clones repo, configures git creds. Missing: MCP server connection, embedded tool extraction.
- **execute:** Health checks, builds tools, builds prompt, runs `ConversationLoop`, drains events, checks exit conditions, handles PR creation. Missing: orchestrator dispatch path.
- **teardown:** Worktree cleanup and home directory removal. Missing: MCP server disconnection.

### Config & Secrets

Job config parsing is well-implemented with nested object flattening. Secrets are correctly excluded from persisted state. Git credential helper injected via shell function.

### Wire Format

JSON-line protocol correctly implemented with `#[serde(tag = "type", content = "payload")]`. Backward compatibility verified by tests.

### Gaps

- 5 DroneEvent variants defined but never emitted (`TaskCompleted`, `StageTransition`, `SubAgentSpawned`, `SubAgentCompleted`, `GitCommit`)
- `CheckpointStore` abstraction not used; checkpoint artifacts are synthetic
- `stage` field absent from `DroneEventBridge`
- `dirty_count` missing from `GitState`
- MCP server connection/disconnection not implemented in setup/teardown
- Embedded tool binary extraction not implemented
- `on_checkpoint` discards serialized snapshot bytes
- `GitRefs` omits `commits: Vec<String>` field

---

## Cross-cutting Concerns

### Test Inventory

| Module | Test Count | Notes |
|--------|-----------|-------|
| **native-drone** | | |
| `drone.rs` | 13 | Covers setup, teardown, config parsing, event bridge integration; no tests for `execute()` |
| `resolve.rs` | 13 | Good coverage of URL/branch/config resolution logic |
| `prompt.rs` | 12 | Good coverage of prompt building logic |
| `pipeline.rs` | 12 | Covers stage detection and serialization |
| `health.rs` | 12 | Good coverage including async health checks |
| `git_workflow.rs` | 11 | Covers permission enforcement; integration test uses real repo |
| `exit_conditions.rs` | 8 | Covers file/output-based exit detection |
| `orchestrator/scheduler.rs` | 12 | Good coverage of DAG scheduling logic |
| `orchestrator/plan_parser.rs` | 9 | Good coverage of markdown plan parsing |
| `orchestrator/executor.rs` | 3 | Minimal — cycle detection, plan-to-scheduler, sub-agent config only |
| `event_bridge.rs` | 10 | Covers event translation |
| `config.rs` | 7 | Covers config loading and env override |
| `cache.rs` | 5 | Covers path naming and checkout (async) |
| **runtime** | | |
| `tools/file_ops.rs` | 18 | Best coverage in the codebase |
| `tools/git.rs` | 12 | Good coverage of parse/build functions |
| `api/anthropic.rs` | 10 | Covers SSE translation, request building |
| `api/sse.rs` | 9 | Good coverage of SSE parsing |
| `tools/external.rs` | 9 | Covers config, serialization, async execution |
| `tools/mcp.rs` | 8 | Serialization only — no tests for `connect()`, `call_tool()`, `shutdown()` |
| `api/openai_compat.rs` | 8 | Good coverage of OpenAI translation |
| `conversation/session.rs` | 8 | Covers session data model and serialization |
| `api/retry.rs` | 7 | Good coverage of retry logic |
| `conversation/loop_core.rs` | 6 | Covers turn execution and tool dispatch |
| `tools/test_runner.rs` | 6 | Covers cargo output parsing |
| `tools/bash.rs` | 5 | Covers command execution, working dir |
| `tools/registry.rs` | 5 | Covers registration and lookup |
| `api/types.rs` | 5 | Covers content block serialization |
| `permission.rs` | 4 | Covers permission level ordering |
| `conversation/compaction.rs` | 4 | Covers summarize and checkpoint compaction |
| `conversation/integration_test.rs` | 2 | End-to-end multi-turn + tool use |
| `tools/agent.rs` | 2 | Minimal — only sub-agent spawn tests |

**Total tests:** ~237 (117 in native-drone, ~120 in runtime)

### Coverage Gaps

- `NativeDrone::execute()` — the core drone execution path has no test coverage
- `Orchestrator::run()` — actual task execution loop is untested
- `McpClient::connect()`, `call_tool()`, `shutdown()` — all live network/process paths untested
- `runtime::api::create_client()` factory function — untested
- `runtime::tools::default_registry()` — untested

### Error Handling Concerns

- `src/drones/native/src/cache.rs:59` — `path.parent().unwrap()` in `git_clone_bare()`. If the computed bare repo path has no parent, this panics. Extremely unlikely but should return an error.
- `src/runtime/src/api/retry.rs:51` — `last_error.unwrap()` after exhausting retries. Logically sound (loop always sets `last_error`) but relies on implicit invariant. Should use `expect()` with a descriptive message.

All other `unwrap()`/`expect()` calls are inside `#[cfg(test)]` blocks.

### TODO/FIXME/HACK Markers

No `TODO`, `FIXME`, `HACK`, `XXX`, or `UNIMPLEMENTED` markers found in either `src/drones/native/src/` or `src/runtime/src/`. Implementation is clean with deferred work tracked externally.

---

## Overall Assessment

### Compliance Summary

| Spec | Compliance | Score |
|------|-----------|-------|
| 00 — Overview & Architecture | Partial | 8/10 |
| 01 — Runtime API Client | Full | 10/10 |
| 02 — Runtime Tool System | Partial | 6/10 |
| 03 — Runtime Conversation Loop | Partial | 7/10 |
| 04 — Drone Pipeline | Partial | 6/10 |
| 05 — Config and Prompts | Partial | 7/10 |
| 06 — Queen Integration | Partial | 6/10 |

### Critical Gaps That Block Production Use

1. **Orchestrator integration (`orchestrated.rs`) not implemented** — The Implement stage cannot execute structured plans with parallel task DAGs. This is the single largest missing feature.
2. **Checkpoint artifacts not stored in Overseer** — Context recovery after compaction is broken; checkpoint IDs are synthetic and point to nothing.
3. **`ArtifactStored` exit condition is a stub** — Cannot verify artifact storage as a completion criterion.
4. **`GitWorkflow` not wired to main execute path** — Stage-based git policy enforcement only works through the (unimplemented) orchestrator, not through the main `ConversationLoop` `GitTool`.
5. **No `NativeDrone::execute()` test coverage** — The primary execution path is entirely untested.

### Recommended Follow-up Work

1. **Implement `orchestrated.rs`** — Wire orchestrator into Implement stage with test-fix loop and summarisation
2. **Implement real checkpoint storage** — Call Overseer MCP `store_artifact` from `on_checkpoint`
3. **Wire `GitWorkflow` into `GitTool`** — Or replace `GitTool` with a `GitWorkflow`-backed tool for policy enforcement
4. **Implement Linux namespace sandboxing for bash** — At minimum filesystem isolation
5. **Implement tool cache** — LRU-bounded result caching with input hashing
6. **Add `NativeDrone::execute()` integration tests** — Mock API client for deterministic testing
7. **Emit all defined DroneEvent variants** — `TaskCompleted`, `StageTransition`, `SubAgentSpawned`, `SubAgentCompleted`, `GitCommit`
8. **Add sub-agent prompt scoping** — Implement `PromptBuilder::for_subagent()`
9. **Add MCP Http transport headers** — Enable auth for HTTP MCP servers
10. **Fix `unwrap()` calls** — Replace with proper error returns in `cache.rs:59` and `retry.rs:51`
