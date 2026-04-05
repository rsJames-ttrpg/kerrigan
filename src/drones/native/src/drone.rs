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
    conversation::loop_core::ConversationLoop,
    permission::PermissionPolicy,
    tools,
};
use serde_json::json;

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
            toml::from_str("")?
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
            // SAFETY: setup runs single-threaded before spawning any tasks
            unsafe { std::env::set_var(key, value) };
        }

        // 9. Persist state for execute phase
        let state = serde_json::json!({
            "task": &job.task,
            "job_run_id": &job.job_run_id,
            "repo_url": &job.repo_url,
            "job_config": &job_config,
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
            toml::from_str("")?
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

        // 7. Create API client
        let api_client = api::create_client(&config.provider);
        let api_client_factory = Arc::new(DefaultApiClientFactory::new(config.provider.clone()));

        // 8. Create conversation loop
        channel.progress("running", "starting agent loop")?;
        let mut conversation = ConversationLoop::new(
            api_client,
            api_client_factory,
            registry,
            config.loop_config,
            bridge.clone(),
            system_prompt,
            PermissionPolicy::allow_all(),
            env.workspace.clone(),
        );

        // 9. Run the agent loop
        let turn_result = conversation.run_turn(&task).await;

        // 10. Drain event bridge messages and forward to Queen
        // Drop the bridge sender to close the channel
        drop(bridge);
        while let Ok(msg) = rx.try_recv() {
            if let Err(e) = channel.send(&msg) {
                tracing::warn!("Failed to forward event to queen: {e}");
            }
        }

        // Handle turn result
        let turn_result = turn_result?;
        tracing::info!(
            iterations = turn_result.iterations,
            compacted = turn_result.compacted,
            "Agent loop completed"
        );

        // 11. Check exit conditions
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

        // 12. Handle git finalization - create PR if configured
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

        // 13. Get current git branch
        let branch = get_current_branch(&env.workspace).await;

        // 14. Build DroneOutput
        let session_value = serde_json::to_value(conversation.session()).unwrap_or(json!({}));

        Ok(DroneOutput {
            exit_code: if all_met { 0 } else { 1 },
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
    let _ = tokio::process::Command::new("git")
        .args(["push", "-u", "origin", "HEAD"])
        .current_dir(workspace)
        .output()
        .await;

    // Create PR
    let output = tokio::process::Command::new("gh")
        .args([
            "pr",
            "create",
            "--fill",
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
}
