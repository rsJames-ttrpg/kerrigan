# Orchestrator Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing orchestrator module into the native drone's Implement stage so that `plan_path` jobs run parallel task execution followed by an iterative test-fix loop.

**Architecture:** When `plan_path` is present in job config and stage is Implement, the drone dispatches to `run_orchestrated()` instead of the single conversation loop. After parallel tasks complete, a configurable test command runs in a loop (up to N iterations), spawning a fix-up agent with summarised test output on each failure.

**Tech Stack:** Rust (edition 2024), tokio, serde, runtime crate (ConversationLoop, ToolRegistry, EventSink)

---

## File Structure

```
src/drones/native/src/
  config.rs              — Add OrchestratorSection to DroneConfig
  resolve.rs             — Add OrchestratorConfig to ResolvedConfig, merge logic
  drone.rs               — Add dispatch branch, extract run_single_loop()
  orchestrator/
    mod.rs               — Add mod orchestrated, re-export run_orchestrated
    orchestrated.rs      — NEW: run_orchestrated() + test runner + fix-up loop
    executor.rs          — (no changes, already fixed in prior PR)
    scheduler.rs         — (no changes, already fixed in prior PR)
    plan_parser.rs       — (no changes)
```

---

### Task 1: Add OrchestratorSection to DroneConfig

**Files:**
- Modify: `src/drones/native/src/config.rs:8-24` (DroneConfig struct)
- Modify: `src/drones/native/src/config.rs:239-257` (Default impl)
- Modify: `src/drones/native/src/config.rs:289-349` (FULL_CONFIG test)

- [ ] **Step 1: Write test for orchestrator config parsing**

Add this test to the `mod tests` block in `src/drones/native/src/config.rs`:

```rust
#[test]
fn parse_orchestrator_section() {
    let config: DroneConfig = toml::from_str(
        r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[orchestrator]
test_command = "cargo test"
max_fixup_iterations = 3
max_parallel = 4
"#,
    )
    .unwrap();

    assert_eq!(config.orchestrator.test_command.as_deref(), Some("cargo test"));
    assert_eq!(config.orchestrator.max_fixup_iterations, 3);
    assert_eq!(config.orchestrator.max_parallel, 4);
}

#[test]
fn orchestrator_section_defaults() {
    let config: DroneConfig = toml::from_str(
        r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
"#,
    )
    .unwrap();

    assert!(config.orchestrator.test_command.is_none());
    assert_eq!(config.orchestrator.max_fixup_iterations, 5);
    assert_eq!(config.orchestrator.max_parallel, 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/drones/native && cargo test config::tests::parse_orchestrator_section config::tests::orchestrator_section_defaults 2>&1`
Expected: FAIL — `no field named orchestrator`

- [ ] **Step 3: Add OrchestratorSection struct and wire into DroneConfig**

Add the struct after `EnvironmentSection` (around line 179):

```rust
#[derive(Debug, Deserialize)]
pub struct OrchestratorSection {
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default = "default_max_fixup_iterations")]
    pub max_fixup_iterations: u32,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

impl Default for OrchestratorSection {
    fn default() -> Self {
        Self {
            test_command: None,
            max_fixup_iterations: default_max_fixup_iterations(),
            max_parallel: default_max_parallel(),
        }
    }
}
```

Add default functions near the other defaults:

```rust
fn default_max_fixup_iterations() -> u32 {
    5
}
fn default_max_parallel() -> usize {
    2
}
```

Add the field to `DroneConfig`:

```rust
#[serde(default)]
pub orchestrator: OrchestratorSection,
```

Add to `Default for DroneConfig`:

```rust
orchestrator: OrchestratorSection::default(),
```

- [ ] **Step 4: Update FULL_CONFIG test to include orchestrator section**

Add this to the FULL_CONFIG string (before the `[[health_checks]]` block):

```toml
[orchestrator]
test_command = "cargo test --workspace"
max_fixup_iterations = 3
max_parallel = 4
```

Add assertions to `parse_full_config`:

```rust
// Orchestrator
assert_eq!(config.orchestrator.test_command.as_deref(), Some("cargo test --workspace"));
assert_eq!(config.orchestrator.max_fixup_iterations, 3);
assert_eq!(config.orchestrator.max_parallel, 4);
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src/drones/native && cargo test config::tests 2>&1`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/drones/native/src/config.rs
git commit -m "feat(native-drone): add OrchestratorSection to DroneConfig"
```

---

### Task 2: Add OrchestratorConfig to ResolvedConfig

**Files:**
- Modify: `src/drones/native/src/resolve.rs:12-18` (ResolvedConfig struct)
- Modify: `src/drones/native/src/resolve.rs:42-145` (resolve method)

- [ ] **Step 1: Write test for orchestrator config resolution**

Add these tests to `mod tests` in `src/drones/native/src/resolve.rs`:

```rust
#[test]
fn orchestrator_defaults_from_drone_toml() {
    let drone = minimal_drone_config();
    let job = HashMap::new();
    let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

    assert!(resolved.orchestrator.test_command.is_none());
    assert_eq!(resolved.orchestrator.max_fixup_iterations, 5);
    assert_eq!(resolved.orchestrator.max_parallel, 2);
}

#[test]
fn orchestrator_from_drone_toml() {
    let drone: DroneConfig = toml::from_str(
        r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[orchestrator]
test_command = "make test"
max_fixup_iterations = 3
max_parallel = 4
"#,
    )
    .unwrap();

    let job = HashMap::new();
    let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

    assert_eq!(resolved.orchestrator.test_command.as_deref(), Some("make test"));
    assert_eq!(resolved.orchestrator.max_fixup_iterations, 3);
    assert_eq!(resolved.orchestrator.max_parallel, 4);
}

#[test]
fn orchestrator_job_config_overrides_drone_toml() {
    let drone: DroneConfig = toml::from_str(
        r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[orchestrator]
test_command = "make test"
max_fixup_iterations = 3
max_parallel = 4
"#,
    )
    .unwrap();

    let job = make_job_config(&[
        ("test_command", "cargo test"),
        ("max_fixup_iterations", "10"),
        ("max_parallel", "8"),
    ]);
    let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

    assert_eq!(resolved.orchestrator.test_command.as_deref(), Some("cargo test"));
    assert_eq!(resolved.orchestrator.max_fixup_iterations, 10);
    assert_eq!(resolved.orchestrator.max_parallel, 8);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src/drones/native && cargo test resolve::tests::orchestrator_defaults 2>&1`
Expected: FAIL — no field `orchestrator` on `ResolvedConfig`

- [ ] **Step 3: Add OrchestratorConfig and merge logic**

Add the struct to `resolve.rs` (after `EnvironmentConfig`):

```rust
/// Resolved orchestrator configuration
pub struct OrchestratorConfig {
    pub test_command: Option<String>,
    pub max_fixup_iterations: u32,
    pub max_parallel: usize,
}
```

Add the field to `ResolvedConfig`:

```rust
pub orchestrator: OrchestratorConfig,
```

Add merge logic at the end of `ResolvedConfig::resolve()`, before the final `Self { ... }` construction:

```rust
// Orchestrator config: job_config overrides drone.toml
let orchestrator = OrchestratorConfig {
    test_command: job_config
        .get("test_command")
        .map(|s| s.clone())
        .or(drone_toml.orchestrator.test_command.clone()),
    max_fixup_iterations: job_config
        .get("max_fixup_iterations")
        .and_then(|v| v.parse().ok())
        .unwrap_or(drone_toml.orchestrator.max_fixup_iterations),
    max_parallel: job_config
        .get("max_parallel")
        .and_then(|v| v.parse().ok())
        .unwrap_or(drone_toml.orchestrator.max_parallel),
};
```

Add `orchestrator` to the returned `Self { ... }`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/drones/native && cargo test resolve::tests 2>&1`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/drones/native/src/resolve.rs
git commit -m "feat(native-drone): add OrchestratorConfig to ResolvedConfig with merge logic"
```

---

### Task 3: Create orchestrated.rs with test runner

**Files:**
- Create: `src/drones/native/src/orchestrator/orchestrated.rs`
- Modify: `src/drones/native/src/orchestrator/mod.rs`

- [ ] **Step 1: Write tests for the test runner helper**

Create `src/drones/native/src/orchestrator/orchestrated.rs` with the test runner and its tests:

```rust
use std::path::Path;

/// Run a shell command and return (success, stdout, stderr).
async fn run_test_command(command: &str, workspace: &Path) -> (bool, String, String) {
    let output = tokio::process::Command::new("sh")
        .args(["-c", command])
        .current_dir(workspace)
        .output()
        .await;

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            (output.status.success(), stdout, stderr)
        }
        Err(e) => (false, String::new(), format!("failed to run command: {e}")),
    }
}

/// Truncate test output to the last `max_bytes` bytes to avoid blowing context.
fn truncate_output(output: &str, max_bytes: usize) -> &str {
    if output.len() <= max_bytes {
        output
    } else {
        let start = output.len() - max_bytes;
        // Find next char boundary to avoid splitting a multi-byte char
        &output[output.ceil_char_boundary(start)..]
    }
}

/// Build a structured summary of orchestrator results for the fix-up agent.
fn build_orchestrator_summary(results: &[super::TaskResult]) -> String {
    let mut summary = String::from("## Orchestrator Task Results\n\n");
    for result in results {
        let status = if result.success { "PASS" } else { "FAIL" };
        summary.push_str(&format!("- **{}** [{}]: {}\n", result.task_id, status, result.output));
        if !result.commits.is_empty() {
            summary.push_str(&format!("  Commits: {}\n", result.commits.join(", ")));
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_test_command_success() {
        let dir = tempfile::tempdir().unwrap();
        let (success, stdout, _stderr) = run_test_command("echo hello", dir.path()).await;
        assert!(success);
        assert_eq!(stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn run_test_command_failure() {
        let dir = tempfile::tempdir().unwrap();
        let (success, _stdout, _stderr) = run_test_command("false", dir.path()).await;
        assert!(!success);
    }

    #[tokio::test]
    async fn run_test_command_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let (success, _, stderr) =
            run_test_command("nonexistent_command_12345", dir.path()).await;
        assert!(!success);
        assert!(!stderr.is_empty());
    }

    #[test]
    fn truncate_output_short() {
        let output = "short";
        assert_eq!(truncate_output(output, 1000), "short");
    }

    #[test]
    fn truncate_output_long() {
        let output = "a".repeat(100);
        let truncated = truncate_output(&output, 50);
        assert_eq!(truncated.len(), 50);
    }

    #[test]
    fn build_orchestrator_summary_mixed() {
        let results = vec![
            super::super::TaskResult {
                task_id: "task-1".into(),
                success: true,
                output: "completed in 5 iterations".into(),
                commits: vec!["abc123".into()],
            },
            super::super::TaskResult {
                task_id: "task-2".into(),
                success: false,
                output: "task failed: compilation error".into(),
                commits: vec![],
            },
        ];
        let summary = build_orchestrator_summary(&results);
        assert!(summary.contains("task-1"));
        assert!(summary.contains("[PASS]"));
        assert!(summary.contains("task-2"));
        assert!(summary.contains("[FAIL]"));
        assert!(summary.contains("abc123"));
    }
}
```

- [ ] **Step 2: Add module to mod.rs**

In `src/drones/native/src/orchestrator/mod.rs`, add:

```rust
mod orchestrated;

pub use orchestrated::run_orchestrated;
```

(The `pub use` will fail until we define `run_orchestrated` — that's fine, we'll add it in step 4.)

- [ ] **Step 3: Run tests to verify helpers work**

Run: `cd src/drones/native && cargo test orchestrator::orchestrated::tests 2>&1`
Expected: ALL PASS (6 tests)

- [ ] **Step 4: Commit**

```bash
git add src/drones/native/src/orchestrator/orchestrated.rs src/drones/native/src/orchestrator/mod.rs
git commit -m "feat(native-drone): add orchestrated.rs with test runner and summary helpers"
```

---

### Task 4: Implement run_orchestrated()

**Files:**
- Modify: `src/drones/native/src/orchestrator/orchestrated.rs`

- [ ] **Step 1: Add the run_orchestrated function**

Add these imports to the top of `orchestrated.rs`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use runtime::api::ApiClientFactory;
use runtime::conversation::loop_core::{CompactionStrategy, ConversationLoop, LoopConfig};
use runtime::event::{EventSink, RuntimeEvent};
use runtime::permission::PermissionPolicy;
use runtime::tools::ToolRegistry;
use serde_json::json;

use crate::git_workflow::{GitOperation, GitWorkflow};
use crate::resolve::OrchestratorConfig;

use super::plan_parser::Task;
use super::{Orchestrator, TaskResult};
```

Add the function after the helper functions, before `#[cfg(test)]`:

```rust
/// Run orchestrated parallel execution followed by iterative test-fix loop.
///
/// Returns a JSON value containing orchestrator results, fix-up metadata,
/// and final test status for inclusion in DroneOutput.
pub async fn run_orchestrated(
    tasks: Vec<Task>,
    config: &OrchestratorConfig,
    event_sink: Arc<dyn EventSink>,
    git_workflow: Arc<GitWorkflow>,
    api_client_factory: Arc<dyn ApiClientFactory>,
    tool_registry: ToolRegistry,
    loop_config: &LoopConfig,
    system_prompt: Vec<String>,
    workspace: PathBuf,
) -> anyhow::Result<OrchestratedResult> {
    // 1. Build and run orchestrator
    event_sink.emit(RuntimeEvent::TurnStart {
        task: format!("Orchestrating {} tasks (max parallel: {})", tasks.len(), config.max_parallel),
    });

    let mut orchestrator = Orchestrator::new(
        tasks,
        config.max_parallel,
        event_sink.clone(),
        git_workflow.clone(),
        api_client_factory.clone(),
        tool_registry.clone_all(),
        loop_config,
        system_prompt.clone(),
        workspace.clone(),
    )?;

    let task_results = orchestrator.run().await;

    // 2. Build summary for fix-up context
    let orchestrator_summary = build_orchestrator_summary(&task_results);
    let all_tasks_passed = task_results.iter().all(|r| r.success);

    tracing::info!(
        total = task_results.len(),
        passed = task_results.iter().filter(|r| r.success).count(),
        failed = task_results.iter().filter(|r| !r.success).count(),
        "Orchestrator complete"
    );

    // 3. Test-fix loop (only if test_command is configured)
    let mut fixup_iterations = 0u32;
    let mut tests_passing = false;
    let mut fixup_summaries = Vec::new();

    if let Some(test_command) = &config.test_command {
        for iteration in 1..=config.max_fixup_iterations {
            event_sink.emit(RuntimeEvent::TurnStart {
                task: format!("Running tests (iteration {iteration}/{}))", config.max_fixup_iterations),
            });

            let (success, stdout, stderr) = run_test_command(test_command, &workspace).await;
            fixup_iterations = iteration;

            if success {
                tracing::info!(iteration, "Tests passing");
                tests_passing = true;
                break;
            }

            tracing::warn!(iteration, "Tests failed, starting fix-up");

            // Combine and truncate test output
            let raw_output = format!("STDOUT:\n{stdout}\n\nSTDERR:\n{stderr}");
            let truncated = truncate_output(&raw_output, 50_000);

            // Summarise test output via a short conversation loop
            let summary = summarise_test_output(
                truncated,
                api_client_factory.clone(),
                event_sink.clone(),
                workspace.clone(),
            )
            .await;
            fixup_summaries.push(summary.clone());

            // Build fix-up prompt
            let fixup_prompt = format!(
                "The following tests failed after parallel implementation. \
                 Fix the integration issues.\n\n\
                 ## Test Failure Summary\n\n{summary}\n\n\
                 {orchestrator_summary}\n\n\
                 This is fix-up attempt {iteration} of {}.",
                config.max_fixup_iterations
            );

            // Run fix-up agent
            event_sink.emit(RuntimeEvent::TurnStart {
                task: format!("Fix-up agent (iteration {iteration})"),
            });

            let api_client = api_client_factory.create();
            // LoopConfig doesn't implement Clone — reconstruct for each fix-up agent
            let fixup_loop_config = LoopConfig {
                max_iterations: loop_config.max_iterations,
                max_context_tokens: loop_config.max_context_tokens,
                compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 4 },
                max_tokens_per_response: loop_config.max_tokens_per_response,
                temperature: loop_config.temperature,
            };
            let mut fixup_loop = ConversationLoop::new(
                api_client,
                api_client_factory.clone(),
                tool_registry.clone_all(),
                fixup_loop_config,
                event_sink.clone(),
                system_prompt.clone(),
                PermissionPolicy::allow_all(),
                workspace.clone(),
            );
            fixup_loop.set_agent_depth(1);

            match fixup_loop.run_turn(&fixup_prompt).await {
                Ok(turn_result) => {
                    tracing::info!(
                        iteration,
                        iterations = turn_result.iterations,
                        "Fix-up agent completed"
                    );
                }
                Err(e) => {
                    tracing::error!(iteration, error = %e, "Fix-up agent failed");
                }
            }

            // Commit fix-up changes
            let commit_msg = format!("fix: test failures (fix-up iteration {iteration})");
            if let Err(e) = git_workflow
                .execute(&GitOperation::Commit {
                    message: commit_msg,
                    paths: vec![".".into()],
                })
                .await
            {
                tracing::warn!(iteration, error = %e, "Fix-up commit failed (may be no changes)");
            }
        }
    } else {
        tracing::warn!("No test_command configured, skipping test-fix loop");
    }

    Ok(OrchestratedResult {
        task_results,
        fixup_iterations,
        tests_passing,
        fixup_summaries,
    })
}

/// Result of the full orchestrated execution flow.
pub struct OrchestratedResult {
    pub task_results: Vec<TaskResult>,
    pub fixup_iterations: u32,
    pub tests_passing: bool,
    pub fixup_summaries: Vec<String>,
}

impl OrchestratedResult {
    /// All orchestrator tasks passed and tests are passing (or no test command).
    pub fn success(&self) -> bool {
        let all_tasks_ok = self.task_results.iter().all(|r| r.success);
        all_tasks_ok && self.tests_passing
    }

    /// Build a JSON representation for inclusion in DroneOutput.conversation.
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "orchestrator_results": self.task_results.iter().map(|r| json!({
                "task_id": r.task_id,
                "success": r.success,
                "output": r.output,
                "commits": r.commits,
            })).collect::<Vec<_>>(),
            "fixup_iterations": self.fixup_iterations,
            "tests_passing": self.tests_passing,
            "fixup_summaries": self.fixup_summaries,
        })
    }
}

/// Summarise test output using a short conversation loop.
async fn summarise_test_output(
    test_output: &str,
    api_client_factory: Arc<dyn ApiClientFactory>,
    event_sink: Arc<dyn EventSink>,
    workspace: PathBuf,
) -> String {
    let api_client = api_client_factory.create();
    let summarise_config = LoopConfig {
        max_iterations: 3,
        max_context_tokens: 50_000,
        compaction_strategy: CompactionStrategy::Summarize { preserve_recent: 2 },
        max_tokens_per_response: 4096,
        temperature: None,
    };

    let system_prompt = vec![
        "You are a test output summariser. Extract the failing test names, \
         error messages, and relevant file/line references. Be concise. \
         Output only the summary, no preamble."
            .to_string(),
    ];

    let registry = ToolRegistry::new(); // no tools needed for summarisation

    let mut summarise_loop = ConversationLoop::new(
        api_client,
        api_client_factory,
        registry,
        summarise_config,
        event_sink,
        system_prompt,
        PermissionPolicy::allow_all(),
        workspace,
    );
    summarise_loop.set_agent_depth(2);

    let prompt = format!("Summarise these test failures:\n\n```\n{test_output}\n```");

    match summarise_loop.run_turn(&prompt).await {
        Ok(_) => {
            // Extract text from the assistant's last message
            use runtime::conversation::session::{ContentBlock, Role};
            let session = summarise_loop.session();
            session
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| {
                    m.blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| test_output.to_string())
        }
        Err(e) => {
            tracing::warn!(error = %e, "Summarisation failed, using raw output");
            test_output.to_string()
        }
    }
}
```

- [ ] **Step 2: Update mod.rs re-export**

In `src/drones/native/src/orchestrator/mod.rs`, ensure the re-export is present:

```rust
pub use orchestrated::run_orchestrated;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src/drones/native && cargo check 2>&1`
Expected: compiles (warnings about unused imports are OK at this stage)

- [ ] **Step 4: Commit**

```bash
git add src/drones/native/src/orchestrator/orchestrated.rs src/drones/native/src/orchestrator/mod.rs
git commit -m "feat(native-drone): implement run_orchestrated with test-fix loop"
```

---

### Task 5: Wire orchestrator dispatch into drone.rs

**Files:**
- Modify: `src/drones/native/src/drone.rs:180-347` (execute method)

- [ ] **Step 1: Add import for orchestrator and plan parser**

Add to the imports at the top of `drone.rs`:

```rust
use crate::orchestrator::{parse_plan, run_orchestrated};
```

- [ ] **Step 2: Extract run_single_loop()**

Extract lines 266–291 of `drone.rs` (the section from "Create conversation loop" through "Handle turn result") into a standalone async function. Place it after the `impl DroneRunner for NativeDrone` block, before the existing helper functions:

```rust
/// Run a single conversation loop (non-orchestrated path).
async fn run_single_loop(
    task: &str,
    config: &crate::resolve::ResolvedConfig,
    event_sink: Arc<dyn EventSink>,
    api_client_factory: Arc<api::DefaultApiClientFactory>,
    registry: runtime::tools::ToolRegistry,
    workspace: &std::path::Path,
    system_prompt: Vec<String>,
    channel: &mut QueenChannel,
) -> anyhow::Result<(serde_json::Value, u32)> {
    let api_client = api::create_client(&config.provider);

    channel.progress("running", "starting agent loop")?;
    // LoopConfig doesn't implement Clone — reconstruct from resolved config
    let loop_config = LoopConfig {
        max_iterations: config.loop_config.max_iterations,
        max_context_tokens: config.loop_config.max_context_tokens,
        compaction_strategy: CompactionStrategy::Summarize {
            preserve_recent: 4,
        },
        max_tokens_per_response: config.loop_config.max_tokens_per_response,
        temperature: config.loop_config.temperature,
    };
    let mut conversation = ConversationLoop::new(
        api_client,
        api_client_factory.clone(),
        registry,
        loop_config,
        event_sink.clone(),
        system_prompt,
        PermissionPolicy::allow_all(),
        workspace.to_path_buf(),
    );

    let turn_result = conversation.run_turn(task).await?;
    tracing::info!(
        iterations = turn_result.iterations,
        compacted = turn_result.compacted,
        "Agent loop completed"
    );

    let session_value = serde_json::to_value(conversation.session()).unwrap_or(json!({}));
    Ok((session_value, turn_result.iterations))
}
```

- [ ] **Step 3: Add orchestrator dispatch branch in execute()**

Replace the section in `execute()` that creates the conversation loop and runs it (roughly lines 266–291) with the dispatch:

```rust
// 8. Dispatch: orchestrated or single-loop
let plan_path = job_config.get("plan_path");
let use_orchestrator = plan_path.is_some() && stage == Stage::Implement;

let (session_value, all_tasks_ok) = if use_orchestrator {
    let plan_path = plan_path.unwrap();
    let plan_file = env.workspace.join(plan_path);
    let plan_content = tokio::fs::read_to_string(&plan_file).await?;
    let tasks = parse_plan(&plan_content);

    if tasks.is_empty() {
        tracing::warn!(%plan_path, "No tasks found in plan, falling back to single loop");
        let (session_value, _) = run_single_loop(
            &task, &config, bridge.clone(), api_client_factory.clone(),
            registry, &env.workspace, system_prompt, channel,
        ).await?;
        (session_value, true)
    } else {
        tracing::info!(tasks = tasks.len(), %plan_path, "Running orchestrated execution");
        channel.progress("orchestrating", &format!("executing {} tasks from plan", tasks.len()))?;

        let git_workflow = Arc::new(GitWorkflow::new(
            config.stage_config.git.clone(),
            env.workspace.clone(),
        ));

        let result = run_orchestrated(
            tasks,
            &config.orchestrator,
            bridge.clone(),
            git_workflow,
            api_client_factory.clone(),
            registry,
            &config.loop_config,
            system_prompt,
            env.workspace.clone(),
        ).await?;

        let success = result.success();
        (result.to_json(), success)
    }
} else {
    let (session_value, _) = run_single_loop(
        &task, &config, bridge.clone(), api_client_factory.clone(),
        registry, &env.workspace, system_prompt, channel,
    ).await?;
    (session_value, true)
};
```

- [ ] **Step 4: Update DroneOutput construction to use dispatched results**

The `exit_code` calculation and `conversation` field in the returned `DroneOutput` should use the values from the dispatch:

```rust
Ok(DroneOutput {
    exit_code: if all_met && all_tasks_ok { 0 } else { 1 },
    conversation: session_value,
    // ... rest unchanged
})
```

- [ ] **Step 5: Verify it compiles**

Run: `cd src/drones/native && cargo check 2>&1`
Expected: compiles

- [ ] **Step 6: Run all existing tests to verify no regressions**

Run: `cd src/drones/native && cargo test 2>&1`
Expected: ALL PASS

- [ ] **Step 7: Commit**

```bash
git add src/drones/native/src/drone.rs
git commit -m "feat(native-drone): wire orchestrator dispatch into execute()"
```

---

### Task 6: Integration test with mock test command

**Files:**
- Modify: `src/drones/native/src/orchestrator/orchestrated.rs` (add integration tests)

- [ ] **Step 1: Add integration test for run_test_command + truncation in a realistic scenario**

Add to the test module in `orchestrated.rs`:

```rust
#[tokio::test]
async fn run_test_command_captures_both_streams() {
    let dir = tempfile::tempdir().unwrap();
    let (success, stdout, stderr) =
        run_test_command("echo out && echo err >&2", dir.path()).await;
    assert!(success);
    assert_eq!(stdout.trim(), "out");
    assert_eq!(stderr.trim(), "err");
}

#[test]
fn orchestrated_result_success_all_pass() {
    let result = super::OrchestratedResult {
        task_results: vec![
            super::super::TaskResult {
                task_id: "a".into(),
                success: true,
                output: "ok".into(),
                commits: vec![],
            },
        ],
        fixup_iterations: 0,
        tests_passing: true,
        fixup_summaries: vec![],
    };
    assert!(result.success());
}

#[test]
fn orchestrated_result_failure_on_failed_task() {
    let result = super::OrchestratedResult {
        task_results: vec![
            super::super::TaskResult {
                task_id: "a".into(),
                success: false,
                output: "failed".into(),
                commits: vec![],
            },
        ],
        fixup_iterations: 0,
        tests_passing: true,
        fixup_summaries: vec![],
    };
    assert!(!result.success());
}

#[test]
fn orchestrated_result_failure_on_test_fail() {
    let result = super::OrchestratedResult {
        task_results: vec![
            super::super::TaskResult {
                task_id: "a".into(),
                success: true,
                output: "ok".into(),
                commits: vec![],
            },
        ],
        fixup_iterations: 3,
        tests_passing: false,
        fixup_summaries: vec!["some failure".into()],
    };
    assert!(!result.success());
}

#[test]
fn orchestrated_result_to_json() {
    let result = super::OrchestratedResult {
        task_results: vec![
            super::super::TaskResult {
                task_id: "task-1".into(),
                success: true,
                output: "done".into(),
                commits: vec!["abc".into()],
            },
        ],
        fixup_iterations: 2,
        tests_passing: true,
        fixup_summaries: vec!["summary1".into(), "summary2".into()],
    };
    let json = result.to_json();
    assert_eq!(json["fixup_iterations"], 2);
    assert_eq!(json["tests_passing"], true);
    assert_eq!(json["orchestrator_results"][0]["task_id"], "task-1");
    assert_eq!(json["fixup_summaries"].as_array().unwrap().len(), 2);
}
```

- [ ] **Step 2: Run all orchestrator tests**

Run: `cd src/drones/native && cargo test orchestrator 2>&1`
Expected: ALL PASS

- [ ] **Step 3: Run full test suite**

Run: `cd src/drones/native && cargo test 2>&1`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/drones/native/src/orchestrator/orchestrated.rs
git commit -m "test(native-drone): add integration tests for OrchestratedResult"
```

---

### Task 7: Format and lint

**Files:**
- Possibly all modified files

- [ ] **Step 1: Run rustfmt**

Run: `rustfmt src/drones/native/src/config.rs src/drones/native/src/resolve.rs src/drones/native/src/drone.rs src/drones/native/src/orchestrator/orchestrated.rs src/drones/native/src/orchestrator/mod.rs`

- [ ] **Step 2: Run clippy**

Run: `cd src/drones/native && cargo clippy -- -D warnings 2>&1`
Expected: No warnings

Fix any issues found.

- [ ] **Step 3: Run all tests one final time**

Run: `cd src/drones/native && cargo test 2>&1`
Expected: ALL PASS

- [ ] **Step 4: Commit any formatting fixes**

```bash
git add -u src/drones/native/
git commit -m "style: format and lint orchestrator integration"
```
