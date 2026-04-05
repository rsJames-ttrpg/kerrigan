use std::collections::HashMap;
use std::path::PathBuf;

use runtime::api::ProviderConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DroneConfig {
    pub provider: ProviderSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub cache: CacheSection,
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub mcp: HashMap<String, McpSection>,
    #[serde(default)]
    pub environment: EnvironmentSection,
    #[serde(default)]
    pub health_checks: Vec<CustomHealthCheck>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderSection {
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
}

impl Default for GitSection {
    fn default() -> Self {
        Self {
            default_branch: default_branch(),
            branch_prefix: default_prefix(),
            auto_commit: true,
            pr_on_complete: true,
            protected_paths: vec![],
        }
    }
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct McpSection {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
fn default_true() -> bool {
    true
}
fn default_cache_size() -> u64 {
    2048
}
fn default_branch() -> String {
    "main".into()
}
fn default_prefix() -> String {
    "kerrigan/".into()
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

impl DroneConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn to_provider_config(&self) -> ProviderConfig {
        match self.provider.kind.as_str() {
            "anthropic" => ProviderConfig::Anthropic {
                api_key: self.provider.api_key.clone().unwrap_or_default(),
                model: self.provider.model.clone(),
                base_url: self.provider.base_url.clone(),
            },
            _ => ProviderConfig::OpenAiCompat {
                base_url: self
                    .provider
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434/v1".into()),
                api_key: self.provider.api_key.clone(),
                model: self.provider.model.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_CONFIG: &str = r#"
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
MY_VAR = "value"

[[health_checks]]
name = "cargo-check"
command = "cargo"
args = ["check"]
required = true
timeout_secs = 60
"#;

    #[test]
    fn parse_full_config() {
        let config: DroneConfig = toml::from_str(FULL_CONFIG).unwrap();

        // Provider
        assert_eq!(config.provider.kind, "anthropic");
        assert_eq!(config.provider.model, "claude-sonnet-4-20250514");
        assert_eq!(config.provider.api_key.as_deref(), Some("sk-test-key"));
        assert_eq!(
            config.provider.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );

        // Runtime
        assert_eq!(config.runtime.max_tokens, 4096);
        assert_eq!(config.runtime.max_iterations, 30);
        assert_eq!(config.runtime.temperature, Some(0.7));
        assert_eq!(config.runtime.timeout_secs, 3600);
        assert_eq!(config.runtime.compaction_strategy, "summarize");
        assert_eq!(config.runtime.compaction_threshold_tokens, 60000);
        assert_eq!(config.runtime.compaction_preserve_recent, 4);

        // Cache
        assert_eq!(config.cache.dir, PathBuf::from("/tmp/drone-cache"));
        assert!(config.cache.repo_cache);
        assert!(!config.cache.tool_cache);
        assert_eq!(config.cache.max_size_mb, 1024);

        // Git
        assert_eq!(config.git.default_branch, "develop");
        assert_eq!(config.git.branch_prefix, "drone/");
        assert!(!config.git.auto_commit);
        assert!(config.git.pr_on_complete);
        assert_eq!(config.git.protected_paths, vec!["CLAUDE.md", ".buckconfig"]);

        // Tools
        assert!(config.tools.sandbox);
        assert_eq!(config.tools.allowed, vec!["read_file", "write_file"]);
        assert_eq!(config.tools.denied, vec!["bash"]);
        let creep = &config.tools.external["creep"];
        assert_eq!(creep.binary, "creep-cli");
        assert_eq!(creep.args, vec!["search"]);
        assert_eq!(creep.description, "File indexing search");
        assert!(creep.embedded);
        assert_eq!(creep.timeout_secs, 10);

        // MCP
        let overseer = &config.mcp["overseer"];
        assert_eq!(overseer.kind, "http");
        assert_eq!(overseer.url, "http://localhost:3100/mcp");

        // Environment
        assert_eq!(config.environment.extra_path, vec!["/usr/local/bin"]);
        assert_eq!(config.environment.env["RUST_LOG"], "debug");
        assert_eq!(config.environment.env["MY_VAR"], "value");

        // Health checks
        assert_eq!(config.health_checks.len(), 1);
        assert_eq!(config.health_checks[0].name, "cargo-check");
        assert_eq!(config.health_checks[0].command, "cargo");
        assert_eq!(config.health_checks[0].args, vec!["check"]);
        assert!(config.health_checks[0].required);
        assert_eq!(config.health_checks[0].timeout_secs, 60);
    }

    #[test]
    fn parse_minimal_config_uses_defaults() {
        let minimal = r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
"#;
        let config: DroneConfig = toml::from_str(minimal).unwrap();

        // Defaults
        assert_eq!(config.runtime.max_tokens, 8192);
        assert_eq!(config.runtime.max_iterations, 50);
        assert_eq!(config.runtime.timeout_secs, 7200);
        assert_eq!(config.runtime.compaction_strategy, "checkpoint");
        assert_eq!(config.runtime.compaction_threshold_tokens, 80000);
        assert_eq!(config.runtime.compaction_preserve_recent, 6);
        assert!(config.runtime.temperature.is_none());

        assert_eq!(config.cache.dir, PathBuf::from("/var/cache/kerrigan/drone"));
        assert!(config.cache.repo_cache);
        assert!(config.cache.tool_cache);
        assert_eq!(config.cache.max_size_mb, 2048);

        assert_eq!(config.git.default_branch, "main");
        assert_eq!(config.git.branch_prefix, "kerrigan/");
        assert!(config.git.auto_commit);
        assert!(config.git.pr_on_complete);
        assert!(config.git.protected_paths.is_empty());

        assert!(config.tools.sandbox);
        assert!(config.tools.allowed.is_empty());
        assert!(config.tools.denied.is_empty());
        assert!(config.tools.external.is_empty());

        assert!(config.mcp.is_empty());
        assert!(config.environment.extra_path.is_empty());
        assert!(config.environment.env.is_empty());
        assert!(config.health_checks.is_empty());
    }

    #[test]
    fn to_provider_config_anthropic() {
        let config: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-test"
base_url = "https://custom.api.com"
"#,
        )
        .unwrap();

        let provider = config.to_provider_config();
        match provider {
            ProviderConfig::Anthropic {
                api_key,
                model,
                base_url,
            } => {
                assert_eq!(api_key, "sk-test");
                assert_eq!(model, "claude-sonnet-4-20250514");
                assert_eq!(base_url.as_deref(), Some("https://custom.api.com"));
            }
            _ => panic!("expected Anthropic variant"),
        }
    }

    #[test]
    fn to_provider_config_openai_compat() {
        let config: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "openai-compat"
model = "llama3"
"#,
        )
        .unwrap();

        let provider = config.to_provider_config();
        match provider {
            ProviderConfig::OpenAiCompat {
                base_url,
                api_key,
                model,
            } => {
                assert_eq!(base_url, "http://localhost:11434/v1");
                assert!(api_key.is_none());
                assert_eq!(model, "llama3");
            }
            _ => panic!("expected OpenAiCompat variant"),
        }
    }

    #[test]
    fn to_provider_config_openai_compat_with_url() {
        let config: DroneConfig = toml::from_str(
            r#"
[provider]
kind = "openai-compat"
model = "gpt-4"
base_url = "https://api.openai.com/v1"
api_key = "sk-openai"
"#,
        )
        .unwrap();

        let provider = config.to_provider_config();
        match provider {
            ProviderConfig::OpenAiCompat {
                base_url,
                api_key,
                model,
            } => {
                assert_eq!(base_url, "https://api.openai.com/v1");
                assert_eq!(api_key.as_deref(), Some("sk-openai"));
                assert_eq!(model, "gpt-4");
            }
            _ => panic!("expected OpenAiCompat variant"),
        }
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drone.toml");
        std::fs::write(
            &path,
            r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
"#,
        )
        .unwrap();

        let config = DroneConfig::load(&path).unwrap();
        assert_eq!(config.provider.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn load_nonexistent_file_errors() {
        let result = DroneConfig::load(std::path::Path::new("/nonexistent/drone.toml"));
        assert!(result.is_err());
    }
}
