use std::path::{Path, PathBuf};
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
fn build_orchestrator_summary(results: &[TaskResult]) -> String {
    let mut summary = String::from("## Orchestrator Task Results\n\n");
    for result in results {
        let status = if result.success { "PASS" } else { "FAIL" };
        summary.push_str(&format!(
            "- **{}** [{}]: {}\n",
            result.task_id, status, result.output
        ));
        if !result.commits.is_empty() {
            summary.push_str(&format!("  Commits: {}\n", result.commits.join(", ")));
        }
    }
    summary
}

/// Run orchestrated parallel execution followed by iterative test-fix loop.
///
/// Returns a structured result containing orchestrator results, fix-up metadata,
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
        task: format!(
            "Orchestrating {} tasks (max parallel: {})",
            tasks.len(),
            config.max_parallel
        ),
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
                task: format!(
                    "Running tests (iteration {iteration}/{})",
                    config.max_fixup_iterations
                ),
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
                compaction_strategy: CompactionStrategy::Summarize {
                    preserve_recent: 4,
                },
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
        compaction_strategy: CompactionStrategy::Summarize {
            preserve_recent: 2,
        },
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
            TaskResult {
                task_id: "task-1".into(),
                success: true,
                output: "completed in 5 iterations".into(),
                commits: vec!["abc123".into()],
            },
            TaskResult {
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
        let result = OrchestratedResult {
            task_results: vec![TaskResult {
                task_id: "a".into(),
                success: true,
                output: "ok".into(),
                commits: vec![],
            }],
            fixup_iterations: 0,
            tests_passing: true,
            fixup_summaries: vec![],
        };
        assert!(result.success());
    }

    #[test]
    fn orchestrated_result_failure_on_failed_task() {
        let result = OrchestratedResult {
            task_results: vec![TaskResult {
                task_id: "a".into(),
                success: false,
                output: "failed".into(),
                commits: vec![],
            }],
            fixup_iterations: 0,
            tests_passing: true,
            fixup_summaries: vec![],
        };
        assert!(!result.success());
    }

    #[test]
    fn orchestrated_result_failure_on_test_fail() {
        let result = OrchestratedResult {
            task_results: vec![TaskResult {
                task_id: "a".into(),
                success: true,
                output: "ok".into(),
                commits: vec![],
            }],
            fixup_iterations: 3,
            tests_passing: false,
            fixup_summaries: vec!["some failure".into()],
        };
        assert!(!result.success());
    }

    #[test]
    fn orchestrated_result_to_json() {
        let result = OrchestratedResult {
            task_results: vec![TaskResult {
                task_id: "task-1".into(),
                success: true,
                output: "done".into(),
                commits: vec!["abc".into()],
            }],
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
}
