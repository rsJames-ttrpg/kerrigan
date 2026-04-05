use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use drone_sdk::{
    harness::QueenChannel,
    protocol::{DroneEnvironment, DroneEvent, DroneMessage, DroneOutput, GitRefs, JobSpec},
    runner::DroneRunner,
};
use runtime::{
    api::{self, DefaultApiClientFactory},
    conversation::loop_core::{CompactionStrategy, ConversationLoop, LoopConfig},
    event::EventSink,
    permission::PermissionPolicy,
    tools,
};
use serde_json::json;

use crate::git_workflow::GitWorkflow;
use crate::orchestrator::{parse_plan, run_orchestrated};

use crate::cache::RepoCache;
use crate::config::DroneConfig;
use crate::event_bridge::DroneEventBridge;
use crate::exit_conditions;
use crate::health;
use crate::pipeline::Stage;
use crate::prompt::PromptBuilder;
use crate::resolve::ResolvedConfig;

pub struct NativeDrone;

/// State created during setup, persisted to disk so execute can load it.
struct DroneState {
    config: ResolvedConfig,
    task: String,
    job_run_id: String,
    repo_url: String,
}

impl NativeDrone {
    fn validate_job_id(id: &str) -> anyhow::Result<()> {
        if id.is_empty() {
            anyhow::bail!("job_run_id is empty");
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!(
                "job_run_id contains invalid characters (only alphanumeric, -, _ allowed): {id}"
            );
        }
        Ok(())
    }

    fn parse_job_config(config: &serde_json::Value) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Some(obj) = config.as_object() {
            for (key, value) in obj {
                match value {
                    serde_json::Value::String(s) => {
                        map.insert(key.clone(), s.clone());
                    }
                    serde_json::Value::Object(inner) => {
                        for (inner_key, inner_value) in inner {
                            if let Some(s) = inner_value.as_str() {
                                map.insert(format!("{key}.{inner_key}"), s.to_string());
                            }
                        }
                    }
                    other => {
                        map.insert(key.clone(), other.to_string());
                    }
                }
            }
        }
        map
    }

    fn resolve_stage(job_config: &HashMap<String, String>) -> Stage {
        job_config
            .get("stage")
            .and_then(|s| serde_json::from_value(json!(s)).ok())
            .unwrap_or(Stage::Freeform)
    }
}

#[async_trait]
impl DroneRunner for NativeDrone {
    async fn setup(&self, job: &JobSpec) -> anyhow::Result<DroneEnvironment> {
        tracing::info!(run_id = %job.job_run_id, "Setting up native drone");

        // 1. Validate job run ID
        Self::validate_job_id(&job.job_run_id)?;

        // 2. Load drone.toml config
        let config_path =
            std::env::var("DRONE_CONFIG").unwrap_or_else(|_| "drone.toml".to_string());
        let drone_toml = if std::path::Path::new(&config_path).exists() {
            DroneConfig::load(std::path::Path::new(&config_path))?
        } else {
            tracing::warn!("No drone.toml found at {config_path}, using defaults");
            DroneConfig::default()
        };

        // 3. Parse job config and resolve stage
        let job_config = Self::parse_job_config(&job.config);
        let stage = Self::resolve_stage(&job_config);
        tracing::info!(?stage, "Resolved stage");

        // 4. Resolve config (merge drone.toml + job spec + stage defaults)
        let resolved = ResolvedConfig::resolve(&drone_toml, &job_config, stage);

        // 5. Create isolated home directory
        let home = PathBuf::from(format!("/tmp/drone-{}", job.job_run_id));
        tokio::fs::create_dir_all(&home).await?;
        let workspace = home.join("workspace");
        tokio::fs::create_dir_all(&workspace).await?;

        // 6. Clone/fetch repo via cache manager
        if !job.repo_url.is_empty() && resolved.cache.repo_cache {
            let branch = job
                .branch
                .as_deref()
                .or(resolved.stage_config.git.branch_name.as_deref())
                .unwrap_or("main");

            let cache = RepoCache::new(resolved.cache.dir.clone());
            cache.checkout(&job.repo_url, branch, &workspace).await?;
            tracing::info!(repo = %job.repo_url, branch, "Repository checked out");
        }

        // 7. Configure git credentials if PAT provided
        if let Some(pat) = job_config.get("secrets.github_pat") {
            let credential_helper = format!(
                "!f() {{ echo \"protocol=https\nhost=github.com\nusername=x-access-token\npassword={pat}\"; }}; f"
            );
            let _ = tokio::process::Command::new("git")
                .args(["config", "credential.helper", &credential_helper])
                .current_dir(&workspace)
                .output()
                .await;
        }

        // 8. Set up environment variables
        for (key, value) in &resolved.environment.env {
            // SAFETY: The drone binary uses current_thread runtime, so no other
            // tokio worker threads exist. No concurrent set_var/var calls possible.
            unsafe { std::env::set_var(key, value) };
        }

        // 9. Persist state for execute phase (filter secrets — never write them to disk)
        let safe_config: HashMap<&String, &String> = job_config
            .iter()
            .filter(|(k, _)| !k.starts_with("secrets."))
            .collect();
        let state = serde_json::json!({
            "task": &job.task,
            "job_run_id": &job.job_run_id,
            "repo_url": &job.repo_url,
            "job_config": &safe_config,
            "stage": serde_json::to_value(&resolved.stage_config.stage).unwrap_or(json!("freeform")),
        });
        let state_path = home.join("drone_state.json");
        tokio::fs::write(&state_path, serde_json::to_string_pretty(&state)?).await?;

        // Also persist drone.toml path for reload in execute
        let config_meta = serde_json::json!({
            "drone_toml_path": config_path,
        });
        tokio::fs::write(
            home.join("config_meta.json"),
            serde_json::to_string(&config_meta)?,
        )
        .await?;

        tracing::info!(home = %home.display(), workspace = %workspace.display(), "Setup complete");

        Ok(DroneEnvironment { home, workspace })
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> anyhow::Result<DroneOutput> {
        // 1. Reload state from setup phase
        let state_json: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(env.home.join("drone_state.json")).await?,
        )?;
        let task = state_json["task"].as_str().unwrap_or("").to_string();
        let job_run_id = state_json["job_run_id"].as_str().unwrap_or("").to_string();
        let repo_url = state_json["repo_url"].as_str().unwrap_or("").to_string();
        let job_config: HashMap<String, String> = state_json["job_config"]
            .as_object()
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let stage = Self::resolve_stage(&job_config);

        // Reload drone.toml
        let config_meta: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(env.home.join("config_meta.json")).await?,
        )?;
        let config_path = config_meta["drone_toml_path"]
            .as_str()
            .unwrap_or("drone.toml");
        let drone_toml = if std::path::Path::new(config_path).exists() {
            DroneConfig::load(std::path::Path::new(config_path))?
        } else {
            DroneConfig::default()
        };
        let config = ResolvedConfig::resolve(&drone_toml, &job_config, stage);

        // 2. Create event bridge
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = Arc::new(DroneEventBridge::new(
            tx,
            env.workspace.clone(),
            job_run_id.clone(),
        ));

        // 3. Run health checks
        channel.progress("health_check", "running health checks")?;
        let health_checks = health::checks_for_stage(&config.stage_config.stage);
        let report = health::run_health_checks(&health_checks).await;

        // Send health check results
        channel.send(&DroneMessage::Event(DroneEvent::TestResults {
            passed: report.checks.iter().filter(|c| c.passed).count() as u32,
            failed: report.checks.iter().filter(|c| !c.passed).count() as u32,
            skipped: 0,
        }))?;

        if !report.all_required_passed() {
            let summary = report.summary();
            tracing::error!(%summary, "Required health checks failed");
            return Err(anyhow::anyhow!("health checks failed: {summary}"));
        }

        channel.progress("health_check", "all checks passed")?;

        // 4. Build tool registry
        let registry = tools::default_registry();

        // 5. Read workspace context (CLAUDE.md if present)
        let workspace_context = read_claude_md(&env.workspace).await;

        // 6. Build prompt
        let prompt_builder = PromptBuilder::for_stage(
            &config.stage_config.stage,
            &config.stage_config,
            &registry,
            &workspace_context,
            None,
            None,
        );
        let system_prompt = prompt_builder.build();

        // 7. Create API client factory
        let api_client_factory = Arc::new(DefaultApiClientFactory::new(config.provider.clone()));

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
                    &task,
                    &config,
                    bridge.clone(),
                    api_client_factory.clone(),
                    registry,
                    &env.workspace,
                    system_prompt,
                    channel,
                )
                .await?;
                (session_value, true)
            } else {
                tracing::info!(tasks = tasks.len(), %plan_path, "Running orchestrated execution");
                channel.progress(
                    "orchestrating",
                    &format!("executing {} tasks from plan", tasks.len()),
                )?;

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
                )
                .await?;

                let success = result.success();
                (result.to_json(), success)
            }
        } else {
            let (session_value, _) = run_single_loop(
                &task,
                &config,
                bridge.clone(),
                api_client_factory.clone(),
                registry,
                &env.workspace,
                system_prompt,
                channel,
            )
            .await?;
            (session_value, true)
        };

        // 9. Drain event bridge messages and forward to Queen
        // Drop the bridge sender to close the channel
        drop(bridge);
        while let Ok(msg) = rx.try_recv() {
            if let Err(e) = channel.send(&msg) {
                tracing::warn!("Failed to forward event to queen: {e}");
            }
        }

        // 10. Check exit conditions
        let conditions = exit_conditions::check_exit_conditions(
            &config.stage_config.exit_conditions,
            &env.workspace,
        )
        .await;
        let all_met = conditions.iter().all(|c| c.met);

        for condition in &conditions {
            tracing::info!(
                condition = %condition.condition,
                met = condition.met,
                detail = %condition.detail,
                "Exit condition check"
            );
        }

        // 11. Handle git finalization - create PR if configured
        let mut pr_url = None;
        if config.stage_config.git.pr_on_stage_complete {
            let pr_result = create_pr_if_needed(&env.workspace).await;
            match pr_result {
                Ok(url) => {
                    pr_url = Some(url.clone());
                    channel.send(&DroneMessage::Event(DroneEvent::GitPrCreated { url }))?;
                }
                Err(e) => {
                    tracing::warn!("PR creation failed: {e}");
                }
            }
        }

        // 12. Get current git branch
        let branch = get_current_branch(&env.workspace).await;

        // 13. Build DroneOutput
        Ok(DroneOutput {
            exit_code: if all_met && all_tasks_ok { 0 } else { 1 },
            conversation: session_value,
            artifacts: vec![],
            git_refs: GitRefs {
                branch,
                pr_url,
                pr_required: config.stage_config.git.pr_on_stage_complete,
            },
            session_jsonl_gz: None,
        })
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        tracing::info!(home = %env.home.display(), "Tearing down native drone");

        // 1. Cleanup git worktree via cache manager (if repo cache was used)
        if let Ok(state_json) = tokio::fs::read_to_string(env.home.join("drone_state.json")).await {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&state_json) {
                let repo_url = state["repo_url"].as_str().unwrap_or("");
                if !repo_url.is_empty() {
                    let config_path =
                        std::env::var("DRONE_CONFIG").unwrap_or_else(|_| "drone.toml".into());
                    if let Ok(drone_toml) = DroneConfig::load(std::path::Path::new(&config_path)) {
                        let cache = RepoCache::new(drone_toml.cache.dir.clone());
                        let _ = cache.cleanup_worktree(repo_url, &env.workspace).await;
                    }
                }
            }
        }

        // 2. Remove the entire drone home directory
        if let Err(e) = tokio::fs::remove_dir_all(&env.home).await {
            tracing::warn!(path = %env.home.display(), error = %e, "Failed to clean up drone home");
        }
    }
}

/// Run a single conversation loop (non-orchestrated path).
async fn run_single_loop(
    task: &str,
    config: &ResolvedConfig,
    event_sink: Arc<dyn EventSink>,
    api_client_factory: Arc<DefaultApiClientFactory>,
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
        event_sink,
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

async fn read_claude_md(workspace: &std::path::Path) -> String {
    let claude_md = workspace.join("CLAUDE.md");
    tokio::fs::read_to_string(&claude_md)
        .await
        .unwrap_or_default()
}

async fn get_current_branch(workspace: &std::path::Path) -> Option<String> {
    tokio::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(workspace)
        .output()
        .await
        .ok()
        .and_then(|o| {
            let branch = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if branch.is_empty() {
                None
            } else {
                Some(branch)
            }
        })
}

async fn create_pr_if_needed(workspace: &std::path::Path) -> anyhow::Result<String> {
    // Check if PR already exists
    let check = tokio::process::Command::new("gh")
        .args(["pr", "view", "--json", "url", "-q", ".url"])
        .current_dir(workspace)
        .output()
        .await?;

    if check.status.success() {
        let url = String::from_utf8_lossy(&check.stdout).trim().to_string();
        if !url.is_empty() {
            tracing::info!(%url, "PR already exists");
            return Ok(url);
        }
    }

    // Push current branch
    match tokio::process::Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
    {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(%stderr, "git push failed");
            anyhow::bail!("git push failed: {stderr}");
        }
        Err(e) => {
            anyhow::bail!("failed to spawn git push: {e}");
        }
        _ => {}
    }

    // Get commit subject for PR title
    let title = tokio::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .current_dir(workspace)
        .output()
        .await
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        })
        .unwrap_or_else(|| "Automated PR from native drone".to_string());

    // Create PR
    let output = tokio::process::Command::new("gh")
        .args([
            "pr",
            "create",
            "--title",
            &title,
            "--body",
            "Automated PR created by native drone",
        ])
        .current_dir(workspace)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh pr create failed: {stderr}");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(url)
}

#[cfg(test)]
mod tests {
    use drone_sdk::runner::DroneRunner;

    use super::*;

    #[test]
    fn test_validate_job_id_valid() {
        assert!(NativeDrone::validate_job_id("run-42").is_ok());
        assert!(NativeDrone::validate_job_id("abc_123").is_ok());
        assert!(NativeDrone::validate_job_id("simple").is_ok());
    }

    #[test]
    fn test_validate_job_id_invalid() {
        assert!(NativeDrone::validate_job_id("").is_err());
        assert!(NativeDrone::validate_job_id("has spaces").is_err());
        assert!(NativeDrone::validate_job_id("has/slash").is_err());
        assert!(NativeDrone::validate_job_id("has..dots").is_err());
    }

    #[test]
    fn test_parse_job_config_flat() {
        let config = json!({
            "model": "claude-sonnet-4-20250514",
            "stage": "implement",
            "max_tokens": "8192"
        });
        let parsed = NativeDrone::parse_job_config(&config);
        assert_eq!(parsed.get("model").unwrap(), "claude-sonnet-4-20250514");
        assert_eq!(parsed.get("stage").unwrap(), "implement");
    }

    #[test]
    fn test_parse_job_config_nested_secrets() {
        let config = json!({
            "secrets": {
                "github_pat": "ghp_xxx",
                "api_key": "sk-xxx"
            }
        });
        let parsed = NativeDrone::parse_job_config(&config);
        assert_eq!(parsed.get("secrets.github_pat").unwrap(), "ghp_xxx");
        assert_eq!(parsed.get("secrets.api_key").unwrap(), "sk-xxx");
    }

    #[test]
    fn test_resolve_stage_from_config() {
        let mut config = HashMap::new();
        config.insert("stage".to_string(), "implement".to_string());
        assert_eq!(NativeDrone::resolve_stage(&config), Stage::Implement);
    }

    #[test]
    fn test_resolve_stage_default_freeform() {
        let config = HashMap::new();
        assert_eq!(NativeDrone::resolve_stage(&config), Stage::Freeform);
    }

    #[test]
    fn test_resolve_stage_invalid_defaults_freeform() {
        let mut config = HashMap::new();
        config.insert("stage".to_string(), "bogus".to_string());
        assert_eq!(NativeDrone::resolve_stage(&config), Stage::Freeform);
    }

    #[tokio::test]
    async fn smoke_test_setup_creates_environment() {
        let drone = NativeDrone;
        let job = JobSpec {
            job_run_id: "smoke-test-001".to_string(),
            repo_url: String::new(), // skip git clone
            branch: None,
            task: "write a hello world program".to_string(),
            config: json!({
                "stage": "freeform",
            }),
        };

        let env = drone.setup(&job).await.unwrap();

        // Verify home and workspace directories were created
        assert!(env.home.exists());
        assert!(env.workspace.exists());
        assert_eq!(env.home, PathBuf::from("/tmp/drone-smoke-test-001"));
        assert_eq!(
            env.workspace,
            PathBuf::from("/tmp/drone-smoke-test-001/workspace")
        );

        // Verify state file was persisted
        let state_path = env.home.join("drone_state.json");
        assert!(state_path.exists());
        let state: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&state_path).await.unwrap()).unwrap();
        assert_eq!(state["task"], "write a hello world program");
        assert_eq!(state["job_run_id"], "smoke-test-001");
        assert_eq!(state["stage"], "freeform");

        // Verify config meta was persisted
        let meta_path = env.home.join("config_meta.json");
        assert!(meta_path.exists());

        // Teardown
        drone.teardown(&env).await;

        // Verify cleanup
        assert!(!env.home.exists());
    }

    #[tokio::test]
    async fn smoke_test_setup_rejects_invalid_job_id() {
        let drone = NativeDrone;
        let job = JobSpec {
            job_run_id: "has/invalid/chars".to_string(),
            repo_url: String::new(),
            branch: None,
            task: "test".to_string(),
            config: json!({}),
        };

        let result = drone.setup(&job).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid characters"),
            "should mention invalid characters"
        );
    }

    #[tokio::test]
    async fn smoke_test_setup_with_stage_config() {
        let drone = NativeDrone;
        let job = JobSpec {
            job_run_id: "smoke-test-stage".to_string(),
            repo_url: String::new(),
            branch: None,
            task: "implement feature".to_string(),
            config: json!({
                "stage": "implement",
                "model": "claude-sonnet-4-20250514",
            }),
        };

        let env = drone.setup(&job).await.unwrap();

        let state: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(env.home.join("drone_state.json"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(state["stage"], "implement");

        drone.teardown(&env).await;
    }

    #[tokio::test]
    async fn smoke_test_teardown_idempotent() {
        let drone = NativeDrone;
        let job = JobSpec {
            job_run_id: "smoke-test-teardown".to_string(),
            repo_url: String::new(),
            branch: None,
            task: "test".to_string(),
            config: json!({}),
        };

        let env = drone.setup(&job).await.unwrap();
        assert!(env.home.exists());

        // First teardown
        drone.teardown(&env).await;
        assert!(!env.home.exists());

        // Second teardown should not panic
        drone.teardown(&env).await;
    }

    #[tokio::test]
    async fn smoke_test_secrets_excluded_from_state_file() {
        let drone = NativeDrone;
        let job = JobSpec {
            job_run_id: "smoke-test-secrets".to_string(),
            repo_url: String::new(),
            branch: None,
            task: "test secrets filtering".to_string(),
            config: json!({
                "stage": "implement",
                "model": "claude-sonnet-4-20250514",
                "secrets": {
                    "github_pat": "ghp_supersecret",
                    "api_key": "sk-secret123"
                }
            }),
        };

        let env = drone.setup(&job).await.unwrap();

        let state: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(env.home.join("drone_state.json"))
                .await
                .unwrap(),
        )
        .unwrap();

        // Secrets must not appear in persisted state
        let job_config = state["job_config"].as_object().unwrap();
        assert!(
            !job_config.contains_key("secrets.github_pat"),
            "secrets.github_pat should not be persisted"
        );
        assert!(
            !job_config.contains_key("secrets.api_key"),
            "secrets.api_key should not be persisted"
        );

        // Non-secret config should still be present
        assert_eq!(job_config.get("stage").unwrap(), "implement");
        assert_eq!(job_config.get("model").unwrap(), "claude-sonnet-4-20250514");

        // Full state file should not contain secret values
        let raw = tokio::fs::read_to_string(env.home.join("drone_state.json"))
            .await
            .unwrap();
        assert!(!raw.contains("ghp_supersecret"), "PAT leaked to disk");
        assert!(!raw.contains("sk-secret123"), "API key leaked to disk");

        drone.teardown(&env).await;
    }

    #[tokio::test]
    async fn smoke_test_event_bridge_integration() {
        use crate::event_bridge::DroneEventBridge;
        use drone_sdk::protocol::DroneEvent;
        use runtime::event::{EventSink, RuntimeEvent};

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let bridge = DroneEventBridge::new(tx, PathBuf::from("/tmp"), "smoke-run".into());

        // Simulate a series of events like a real execution
        bridge.emit(RuntimeEvent::TurnStart {
            task: "implement feature".into(),
        });
        bridge.emit(RuntimeEvent::Heartbeat);
        bridge.emit(RuntimeEvent::ToolUseStart {
            id: "t1".into(),
            name: "bash".into(),
            input: json!({"command": "cargo test"}),
        });
        bridge.emit(RuntimeEvent::ToolUseEnd {
            id: "t1".into(),
            name: "bash".into(),
            result: runtime::tools::ToolResult::success("ok".into()),
            duration_ms: 500,
        });
        bridge.emit(RuntimeEvent::Usage(runtime::api::TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 0,
        }));

        // Collect all messages
        drop(bridge);
        let mut messages = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            messages.push(msg);
        }

        assert_eq!(messages.len(), 5);

        // Verify first message is TaskStarted event
        assert!(matches!(
            &messages[0],
            DroneMessage::Event(DroneEvent::TaskStarted { .. })
        ));
        // Second is heartbeat progress
        assert!(matches!(&messages[1], DroneMessage::Progress(p) if p.status == "heartbeat"));
        // Third is tool_use progress
        assert!(matches!(&messages[2], DroneMessage::Progress(p) if p.status == "tool_use"));
        // Fourth is ToolUse event
        assert!(matches!(
            &messages[3],
            DroneMessage::Event(DroneEvent::ToolUse {
                duration_ms: 500,
                ..
            })
        ));
        // Fifth is TokenUsage event
        assert!(matches!(
            &messages[4],
            DroneMessage::Event(DroneEvent::TokenUsage { input: 1000, .. })
        ));

        // Verify all messages serialize correctly (roundtrip)
        for msg in &messages {
            let json = serde_json::to_string(msg).unwrap();
            let _: DroneMessage = serde_json::from_str(&json).unwrap();
        }
    }
}
