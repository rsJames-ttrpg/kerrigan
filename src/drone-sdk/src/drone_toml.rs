use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// Shared drone.toml configuration read from the target repo's workspace root.
/// All fields are optional with sensible defaults. If the file is absent,
/// `DroneToml::default()` applies.
#[derive(Debug, Deserialize, Default)]
pub struct DroneToml {
    #[serde(default)]
    pub provider: Option<ProviderSection>,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub cache: CacheSection,
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub setup: SetupSection,
    #[serde(default)]
    pub prompts: PromptsSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub mcp: HashMap<String, McpSection>,
    #[serde(default)]
    pub environment: EnvironmentSection,
    #[serde(default)]
    pub orchestrator: OrchestratorSection,
    #[serde(default)]
    pub health_checks: Vec<CustomHealthCheck>,
}

#[derive(Debug, Deserialize)]
pub struct GitSection {
    #[serde(default = "default_branch")]
    pub default_branch: String,
    #[serde(default = "default_prefix")]
    pub branch_prefix: String,
    #[serde(default = "default_true")]
    pub auto_commit: bool,
    #[serde(default = "default_true")]
    pub pr_on_complete: bool,
    #[serde(default)]
    pub protected_paths: Vec<String>,
    #[serde(default)]
    pub identity: HashMap<String, IdentitySection>,
}

impl Default for GitSection {
    fn default() -> Self {
        Self {
            default_branch: default_branch(),
            branch_prefix: default_prefix(),
            auto_commit: default_true(),
            pr_on_complete: default_true(),
            protected_paths: Vec::new(),
            identity: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct IdentitySection {
    pub user_name: String,
    pub user_email: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SetupSection {
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PromptsSection {
    #[serde(default)]
    pub extra_rules: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderSection {
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RuntimeSection {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_compaction_strategy")]
    pub compaction_strategy: String,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold_tokens: u32,
    #[serde(default = "default_compaction_preserve")]
    pub compaction_preserve_recent: u32,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            max_iterations: default_max_iterations(),
            temperature: None,
            timeout_secs: default_timeout(),
            compaction_strategy: default_compaction_strategy(),
            compaction_threshold_tokens: default_compaction_threshold(),
            compaction_preserve_recent: default_compaction_preserve(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CacheSection {
    #[serde(default = "default_cache_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_true")]
    pub repo_cache: bool,
    #[serde(default = "default_true")]
    pub tool_cache: bool,
    #[serde(default = "default_cache_size")]
    pub max_size_mb: u64,
}

impl Default for CacheSection {
    fn default() -> Self {
        Self {
            dir: default_cache_dir(),
            repo_cache: true,
            tool_cache: true,
            max_size_mb: default_cache_size(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolsSection {
    #[serde(default = "default_true")]
    pub sandbox: bool,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub external: HashMap<String, ExternalToolSection>,
}

impl Default for ToolsSection {
    fn default() -> Self {
        Self {
            sandbox: true,
            allowed: vec![],
            denied: vec![],
            external: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExternalToolSection {
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub description: String,
    #[serde(default)]
    pub input_schema_path: Option<String>,
    #[serde(default = "default_permission")]
    pub permission: String,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub embedded: bool,
    #[serde(default = "default_tool_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpSection {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EnvironmentSection {
    #[serde(default)]
    pub extra_path: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl Default for EnvironmentSection {
    fn default() -> Self {
        Self {
            extra_path: vec![],
            env: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CustomHealthCheck {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_health_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
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

fn default_branch() -> String {
    "main".to_string()
}

fn default_prefix() -> String {
    "feat/".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_max_iterations() -> u32 {
    50
}

fn default_timeout() -> u64 {
    7200
}

fn default_compaction_strategy() -> String {
    "checkpoint".into()
}

fn default_compaction_threshold() -> u32 {
    80000
}

fn default_compaction_preserve() -> u32 {
    6
}

fn default_cache_dir() -> PathBuf {
    PathBuf::from("/var/cache/kerrigan/drone")
}

fn default_cache_size() -> u64 {
    2048
}

fn default_permission() -> String {
    "read-only".into()
}

fn default_output_format() -> String {
    "markdown".into()
}

fn default_tool_timeout() -> u64 {
    30
}

fn default_health_timeout() -> u64 {
    30
}

fn default_max_fixup_iterations() -> u32 {
    5
}

fn default_max_parallel() -> usize {
    2
}

impl DroneToml {
    /// Load drone.toml from a workspace directory. Returns `Ok(default)` if
    /// the file doesn't exist. Returns `Err` only on parse failures or validation errors.
    pub fn load(workspace: &Path) -> anyhow::Result<Self> {
        let path = workspace.join("drone.toml");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(e).context(format!("failed to read drone.toml at {}", path.display()));
            }
        };
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("failed to parse drone.toml at {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate config values after deserialization.
    fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.git.default_branch.is_empty(),
            "git.default_branch must not be empty"
        );
        anyhow::ensure!(
            !self.git.branch_prefix.is_empty(),
            "git.branch_prefix must not be empty"
        );
        for (drone_type, id) in &self.git.identity {
            anyhow::ensure!(
                !id.user_name.is_empty(),
                "git.identity.{drone_type}.user_name must not be empty"
            );
            anyhow::ensure!(
                !id.user_email.is_empty(),
                "git.identity.{drone_type}.user_email must not be empty"
            );
            anyhow::ensure!(
                !id.user_name.contains('\n'),
                "git.identity.{drone_type}.user_name must not contain newlines"
            );
            anyhow::ensure!(
                !id.user_email.contains('\n'),
                "git.identity.{drone_type}.user_email must not contain newlines"
            );
        }
        Ok(())
    }

    /// Get the git identity for a specific drone type, with fallback defaults.
    pub fn git_identity(&self, drone_type: &str) -> IdentitySection {
        self.git
            .identity
            .get(drone_type)
            .cloned()
            .unwrap_or_else(|| IdentitySection {
                user_name: format!("{drone_type}-drone"),
                user_email: format!("{drone_type}-drone@noreply"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[git]
default_branch = "develop"
branch_prefix = "kerrigan/"
auto_commit = false
pr_on_complete = true
protected_paths = ["README.md"]

[git.identity.claude]
user_name = "claude-bot"
user_email = "claude@myorg.com"

[git.identity.native]
user_name = "native-bot"
user_email = "native@myorg.com"

[setup]
commands = ["./tools/setup-hooks.sh", "npm install"]

[prompts]
extra_rules = """
## Build
Use buck2 build, not cargo build.
"""
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        assert_eq!(config.git.default_branch, "develop");
        assert_eq!(config.git.branch_prefix, "kerrigan/");
        assert!(!config.git.auto_commit);
        assert_eq!(config.git.protected_paths, vec!["README.md"]);

        let claude_id = config.git_identity("claude");
        assert_eq!(claude_id.user_name, "claude-bot");
        assert_eq!(claude_id.user_email, "claude@myorg.com");

        let native_id = config.git_identity("native");
        assert_eq!(native_id.user_name, "native-bot");

        assert_eq!(config.setup.commands.len(), 2);
        assert!(config.prompts.extra_rules.contains("buck2 build"));
    }

    #[test]
    fn parse_minimal_config() {
        let config: DroneToml = toml::from_str("").unwrap();
        assert_eq!(config.git.default_branch, "main");
        assert_eq!(config.git.branch_prefix, "feat/");
        assert!(config.git.auto_commit);
        assert!(config.git.pr_on_complete);
        assert!(config.setup.commands.is_empty());
        assert!(config.prompts.extra_rules.is_empty());
    }

    #[test]
    fn identity_fallback_for_unknown_type() {
        let config: DroneToml = toml::from_str("").unwrap();
        let id = config.git_identity("claude");
        assert_eq!(id.user_name, "claude-drone");
        assert_eq!(id.user_email, "claude-drone@noreply");
    }

    #[test]
    fn identity_with_partial_config() {
        let toml_str = r#"
[git.identity.claude]
user_name = "my-claude"
user_email = "claude@example.com"
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        let claude = config.git_identity("claude");
        assert_eq!(claude.user_name, "my-claude");

        // native not defined — falls back
        let native = config.git_identity("native");
        assert_eq!(native.user_name, "native-drone");
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = std::path::PathBuf::from("/tmp/nonexistent-workspace-test");
        let config = DroneToml::load(&dir).unwrap();
        assert_eq!(config.git.default_branch, "main");
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("drone.toml"), "not = [valid toml").unwrap();
        let result = DroneToml::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_empty_identity_name() {
        let toml_str = r#"
[git.identity.claude]
user_name = ""
user_email = "claude@example.com"
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_newline_in_identity() {
        let config = DroneToml {
            git: GitSection {
                identity: {
                    let mut m = HashMap::new();
                    m.insert(
                        "claude".to_string(),
                        IdentitySection {
                            user_name: "claude\ninjection".to_string(),
                            user_email: "claude@example.com".to_string(),
                        },
                    );
                    m
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn parse_full_native_config() {
        let toml_str = r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-test-key"
base_url = "https://api.anthropic.com"

[runtime]
max_tokens = 4096
max_iterations = 30
temperature = 0.7
timeout_secs = 3600
compaction_strategy = "summarize"
compaction_threshold_tokens = 60000
compaction_preserve_recent = 4

[cache]
dir = "/tmp/drone-cache"
repo_cache = true
tool_cache = false
max_size_mb = 1024

[git]
default_branch = "develop"
branch_prefix = "drone/"
auto_commit = false
pr_on_complete = true
protected_paths = ["CLAUDE.md", ".buckconfig"]

[git.identity.claude]
user_name = "claude-bot"
user_email = "claude@myorg.com"

[tools]
sandbox = true
allowed = ["read_file", "write_file"]
denied = ["bash"]

[tools.external.creep]
binary = "creep-cli"
args = ["search"]
description = "File indexing search"
permission = "read-only"
output_format = "json"
embedded = true
timeout_secs = 10

[mcp.overseer]
kind = "http"
url = "http://localhost:3100/mcp"

[environment]
extra_path = ["/usr/local/bin"]

[environment.env]
RUST_LOG = "debug"

[orchestrator]
test_command = "cargo test --workspace"
max_fixup_iterations = 3
max_parallel = 4

[[health_checks]]
name = "cargo-check"
command = "cargo"
args = ["check"]
required = true
timeout_secs = 60

[setup]
commands = ["./tools/setup-hooks.sh"]

[prompts]
extra_rules = "Use buck2 build"
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();

        let provider = config.provider.as_ref().unwrap();
        assert_eq!(provider.kind, "anthropic");
        assert_eq!(provider.model, "claude-sonnet-4-20250514");
        assert_eq!(provider.api_key.as_deref(), Some("sk-test-key"));

        assert_eq!(config.runtime.max_tokens, 4096);
        assert_eq!(config.runtime.max_iterations, 30);
        assert_eq!(config.runtime.temperature, Some(0.7));
        assert_eq!(config.runtime.compaction_strategy, "summarize");

        assert_eq!(config.cache.dir, PathBuf::from("/tmp/drone-cache"));
        assert!(!config.cache.tool_cache);
        assert_eq!(config.cache.max_size_mb, 1024);

        assert_eq!(config.git.default_branch, "develop");
        let claude_id = config.git_identity("claude");
        assert_eq!(claude_id.user_name, "claude-bot");

        assert!(config.tools.sandbox);
        assert_eq!(config.tools.allowed, vec!["read_file", "write_file"]);
        assert_eq!(config.tools.denied, vec!["bash"]);
        let creep = &config.tools.external["creep"];
        assert_eq!(creep.binary, "creep-cli");
        assert!(creep.embedded);

        assert_eq!(config.mcp["overseer"].kind, "http");
        assert_eq!(config.mcp["overseer"].url, "http://localhost:3100/mcp");

        assert_eq!(config.environment.extra_path, vec!["/usr/local/bin"]);
        assert_eq!(config.environment.env["RUST_LOG"], "debug");

        assert_eq!(
            config.orchestrator.test_command.as_deref(),
            Some("cargo test --workspace")
        );
        assert_eq!(config.orchestrator.max_fixup_iterations, 3);
        assert_eq!(config.orchestrator.max_parallel, 4);

        assert_eq!(config.health_checks.len(), 1);
        assert_eq!(config.health_checks[0].name, "cargo-check");

        assert_eq!(config.setup.commands, vec!["./tools/setup-hooks.sh"]);
        assert!(config.prompts.extra_rules.contains("buck2 build"));
    }

    #[test]
    fn parse_no_provider_uses_none() {
        let config: DroneToml = toml::from_str("").unwrap();
        assert!(config.provider.is_none());
        assert_eq!(config.runtime.max_tokens, 8192);
        assert_eq!(config.runtime.max_iterations, 50);
        assert_eq!(config.cache.dir, PathBuf::from("/var/cache/kerrigan/drone"));
        assert!(config.tools.sandbox);
        assert!(config.mcp.is_empty());
        assert!(config.health_checks.is_empty());
        assert_eq!(config.orchestrator.max_fixup_iterations, 5);
        assert_eq!(config.orchestrator.max_parallel, 2);
    }

    #[test]
    fn parse_repo_drone_toml_format() {
        let toml_str = r#"
[git]
default_branch = "main"

[git.identity.claude]
user_name = "claude-drone"
user_email = "claude-drone@noreply"

[git.identity.native]
user_name = "native-drone"
user_email = "native-drone@noreply"

[setup]
commands = ["./tools/setup-hooks.sh"]

[prompts]
extra_rules = """
## Build & Test
Use buck2 build, not cargo build.
"""
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        assert!(config.provider.is_none());
        assert_eq!(config.git.default_branch, "main");
        assert_eq!(config.setup.commands, vec!["./tools/setup-hooks.sh"]);
        assert!(config.prompts.extra_rules.contains("buck2 build"));
        let claude = config.git_identity("claude");
        assert_eq!(claude.user_name, "claude-drone");
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("drone.toml"),
            r#"
[git.identity.claude]
user_name = "test-claude"
user_email = "test@example.com"

[setup]
commands = ["echo hello"]
"#,
        )
        .unwrap();

        let config = DroneToml::load(dir.path()).unwrap();
        let id = config.git_identity("claude");
        assert_eq!(id.user_name, "test-claude");
        assert_eq!(config.setup.commands, vec!["echo hello"]);
    }
}
