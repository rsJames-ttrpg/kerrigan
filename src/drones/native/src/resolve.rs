use std::collections::HashMap;
use std::path::PathBuf;

use runtime::api::ProviderConfig;
use runtime::conversation::loop_core::{CompactionStrategy, LoopConfig};

use crate::config::{CacheSection, DroneConfig};
use crate::pipeline::{Stage, StageConfig};

/// Fully resolved configuration after merging all layers:
/// compiled defaults → drone.toml → job spec overrides → stage defaults
pub struct ResolvedConfig {
    pub provider: ProviderConfig,
    pub loop_config: LoopConfig,
    pub stage_config: StageConfig,
    pub cache: CacheConfig,
    pub environment: EnvironmentConfig,
    pub orchestrator: OrchestratorConfig,
}

/// Resolved cache configuration
pub struct CacheConfig {
    pub dir: PathBuf,
    pub repo_cache: bool,
    pub tool_cache: bool,
    pub max_size_mb: u64,
}

/// Resolved environment configuration
pub struct EnvironmentConfig {
    pub extra_path: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Resolved orchestrator configuration
pub struct OrchestratorConfig {
    pub test_command: Option<String>,
    pub max_fixup_iterations: u32,
    pub max_parallel: usize,
}

impl ResolvedConfig {
    /// Merge all config layers: drone.toml base, job spec overrides, and stage defaults.
    ///
    /// Priority (highest wins):
    /// 1. Stage defaults (for stage-specific fields like allowed_tools, git ops)
    /// 2. Job spec overrides (operator-provided per-run)
    /// 3. drone.toml values
    /// 4. Compiled defaults (via serde defaults)
    pub fn resolve(
        drone_toml: &DroneConfig,
        job_config: &HashMap<String, String>,
        stage: Stage,
    ) -> Self {
        // Start from drone.toml provider config
        let mut provider = drone_toml.to_provider_config();

        // Override model from job spec
        if let Some(model) = job_config.get("model") {
            match &mut provider {
                ProviderConfig::Anthropic { model: m, .. } => *m = model.clone(),
                ProviderConfig::OpenAiCompat { model: m, .. } => *m = model.clone(),
            }
        }

        // Override API key from job spec (secrets.api_key)
        if let Some(api_key) = job_config.get("secrets.api_key") {
            match &mut provider {
                ProviderConfig::Anthropic { api_key: k, .. } => *k = api_key.clone(),
                ProviderConfig::OpenAiCompat { api_key: k, .. } => {
                    *k = Some(api_key.clone());
                }
            }
        }

        // Build loop config from drone.toml, with job spec overrides
        let max_iterations = job_config
            .get("max_iterations")
            .and_then(|v| v.parse().ok())
            .unwrap_or(drone_toml.runtime.max_iterations);

        let max_tokens = job_config
            .get("max_tokens")
            .and_then(|v| v.parse().ok())
            .unwrap_or(drone_toml.runtime.max_tokens);

        let temperature = job_config
            .get("temperature")
            .and_then(|v| v.parse().ok())
            .or(drone_toml.runtime.temperature);

        let compaction_preserve = drone_toml.runtime.compaction_preserve_recent;
        let compaction_strategy = match drone_toml.runtime.compaction_strategy.as_str() {
            "summarize" => CompactionStrategy::Summarize {
                preserve_recent: compaction_preserve,
            },
            _ => CompactionStrategy::Checkpoint {
                preserve_recent: compaction_preserve,
            },
        };

        let loop_config = LoopConfig {
            max_iterations,
            max_context_tokens: drone_toml.runtime.compaction_threshold_tokens,
            compaction_strategy,
            max_tokens_per_response: max_tokens,
            temperature,
        };

        // Get stage defaults, then merge git config from drone.toml
        let mut stage_config = stage.default_config();

        // Merge protected paths from drone.toml into stage git config
        for path in &drone_toml.git.protected_paths {
            if !stage_config.git.protected_paths.contains(path) {
                stage_config.git.protected_paths.push(path.clone());
            }
        }

        // Override branch from job spec
        if let Some(branch) = job_config.get("branch") {
            stage_config.git.branch_name = Some(branch.clone());
        }

        // Override max_turns from job spec
        if let Some(turns) = job_config.get("max_turns").and_then(|v| v.parse().ok()) {
            stage_config.max_turns = turns;
        }

        // Cache config from drone.toml
        let cache = CacheConfig::from(&drone_toml.cache);

        // Environment config from drone.toml, with job spec env overrides
        let mut env = drone_toml.environment.env.clone();
        for (k, v) in job_config {
            if let Some(key) = k.strip_prefix("env.") {
                env.insert(key.to_string(), v.clone());
            }
        }

        let environment = EnvironmentConfig {
            extra_path: drone_toml.environment.extra_path.clone(),
            env,
        };

        // Orchestrator config: job_config overrides drone.toml
        let orchestrator = OrchestratorConfig {
            test_command: job_config
                .get("test_command")
                .cloned()
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

        Self {
            provider,
            loop_config,
            stage_config,
            cache,
            environment,
            orchestrator,
        }
    }
}

impl From<&CacheSection> for CacheConfig {
    fn from(section: &CacheSection) -> Self {
        Self {
            dir: section.dir.clone(),
            repo_cache: section.repo_cache,
            tool_cache: section.tool_cache,
            max_size_mb: section.max_size_mb,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_drone_config() -> DroneConfig {
        toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-base"
"#,
        )
        .unwrap()
    }

    fn make_job_config(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn job_spec_model_overrides_drone_toml() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[("model", "claude-opus-4-20250514")]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        match &resolved.provider {
            ProviderConfig::Anthropic { model, .. } => {
                assert_eq!(model, "claude-opus-4-20250514");
            }
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn job_spec_api_key_overrides_drone_toml() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[("secrets.api_key", "sk-override")]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        match &resolved.provider {
            ProviderConfig::Anthropic { api_key, .. } => {
                assert_eq!(api_key, "sk-override");
            }
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn drone_toml_defaults_used_when_no_job_overrides() {
        let drone = minimal_drone_config();
        let job = HashMap::new();
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(resolved.loop_config.max_iterations, 50);
        assert_eq!(resolved.loop_config.max_tokens_per_response, 8192);
        assert!(resolved.loop_config.temperature.is_none());
    }

    #[test]
    fn job_spec_overrides_runtime_params() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[
            ("max_iterations", "30"),
            ("max_tokens", "4096"),
            ("temperature", "0.5"),
        ]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(resolved.loop_config.max_iterations, 30);
        assert_eq!(resolved.loop_config.max_tokens_per_response, 4096);
        assert_eq!(resolved.loop_config.temperature, Some(0.5));
    }

    #[test]
    fn stage_defaults_apply_correctly() {
        let drone = minimal_drone_config();
        let job = HashMap::new();

        let spec = ResolvedConfig::resolve(&drone, &job, Stage::Spec);
        assert!(spec.stage_config.denied_tools.contains(&"bash".to_string()));
        assert_eq!(spec.stage_config.max_turns, 25);

        let implement = ResolvedConfig::resolve(&drone, &job, Stage::Implement);
        assert!(implement.stage_config.denied_tools.is_empty());
        assert_eq!(implement.stage_config.max_turns, 100);
    }

    #[test]
    fn protected_paths_merge_from_drone_toml_and_stage() {
        let drone: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[git]
protected_paths = [".buckconfig", "Cargo.lock"]
"#,
        )
        .unwrap();

        let job = HashMap::new();
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        // Implement stage protects CLAUDE.md by default
        assert!(
            resolved
                .stage_config
                .git
                .protected_paths
                .contains(&"CLAUDE.md".to_string())
        );
        // drone.toml paths are merged in
        assert!(
            resolved
                .stage_config
                .git
                .protected_paths
                .contains(&".buckconfig".to_string())
        );
        assert!(
            resolved
                .stage_config
                .git
                .protected_paths
                .contains(&"Cargo.lock".to_string())
        );
    }

    #[test]
    fn job_spec_branch_overrides_stage_default() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[("branch", "feat/my-feature")]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(
            resolved.stage_config.git.branch_name,
            Some("feat/my-feature".to_string())
        );
    }

    #[test]
    fn job_spec_max_turns_overrides_stage_default() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[("max_turns", "200")]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(resolved.stage_config.max_turns, 200);
    }

    #[test]
    fn environment_env_overrides_from_job_spec() {
        let drone: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[environment.env]
RUST_LOG = "info"
BASE_VAR = "base"
"#,
        )
        .unwrap();

        let job = make_job_config(&[("env.RUST_LOG", "debug"), ("env.NEW_VAR", "new")]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(resolved.environment.env["RUST_LOG"], "debug");
        assert_eq!(resolved.environment.env["BASE_VAR"], "base");
        assert_eq!(resolved.environment.env["NEW_VAR"], "new");
    }

    #[test]
    fn compaction_strategy_summarize() {
        let drone: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[runtime]
compaction_strategy = "summarize"
compaction_preserve_recent = 8
"#,
        )
        .unwrap();

        let resolved = ResolvedConfig::resolve(&drone, &HashMap::new(), Stage::Implement);
        match resolved.loop_config.compaction_strategy {
            CompactionStrategy::Summarize { preserve_recent } => {
                assert_eq!(preserve_recent, 8);
            }
            _ => panic!("expected Summarize"),
        }
    }

    #[test]
    fn compaction_strategy_checkpoint_default() {
        let drone = minimal_drone_config();
        let resolved = ResolvedConfig::resolve(&drone, &HashMap::new(), Stage::Implement);
        match resolved.loop_config.compaction_strategy {
            CompactionStrategy::Checkpoint { preserve_recent } => {
                assert_eq!(preserve_recent, 6);
            }
            _ => panic!("expected Checkpoint"),
        }
    }

    #[test]
    fn cache_config_from_drone_toml() {
        let drone: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"

[cache]
dir = "/tmp/my-cache"
repo_cache = false
max_size_mb = 512
"#,
        )
        .unwrap();

        let resolved = ResolvedConfig::resolve(&drone, &HashMap::new(), Stage::Implement);
        assert_eq!(resolved.cache.dir, PathBuf::from("/tmp/my-cache"));
        assert!(!resolved.cache.repo_cache);
        assert_eq!(resolved.cache.max_size_mb, 512);
    }

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

        assert_eq!(
            resolved.orchestrator.test_command.as_deref(),
            Some("make test")
        );
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

        assert_eq!(
            resolved.orchestrator.test_command.as_deref(),
            Some("cargo test")
        );
        assert_eq!(resolved.orchestrator.max_fixup_iterations, 10);
        assert_eq!(resolved.orchestrator.max_parallel, 8);
    }

    #[test]
    fn invalid_job_spec_values_use_defaults() {
        let drone = minimal_drone_config();
        let job = make_job_config(&[
            ("max_iterations", "not-a-number"),
            ("temperature", "invalid"),
        ]);
        let resolved = ResolvedConfig::resolve(&drone, &job, Stage::Implement);

        assert_eq!(resolved.loop_config.max_iterations, 50);
        assert!(resolved.loop_config.temperature.is_none());
    }
}
