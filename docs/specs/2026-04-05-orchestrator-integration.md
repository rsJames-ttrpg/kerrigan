# Orchestrator Integration: Wiring Parallel Task Execution into the Native Drone

## Summary

Wire the existing orchestrator module (plan parser, DAG scheduler, parallel executor) into the native drone's Implement stage, and add an iterative test-fix loop that runs after all orchestrated tasks complete.

## Trigger and Dispatch

In `NativeDrone::execute()`, after reloading state and resolving config, branch on two conditions:

1. `job_config` contains a `plan_path` key
2. `stage == Stage::Implement`

When both hold, read `workspace.join(plan_path)`, parse it with `parse_plan()`. If tasks are found, call `run_orchestrated()`. If no tasks are found (malformed plan or no checkbox items), fall through to the existing single-loop path with a warning log.

All other stages and non-plan jobs continue through the existing `ConversationLoop::run_turn()` unchanged. The existing single-loop code gets extracted into a `run_single_loop()` function for symmetry.

## Orchestrated Execution Flow

`run_orchestrated()` lives in `src/drones/native/src/orchestrator/orchestrated.rs`. It takes the parsed tasks, resolved config, workspace, event bridge, API client factory, tool registry, system prompt, and git workflow.

### Steps

1. **Build Orchestrator** from parsed tasks, `max_parallel` (from config, default 2), and runtime deps.
2. **Run orchestrator** via `orchestrator.run()`, returning `Vec<TaskResult>`.
3. **Collect summary** of which tasks succeeded/failed, which files each touched, which commits were made.
4. **Enter test-fix loop** (up to `max_fixup_iterations`, default 5):
   - Run `test_command` (resolved from job_config > drone.toml > skip if unset).
   - If tests pass, break.
   - If tests fail, summarise the test output via a summarisation sub-loop.
   - Spawn a fix-up `ConversationLoop` with the summarised test output + orchestrator results summary.
   - After fix-up completes, commit changes via `git_workflow`, loop back to test.
5. **Return** aggregated `TaskResult`s plus fix-up commits as a single `DroneOutput`.

If no `test_command` is configured (neither in job_config nor drone.toml), the test-fix loop is skipped entirely with a warning log.

## Configuration

### `drone.toml` — new `[orchestrator]` section

```toml
[orchestrator]
test_command = "cargo test"
max_fixup_iterations = 5
max_parallel = 2
```

All fields optional. This adds an `OrchestratorSection` to `DroneConfig`, merged into `ResolvedConfig` like other sections.

> **Future consideration:** `test_command` may need to become a list of commands (e.g. `cargo test` + `cargo clippy` + integration tests). For now a single string is sufficient; revisit when multi-command needs arise.

### `job_config` — per-job overrides

- `test_command` — overrides `drone.toml`
- `max_fixup_iterations` — overrides `drone.toml`
- `max_parallel` — overrides `drone.toml`

Resolution order: `job_config` > `drone.toml` > hardcoded defaults. `test_command` has no hardcoded default (skip test-fix loop if unset). `max_fixup_iterations` defaults to 5. `max_parallel` defaults to 2.

## Fix-up Agent Design

### Fix-up ConversationLoop

- **System prompt:** Same as the Implement stage system prompt, so it has full tool access and workspace awareness.
- **User prompt (per iteration):** Built from three pieces:
  1. Summarised test output from the summarisation sub-loop
  2. Orchestrator results summary: for each task, its ID, description, files touched, success/failure
  3. Iteration context: "This is fix-up attempt N of M. Previous attempt did not resolve all test failures."
- **Loop config:** Same `LoopConfig` as task agents (from `sub_agent_config`), using the stage's `max_turns` (100 for Implement).
- **Git:** After each fix-up `run_turn()`, commit via `git_workflow` with message `"fix: test failures (fix-up iteration N)"` before re-running tests.

### Summarisation Sub-loop

A `ConversationLoop` with:
- `max_iterations: 3`
- System prompt: "You are a test output summariser. Extract the failing test names, error messages, and relevant file/line references. Be concise."
- Input: raw test stderr/stdout, truncated to last 50KB to avoid blowing context.

Returns a concise summary the fix-up agent can act on without consuming excessive context.

## Output and Reporting

`run_orchestrated()` returns a `DroneOutput`:

- **exit_code:** 0 if all orchestrator tasks succeeded AND tests pass (immediately or after fix-up). 1 otherwise.
- **conversation:** JSON object containing:
  - `orchestrator_results`: array of task results (id, success, output, commits)
  - `fixup_iterations`: number of fix-up loops that ran
  - `tests_passing`: final boolean
  - `fixup_summaries`: array of summarised test outputs per iteration
- **artifacts:** empty (future: plan file, test reports)
- **git_refs:** branch and PR URL, populated by existing `create_pr_if_needed()` logic in `drone.rs` after `run_orchestrated()` returns.

Existing exit condition checks (`ExitCondition::TestsPassing`, `ExitCondition::PrCreated`) still run after orchestrated execution, validating the final workspace state.

## Files to Create or Modify

- **Create:** `src/drones/native/src/orchestrator/orchestrated.rs` — `run_orchestrated()` function
- **Modify:** `src/drones/native/src/orchestrator/mod.rs` — add `mod orchestrated` and re-export
- **Modify:** `src/drones/native/src/drone.rs` — add dispatch branch, extract `run_single_loop()`
- **Modify:** `src/drones/native/src/config.rs` — add `OrchestratorSection` to `DroneConfig`
- **Modify:** `src/drones/native/src/resolve.rs` — merge orchestrator config into `ResolvedConfig`
