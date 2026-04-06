---
title: Native Drone
slug: native-drone
description: Native drone binary — Rust-native agent runner with TOML config, pipeline stages, orchestrated parallel execution, git workflow enforcement, health checks, and repo caching
lastmod: 2026-04-06
tags: [drone, agent, orchestrator, pipeline, git, config]
sources:
  - path: src/drones/native/src/main.rs
    hash: 8cb947a1a1383087c5881c81dccd99802baad3b9c71ce87abadf26a0746fd1b4
  - path: src/drones/native/src/drone.rs
    hash: 63d2d496842b768647cc783385e796cf8c48ebcb81ad8dc0d007b86294d04ab0
  - path: src/drones/native/src/pipeline.rs
    hash: b53aaa3ee2ac7ffbb47f4208157f091a271de031e28c4a4e0f603b93f8f20386
  - path: src/drones/native/src/resolve.rs
    hash: 65405c3f155e0f1d30b1ef990c7997252abd5a9bcef1c0b03ad0a7c4eedcfa89
  - path: src/drones/native/src/cache.rs
    hash: 2561cdcce1bceac046892f36b8005a2f78886aaa33eed6b3669e969e8c36b998
  - path: src/drones/native/src/health.rs
    hash: 69a5c3cd7d8061131037eb064f09b749bfcc9b9420959762d80ad32f4e97192f
  - path: src/drones/native/src/event_bridge.rs
    hash: 72b92bc5c1c0dc94918855169e7488ee0bf0f594059c1f72b49ec6b784eaf4ac
  - path: src/drones/native/src/exit_conditions.rs
    hash: 30cb6a11f66db09930fd4d8fa86bed1098f17d54c12fddba9bc815a39b7ef1bd
  - path: src/drones/native/src/git_workflow.rs
    hash: f0036f6c92ceea1c7957398eb4c3c549ed09ca3fcdc8b760d5c1167e92dd8d1b
  - path: src/drones/native/src/prompt.rs
    hash: 2378c770e95a571739b6a0fe449aa91b848f539d345a225a4fbbbc0d45f78f80
  - path: src/drones/native/src/orchestrator/mod.rs
    hash: 2ba05e48305beb3b2ca440fd07f8ca7a3860f7c2cd924ee281f14de4842eec26
  - path: src/drones/native/src/orchestrator/plan_parser.rs
    hash: 531f3a7daee2a8d64e407ddf85b289a9a20af6927bbb9d0097dde9151718d5f7
  - path: src/drones/native/src/orchestrator/executor.rs
    hash: 58e01ddc995f9547f16b26d2cd18615aa24b667e5edf6b8088a32e9d0b1bf08c
  - path: src/drones/native/src/orchestrator/scheduler.rs
    hash: a51b59a25f9ff2e33e18cfd901cccead4c883d51270c65936edba33f268ca624
  - path: src/drones/native/src/orchestrator/orchestrated.rs
    hash: b8ea51549e365e96d0eac74fa666c600ee06f26c3b1936d40fe4cfcfe6778db9
  - path: src/drones/native/BUCK
    hash: 43df1dd7e13c96b647ff310ecd8821a5d36e544328b7f23b74b8956c30d4a0dd
sections: [overview, lifecycle, configuration, config-resolution, pipeline-stages, health-checks, prompt-builder, git-workflow, exit-conditions, repo-cache, event-bridge, orchestrator, plan-format, build-and-test]
---

# Native Drone

## Overview

The native drone (`src/drones/native/`) is a Rust-native agent runner that executes kerrigan pipeline jobs without depending on the Claude CLI. It implements the `DroneRunner` trait from `drone-sdk` and uses the `runtime` crate's `ConversationLoop` directly to run LLM agent loops with tool use.

Key differences from the Claude drone (`src/drones/claude/base/`):

- **No CLI dependency** — calls the `runtime` crate's API client and conversation loop directly
- **TOML configuration** — `drone.toml` provides full control over provider, runtime, git, tools, cache, orchestrator, and environment settings
- **Multi-layer config resolution** — merges compiled defaults, `drone.toml`, job spec overrides, and stage defaults
- **Task orchestration** — parses structured markdown plans into dependency graphs and executes tasks in parallel with a test-fix loop
- **Git workflow enforcement** — operation allow-lists, protected paths (glob-based), force-push denial, mutex-serialized commits
- **Health checks** — stage-aware pre-execution validation of required tooling
- **Repo caching** — bare git repos with blake3-hashed paths and worktree-based checkouts

Entry point: `drone_sdk::harness::run(drone::NativeDrone)` via `main.rs`. Uses `tokio::main(flavor = "current_thread")`.

Build target: `buck2 build root//src/drones/native:native-drone`

## Lifecycle

The `NativeDrone` struct implements `DroneRunner` with three phases:

### Setup

1. Validates `job_run_id` — alphanumeric, hyphens, underscores only
2. Loads `drone.toml` from `$DRONE_CONFIG_DIR` env var (default: `"."`) via `DroneToml::load()` from `drone-sdk`
3. Parses job config — flattens nested JSON objects with dot notation (e.g., `secrets.github_pat`)
4. Resolves pipeline stage from `config.stage` field (defaults to `Freeform`)
5. Merges config layers via `ResolvedConfig::resolve()`
6. Creates isolated home at `/tmp/drone-{job_run_id}/` with `workspace/` subdirectory
7. Clones/fetches repo via `RepoCache` if `repo_url` is non-empty and `repo_cache` is enabled
8. Configures git credential helper if `secrets.github_pat` is provided
9. Sets environment variables from resolved config
10. Persists state to `drone_state.json` and `config_meta.json` — **secrets are filtered out**

Returns `DroneEnvironment { home, workspace }`.

### Execute

1. Reloads `drone_state.json` and `config_meta.json` from setup phase
2. Re-resolves config (drone.toml + job config + stage)
3. Creates `DroneEventBridge` (mpsc channel translating `RuntimeEvent` to `DroneMessage`)
4. Runs stage-aware health checks — aborts if any required check fails
5. Builds tool registry and system prompt via `PromptBuilder::for_stage()`
6. Reads workspace `CLAUDE.md` for project context
7. Dispatches execution:
   - **Orchestrated** — if `plan_path` is set and stage is `Implement`, parses the plan into tasks and runs `run_orchestrated()`
   - **Single loop** — otherwise runs a single `ConversationLoop::run_turn()` with the job task
8. Drains event bridge messages and forwards to Queen
9. Checks exit conditions (file globs, tests passing, PR created, custom commands)
10. Creates PR if `pr_on_stage_complete` is true for the stage
11. Returns `DroneOutput` with exit code, conversation, git refs, and artifacts

### Teardown

1. Cleans up git worktree via `RepoCache::cleanup_worktree()`
2. Removes the entire drone home directory (`/tmp/drone-{job_run_id}/`)
3. Idempotent — safe to call multiple times

## Configuration

Configuration is loaded via `DroneToml::load()` from the `drone-sdk` crate — the same struct used by the claude drone. All sections have defaults (including `[provider]`, which is `Option` and defaults to `None`).

```toml
[provider]                      # optional section — defaults to None
kind = "anthropic"              # or "openai-compat"
model = "claude-sonnet-4-20250514"
api_key = "sk-..."              # optional, can come from job spec secrets
base_url = "https://..."        # optional

[runtime]
max_tokens = 8192           # per-response token limit
max_iterations = 50         # conversation loop iterations
temperature = 0.7           # optional
timeout_secs = 7200         # 2 hours
compaction_strategy = "checkpoint"  # or "summarize"
compaction_threshold_tokens = 80000
compaction_preserve_recent = 6

[cache]
dir = "/var/cache/kerrigan/drone"
repo_cache = true
tool_cache = true           # parsed but unused
max_size_mb = 2048

[git]
default_branch = "main"
branch_prefix = "kerrigan/"
auto_commit = true
pr_on_complete = true
protected_paths = ["CLAUDE.md", ".buckconfig"]

[git.identity.claude]           # per-drone-type git identity
user_name = "claude-drone"
user_email = "claude-drone@noreply"

[git.identity.native]
user_name = "native-drone"
user_email = "native-drone@noreply"

[setup]
commands = ["./tools/setup-hooks.sh"]  # post-clone setup commands

[prompts]
extra_rules = "Use buck2 build, not cargo build."

[tools]
sandbox = true
allowed = ["read_file", "write_file"]  # empty = all allowed
denied = ["bash"]                       # empty = none denied

[tools.external.creep]
binary = "creep-cli"
args = ["search"]
description = "File indexing search"
permission = "read-only"    # default
output_format = "markdown"  # default
embedded = true
timeout_secs = 10

[mcp.overseer]              # parsed but unused
kind = "http"
url = "http://localhost:3100/mcp"

[environment]
extra_path = ["/usr/local/bin"]  # parsed but unused
[environment.env]
RUST_LOG = "debug"

[orchestrator]
test_command = "cargo test --workspace"  # optional, enables test-fix loop
max_fixup_iterations = 5
max_parallel = 2            # clamped to minimum 1

[[health_checks]]           # custom health checks (parsed but unused currently)
name = "cargo-check"
command = "cargo"
args = ["check"]
required = true
timeout_secs = 60
```

Provider mapping: `kind = "anthropic"` produces `ProviderConfig::Anthropic`; anything else produces `ProviderConfig::OpenAiCompat` (defaults to `http://localhost:11434/v1` for local inference). If `[provider]` is omitted entirely, defaults to Anthropic with `claude-sonnet-4-20250514`.

## Config Resolution

`ResolvedConfig::resolve()` merges four layers (highest priority wins):

1. **Stage defaults** — stage-specific fields like allowed/denied tools, git operations, max turns
2. **Job spec overrides** — operator-provided per-run values from the job config JSON
3. **drone.toml values** — the base configuration file
4. **Compiled defaults** — serde default functions

Job spec keys that are recognized as overrides:

| Key | Overrides |
|-----|-----------|
| `model` | Provider model |
| `secrets.api_key` | Provider API key |
| `max_iterations` | Loop max iterations |
| `max_tokens` | Max tokens per response |
| `temperature` | Sampling temperature |
| `branch` | Git branch name |
| `max_turns` | Stage max turns |
| `env.*` | Environment variables (e.g., `env.RUST_LOG=debug`) |
| `test_command` | Orchestrator test command |
| `max_fixup_iterations` | Orchestrator fix-up limit |
| `max_parallel` | Orchestrator parallelism |

Protected paths are **merged** — stage defaults and `drone.toml` paths are combined, not replaced.

Compaction strategies: `"checkpoint"` (default) or `"summarize"`, both parameterized by `compaction_preserve_recent`.

## Pipeline Stages

Six stages, each with distinct defaults for tools, git, and turn limits:

| Stage | Max Turns | Allowed Tools | Denied Tools | Git Ops | PR on Complete |
|-------|-----------|---------------|--------------|---------|----------------|
| **Spec** | 25 | read, glob, grep, write, edit | bash, git, test, agent | status, diff, log | No |
| **Plan** | 25 | read, glob, grep, write, edit | bash, git, test, agent | status, diff, log | No |
| **Implement** | 100 | all | none | all | Yes |
| **Review** | 25 | read, glob, grep, git, bash | write, edit | status, diff, log, commit, push | No |
| **Evolve** | 25 | read, glob, grep, bash | write, edit, git, test, agent | none | No |
| **Freeform** | 50 | all | none | all | No |

Entry requirements:
- **Plan** requires `spec` artifact
- **Implement** requires `plan` artifact

Exit conditions:
- **Spec**: `docs/specs/*.md` file created + `spec` artifact stored
- **Plan**: `docs/plans/*.md` file created + `plan` artifact stored
- **Implement**: tests passing + PR created
- **Review**: `review` artifact stored
- **Evolve/Freeform**: none

Protected paths: **Implement** protects `CLAUDE.md` by default. Additional paths merge from `drone.toml`.

Stage is resolved from job config `stage` field. Unknown/missing values default to `Freeform`. Serializes as lowercase JSON string (`"implement"`, `"spec"`, etc.).

## Health Checks

Stage-aware pre-execution validation. All stages get base checks; some stages add extras.

**Base checks** (all stages, all required):
- `cargo --version`
- `rustc --version`
- `git --version`

**Implement stage** adds (required):
- `cargo check` (300s timeout)
- `cargo test` (600s timeout)
- `gh --version`

**Review stage** adds (required):
- `cargo check` (300s timeout)
- `gh --version`

**All stages** add (optional):
- `creep-cli --version`

Checks run sequentially. Each check has a configurable timeout. The `HealthReport` tracks pass/fail per check. If any **required** check fails, execute aborts with an error.

## Prompt Builder

`PromptBuilder` assembles a multi-section system prompt with priority-based ordering. Higher priority sections appear first; when a token budget is applied, lowest priority sections are dropped first.

Priority levels used by `for_stage()`:

| Priority | Section | Content |
|----------|---------|---------|
| 255 | identity | Agent identity and behavioral instructions |
| 255 | environment | Working directory, date, stage |
| 200 | mission | Stage-specific `system_prompt` from `StageConfig` |
| 180 | tools | Auto-generated tool guide from `ToolRegistry` |
| 180 | git_rules | Branch, protected paths, no force push |
| 150 | project_context | Workspace `CLAUDE.md` content |
| 150 | task_state | Current task state (for orchestrated sub-agents) |
| 100 | constraints | Denied tools, base rules (no system packages, no scope creep) |
| 50 | checkpoint | Checkpoint reference (lowest priority, dropped first) |

Token estimation: `len / 4` (rough char-to-token ratio). `build_within_budget(max_tokens)` drops lowest-priority sections first until the budget is satisfied.

## Git Workflow

`GitWorkflow` enforces git operation policies per stage. Uses a `Mutex`-based serializer to prevent concurrent git operations.

**Operations** (typed enum `GitOperation`):
- `Status`, `Diff { staged }`, `Log { count }`
- `CreateBranch { name, from }`, `Commit { message, paths }`, `Push { force }`
- `CreatePr { title, body, base }`, `CheckoutFile { path, ref_ }`

**Policy enforcement** (in `execute()`):

1. **Force push always denied** — returns `GitWorkflowError::ForcePushDenied` regardless of allow-list
2. **Operation allow-list** — if `allowed_operations` is `Some(list)`, the operation's `GitOperationKind` must be in the list. `None` means all operations are allowed.
3. **Protected paths** — commit operations check each path against glob matchers compiled from `StageGitConfig.protected_paths`. Returns `GitWorkflowError::ProtectedPath`.
4. **Branch name enforcement** — `CreateBranch` validates against `StageGitConfig.branch_name` if set. Returns `GitWorkflowError::BranchNameMismatch`.

After policy checks pass, the operation is executed via `tokio::process::Command` under the serializer lock. PR creation uses `gh pr create`.

## Exit Conditions

`check_exit_conditions()` evaluates a list of `ExitCondition` variants against the workspace:

| Variant | Check | Implementation |
|---------|-------|----------------|
| `FileCreated { glob }` | Glob match against workspace files | `globset::Glob` + `ignore::WalkBuilder` (respects `.gitignore`) |
| `TestsPassing` | `cargo test` exits 0 | `tokio::process::Command` |
| `PrCreated` | `gh pr view --json url` exits 0 | `tokio::process::Command` |
| `ArtifactStored { kind }` | Always returns `met: false` | Stubbed — "requires MCP check" |
| `Custom(command)` | `sh -c <command>` exits 0 | `tokio::process::Command` |

Each condition produces a `ConditionResult { condition, met, detail }`. The drone's exit code is 0 only if all conditions are met and all tasks succeeded.

## Repo Cache

`RepoCache` manages a persistent cache of bare git repos to avoid full clones on every job.

- **Cache directory**: configurable via `drone.toml` `[cache] dir` (default: `/var/cache/kerrigan/drone`)
- **Path derivation**: `blake3(repo_url)` truncated to 16 hex chars, stored as `{cache_dir}/repos/{hash}.git`
- **Checkout flow**:
  1. If bare repo exists at derived path, `git fetch origin`
  2. Otherwise, `git clone --bare <url> <path>`
  3. `git worktree add <workspace_path> <branch>` from the bare repo
- **Cleanup**: `git worktree remove --force <workspace_path>`, called during teardown
- **Deterministic**: same URL always maps to the same cache path

## Event Bridge

`DroneEventBridge` implements `runtime::event::EventSink` and translates `RuntimeEvent` variants into `DroneMessage` variants sent over an unbounded mpsc channel.

| RuntimeEvent | DroneMessage |
|-------------|--------------|
| `TurnStart { task }` | `Event(TaskStarted { task_id: "turn", description })` |
| `ToolUseStart { name, .. }` | `Progress { status: "tool_use", detail: name }` |
| `ToolUseEnd { name, duration_ms, .. }` | `Event(ToolUse { name, duration_ms, tokens_used: 0 })` |
| `Usage(TokenUsage)` | `Event(TokenUsage { input, output, cache_read, total_cost_usd: None })` |
| `Heartbeat` | `Progress { status: "heartbeat", detail: "alive" }` |
| `CompactionTriggered { reason, .. }` | `Progress { status: "compacting", detail: reason }` |
| `CheckpointCreated { artifact_id }` | `Event(Checkpoint { artifact_id, tokens_before: 0, tokens_after: 0 })` |
| `Error(msg)` | `Progress { status: "error", detail: msg }` |
| `TextDelta`, `TurnEnd` | Ignored (no message sent) |

The `on_checkpoint()` method serializes the `Session` to JSON, captures git state (branch + HEAD), and returns a `CheckpointContext` with an artifact ID of `checkpoint-{run_id}-{timestamp}`.

Note: `tokens_used` in `ToolUse` events is hardcoded to 0 (not tracked per-tool).

## Orchestrator

When `plan_path` is set in the job config and the stage is `Implement`, the native drone switches to orchestrated execution. This system parses a structured plan, builds a dependency graph, executes tasks in parallel via sub-agent conversation loops, and runs an iterative test-fix loop.

### Architecture

- **`parse_plan(markdown)`** — parses structured markdown into `Vec<Task>` (see Plan Format)
- **`TaskScheduler`** — DAG-aware scheduler tracking task state (ready, in-progress, completed, failed). Validates the graph with Kahn's algorithm for cycle detection. Failed tasks block all downstream dependents.
- **`Orchestrator`** — spawns sub-agent `ConversationLoop`s as tokio tasks, respecting `max_parallel` concurrency. Each sub-agent gets `agent_depth = 1`, a summarize compaction strategy, and the full system prompt. On success, commits the task's declared files via `GitWorkflow`.
- **`run_orchestrated(tasks, ctx)`** — top-level function that:
  1. Runs `Orchestrator::run()` to execute all tasks
  2. If `test_command` is configured, enters an iterative test-fix loop (up to `max_fixup_iterations`)
  3. On test failure: summarizes output via a short summariser agent (depth 2), then spawns a fix-up agent (depth 1) with the failure summary and orchestrator results as context
  4. Fix-up agent commits only modified tracked files (avoids staging untracked changes)
  5. Aborts the loop after 2 consecutive agent failures
  6. Returns `OrchestratedResult` with task results, fix-up metadata, and test status

### Configuration

From `[orchestrator]` in `drone.toml`, overridable via job config:

| Field | Default | Description |
|-------|---------|-------------|
| `test_command` | None | Shell command to run tests. If unset, test-fix loop is skipped. |
| `max_fixup_iterations` | 5 | Maximum test-fix iterations before giving up |
| `max_parallel` | 2 | Maximum concurrent sub-agent tasks (clamped to min 1) |

### Fallback

If `plan_path` is provided but the parser extracts zero tasks (malformed or unexpected format), the drone logs a warning, emits a progress message, and falls back to a single conversation loop.

## Plan Format

The plan parser expects structured markdown with this format:

```markdown
- [ ] **task-id**: Description of the task
  - Files: src/foo.rs, src/bar.rs
  - Depends: task-other, task-another
```

Rules:
- Task lines must start with `- [ ] **` followed by the ID in bold, then `**: ` and the description
- `Files:` and `Depends:` are optional, case-insensitive (`files:` and `depends:` also work)
- `Depends: none` or empty means no dependencies
- Multiple dependencies/files are comma-separated
- Surrounding markdown (headings, prose, notes) is ignored
- The resulting task list must form a valid DAG (no cycles, all dependency IDs must exist)

## Build and Test

```bash
# Build
buck2 build root//src/drones/native:native-drone

# Test (137 tests)
cd src/drones/native && cargo test

# Buck2 test
buck2 test root//src/drones/native:native-drone-test
```

Dependencies: `runtime`, `drone-sdk` (internal); `tokio`, `serde`, `serde_json`, `async-trait`, `anyhow`, `tracing`, `tracing-subscriber`, `toml`, `globset`, `ignore`, `thiserror`, `tempfile`, `blake3`, `chrono`, `futures` (third-party).

External CLI tools assumed available at runtime: `git`, `cargo`, `rustc`, `gh` (GitHub CLI), `creep-cli` (optional).
