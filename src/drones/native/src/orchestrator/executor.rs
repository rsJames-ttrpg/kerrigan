use std::path::PathBuf;
use std::sync::Arc;

use runtime::api::ApiClientFactory;
use runtime::conversation::loop_core::{CompactionStrategy, ConversationLoop, LoopConfig};
use runtime::event::{EventSink, RuntimeEvent};
use runtime::permission::PermissionPolicy;
use runtime::tools::ToolRegistry;

use crate::git_workflow::GitWorkflow;

use super::plan_parser::Task;
use super::scheduler::TaskScheduler;

/// Configuration for sub-agent conversation loops, stored as copyable values
/// since `LoopConfig` does not implement `Clone`.
struct SubAgentConfig {
    max_iterations: u32,
    max_context_tokens: u32,
    max_tokens_per_response: u32,
    temperature: Option<f32>,
}

impl SubAgentConfig {
    fn from_loop_config(config: &LoopConfig) -> Self {
        Self {
            max_iterations: config.max_iterations,
            max_context_tokens: config.max_context_tokens,
            max_tokens_per_response: config.max_tokens_per_response,
            temperature: config.temperature,
        }
    }

    fn to_loop_config(&self) -> LoopConfig {
        LoopConfig {
            max_iterations: self.max_iterations,
            max_context_tokens: self.max_context_tokens,
            compaction_strategy: CompactionStrategy::Summarize {
                preserve_recent: 4,
            },
            max_tokens_per_response: self.max_tokens_per_response,
            temperature: self.temperature,
        }
    }
}

pub struct Orchestrator {
    scheduler: TaskScheduler,
    max_parallel: usize,
    event_sink: Arc<dyn EventSink>,
    git_workflow: Arc<GitWorkflow>,
    api_client_factory: Arc<dyn ApiClientFactory>,
    tool_registry: ToolRegistry,
    sub_agent_config: SubAgentConfig,
    system_prompt: Vec<String>,
    workspace: PathBuf,
}

#[derive(Debug)]
pub struct TaskResult {
    pub task_id: String,
    pub success: bool,
    pub output: String,
    pub commits: Vec<String>,
}

impl Orchestrator {
    pub fn new(
        tasks: Vec<Task>,
        max_parallel: usize,
        event_sink: Arc<dyn EventSink>,
        git_workflow: Arc<GitWorkflow>,
        api_client_factory: Arc<dyn ApiClientFactory>,
        tool_registry: ToolRegistry,
        loop_config: &LoopConfig,
        system_prompt: Vec<String>,
        workspace: PathBuf,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            scheduler: TaskScheduler::new(tasks)?,
            max_parallel,
            event_sink,
            git_workflow,
            api_client_factory,
            tool_registry,
            sub_agent_config: SubAgentConfig::from_loop_config(loop_config),
            system_prompt,
            workspace,
        })
    }

    /// Run all tasks, respecting the dependency graph and parallelism limit.
    pub async fn run(&mut self) -> Vec<TaskResult> {
        let mut results = Vec::new();
        let mut active: Vec<tokio::task::JoinHandle<TaskResult>> = Vec::new();

        loop {
            // Spawn ready tasks up to max_parallel
            while active.len() < self.max_parallel {
                let ready = self.scheduler.ready_tasks();
                if ready.is_empty() {
                    break;
                }
                let task = ready[0].clone();
                self.scheduler.start(&task.id);

                self.event_sink.emit(RuntimeEvent::TurnStart {
                    task: format!("Task {}: {}", task.id, task.description),
                });

                let handle = self.spawn_task_agent(task);
                active.push(handle);
            }

            if active.is_empty() {
                break;
            }

            // Wait for any task to complete
            let (result, _index, remaining) = futures::future::select_all(active).await;
            active = remaining;

            match result {
                Ok(task_result) => {
                    let task_id = task_result.task_id.clone();
                    let success = task_result.success;
                    self.scheduler.complete(&task_id);

                    tracing::info!(
                        task_id = %task_id,
                        success,
                        remaining = self.scheduler.remaining(),
                        "Task completed"
                    );

                    results.push(task_result);
                }
                Err(e) => {
                    tracing::error!("task agent panicked: {e}");
                }
            }
        }

        results
    }

    fn spawn_task_agent(&self, task: Task) -> tokio::task::JoinHandle<TaskResult> {
        let task_id = task.id.clone();
        let event_sink = self.event_sink.clone();
        let api_client_factory = self.api_client_factory.clone();
        let tool_registry = self.tool_registry.clone_all();
        let loop_config = self.sub_agent_config.to_loop_config();
        let system_prompt = self.system_prompt.clone();
        let workspace = self.workspace.clone();
        let git_workflow = self.git_workflow.clone();

        let prompt = format!(
            "Implement task {}: {}\n\nRelevant files: {}",
            task.id,
            task.description,
            task.files.join(", ")
        );

        tokio::spawn(async move {
            let api_client = api_client_factory.create();
            let mut sub_loop = ConversationLoop::new(
                api_client,
                api_client_factory,
                tool_registry,
                loop_config,
                event_sink.clone(),
                system_prompt,
                PermissionPolicy::allow_all(),
                workspace,
            );
            sub_loop.set_agent_depth(1);

            let (success, output) = match sub_loop.run_turn(&prompt).await {
                Ok(turn_result) => {
                    let output = format!(
                        "completed in {} iterations (compacted: {})",
                        turn_result.iterations, turn_result.compacted
                    );
                    (true, output)
                }
                Err(e) => (false, format!("task failed: {e}")),
            };

            // Serialize git commit for this task's changes
            let commits = if success {
                let commit_msg = format!("task {task_id}: {}", task.description);
                match git_workflow
                    .execute(&crate::git_workflow::GitOperation::Commit {
                        message: commit_msg,
                        paths: task.files.clone(),
                    })
                    .await
                {
                    Ok(commit_output) => vec![commit_output],
                    Err(e) => {
                        tracing::warn!(task_id = %task_id, "git commit failed: {e}");
                        vec![]
                    }
                }
            } else {
                vec![]
            };

            TaskResult {
                task_id,
                success,
                output,
                commits,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::parse_plan;
    use crate::pipeline::StageGitConfig;

    fn make_git_workflow(dir: &std::path::Path) -> Arc<GitWorkflow> {
        Arc::new(GitWorkflow::new(
            StageGitConfig {
                branch_name: None,
                allowed_operations: None,
                commit_on_checkpoint: false,
                commit_on_task_complete: true,
                pr_on_stage_complete: false,
                protected_paths: vec![],
            },
            dir.to_path_buf(),
        ))
    }

    #[test]
    fn test_orchestrator_rejects_cycle() {
        let tasks = vec![
            Task {
                id: "a".into(),
                description: "first".into(),
                dependencies: vec!["b".into()],
                files: vec![],
            },
            Task {
                id: "b".into(),
                description: "second".into(),
                dependencies: vec!["a".into()],
                files: vec![],
            },
        ];

        // Scheduler validation catches the cycle
        let result = TaskScheduler::new(tasks);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_plan_into_scheduler() {
        let md = r#"## Tasks

- [ ] **auth-middleware**: Add auth middleware to axum router
  - Files: src/api/mod.rs, src/api/auth.rs
  - Depends: none

- [ ] **auth-tests**: Write auth middleware tests
  - Files: src/api/auth.rs, tests/api_auth.rs
  - Depends: auth-middleware

- [ ] **auth-docs**: Document the auth flow
  - Files: docs/auth.md
  - Depends: auth-middleware
"#;
        let tasks = parse_plan(md);
        assert_eq!(tasks.len(), 3);

        // Verify scheduler accepts this DAG
        let mut scheduler = TaskScheduler::new(tasks).unwrap();

        // Initially only auth-middleware is ready (no deps)
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "auth-middleware");

        // After completing auth-middleware, both auth-tests and auth-docs are ready
        scheduler.start("auth-middleware");
        scheduler.complete("auth-middleware");
        let ready = scheduler.ready_tasks();
        assert_eq!(ready.len(), 2);
    }

    #[test]
    fn test_sub_agent_config_roundtrip() {
        let original = LoopConfig {
            max_iterations: 25,
            max_context_tokens: 50_000,
            compaction_strategy: CompactionStrategy::Summarize {
                preserve_recent: 4,
            },
            max_tokens_per_response: 8192,
            temperature: Some(0.7),
        };

        let sub_config = SubAgentConfig::from_loop_config(&original);
        let reconstructed = sub_config.to_loop_config();

        assert_eq!(reconstructed.max_iterations, 25);
        assert_eq!(reconstructed.max_context_tokens, 50_000);
        assert_eq!(reconstructed.max_tokens_per_response, 8192);
        assert_eq!(reconstructed.temperature, Some(0.7));
    }
}
