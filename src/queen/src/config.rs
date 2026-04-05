use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "queen", about = "Queen hatchery manager")]
pub struct Cli {
    /// Path to config file
    #[arg(long, default_value = "hatchery.toml")]
    pub config: PathBuf,

    /// Hatchery name
    #[arg(long, env = "QUEEN_NAME")]
    pub name: Option<String>,

    /// Overseer base URL
    #[arg(long, env = "QUEEN_OVERSEER_URL")]
    pub overseer_url: Option<String>,

    /// Maximum concurrent drones
    #[arg(long, env = "QUEEN_MAX_CONCURRENCY")]
    pub max_concurrency: Option<i32>,

    /// Directory where drone workspaces are created
    #[arg(long, env = "QUEEN_DRONE_DIR")]
    pub drone_dir: Option<String>,
}

fn default_overseer_url() -> String {
    "http://localhost:3100".to_string()
}

fn default_heartbeat_interval() -> u64 {
    30
}

fn default_poll_interval() -> u64 {
    10
}

fn default_max_concurrency() -> i32 {
    4
}

fn default_drone_timeout() -> String {
    "2h".to_string()
}

fn default_stall_threshold() -> u64 {
    300
}

fn default_creep_binary() -> String {
    "./creep".to_string()
}

fn default_health_port() -> u16 {
    9090
}

fn default_restart_delay() -> u64 {
    5
}

fn default_creep_enabled() -> bool {
    true
}

fn default_notification_backend() -> String {
    "log".to_string()
}

fn default_drone_dir() -> String {
    "./drones".to_string()
}

#[derive(Debug, Deserialize)]
pub struct QueenConfig {
    pub name: String,
    #[serde(default = "default_overseer_url")]
    pub overseer_url: String,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval: u64,
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: i32,
    #[serde(default = "default_drone_timeout")]
    pub drone_timeout: String,
    #[serde(default = "default_stall_threshold")]
    pub stall_threshold: u64,
    #[serde(default = "default_drone_dir")]
    pub drone_dir: String,
    /// Default repo_url injected into jobs that don't specify one.
    pub default_repo_url: Option<String>,
}

/// Configuration for a single LSP server within hatchery.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct LspServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub extensions: Vec<String>,
    pub language_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreepConfig {
    #[serde(default = "default_creep_enabled")]
    pub enabled: bool,
    #[serde(default = "default_creep_binary")]
    pub binary: String,
    #[serde(default = "default_health_port")]
    pub health_port: u16,
    #[serde(default = "default_restart_delay")]
    pub restart_delay: u64,
    /// LSP server configurations, keyed by server name.
    #[serde(default)]
    pub lsp: HashMap<String, LspServerConfig>,
}

impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            enabled: default_creep_enabled(),
            binary: default_creep_binary(),
            health_port: default_health_port(),
            restart_delay: default_restart_delay(),
            lsp: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_notification_backend")]
    pub backend: String,
    pub url: Option<String>,
    pub token: Option<String>,
    pub events: Option<Vec<String>>,
    pub body: Option<serde_json::Value>,
    #[serde(default)]
    pub tls_skip_verify: bool,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            backend: default_notification_backend(),
            url: None,
            token: None,
            events: None,
            body: None,
            tls_skip_verify: false,
        }
    }
}

fn default_evolution_enabled() -> bool {
    false
}

fn default_min_sessions() -> usize {
    5
}

fn default_run_interval() -> usize {
    10
}

fn default_time_interval() -> String {
    "24h".to_string()
}

fn default_evolution_definition() -> String {
    "evolve-from-analysis".to_string()
}

#[derive(Debug, Deserialize)]
pub struct EvolutionConfig {
    #[serde(default = "default_evolution_enabled")]
    pub enabled: bool,
    #[serde(default = "default_min_sessions")]
    pub min_sessions: usize,
    #[serde(default = "default_run_interval")]
    pub run_interval: usize,
    #[serde(default = "default_time_interval")]
    pub time_interval: String,
    #[serde(default = "default_evolution_definition")]
    pub drone_definition: String,
    /// Target repo for evolution drone issue creation.
    pub repo_url: Option<String>,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            enabled: default_evolution_enabled(),
            min_sessions: default_min_sessions(),
            run_interval: default_run_interval(),
            time_interval: default_time_interval(),
            drone_definition: default_evolution_definition(),
            repo_url: None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub queen: QueenConfig,
    #[serde(default)]
    pub creep: CreepConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub evolution: EvolutionConfig,
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let contents =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Config =
            toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.queen.name.is_empty() {
            anyhow::bail!("queen.name must not be empty");
        }
        if self.queen.max_concurrency <= 0 {
            anyhow::bail!("queen.max_concurrency must be greater than 0");
        }
        if self.queen.heartbeat_interval == 0 {
            anyhow::bail!("queen.heartbeat_interval must be greater than 0");
        }
        if self.queen.poll_interval == 0 {
            anyhow::bail!("queen.poll_interval must be greater than 0");
        }
        if self.evolution.enabled {
            if self.evolution.min_sessions == 0 {
                anyhow::bail!("evolution.min_sessions must be greater than 0");
            }
            if crate::parse_duration(&self.evolution.time_interval).is_err() {
                anyhow::bail!(
                    "evolution.time_interval '{}' is not a valid duration (use e.g. '24h', '30m', '60s')",
                    self.evolution.time_interval
                );
            }
        }
        if self.notifications.backend == "webhook" {
            if self.notifications.url.as_ref().is_none_or(|u| u.is_empty()) {
                anyhow::bail!("notifications.url is required for webhook backend");
            }
            if let Some(events) = &self.notifications.events {
                use crate::notifier::webhook::VALID_EVENTS;
                for e in events {
                    if !VALID_EVENTS.contains(&e.as_str()) {
                        anyhow::bail!("unknown notification event: '{e}'");
                    }
                }
            }
        }
        Ok(())
    }

    pub fn apply_overrides(&mut self, cli: &Cli) {
        if let Some(name) = &cli.name {
            self.queen.name = name.clone();
        }
        if let Some(url) = &cli.overseer_url {
            self.queen.overseer_url = url.clone();
        }
        if let Some(max) = cli.max_concurrency {
            self.queen.max_concurrency = max;
        }
        if let Some(dir) = &cli.drone_dir {
            self.queen.drone_dir = dir.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_toml(contents: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{}", contents).unwrap();
        f
    }

    #[test]
    fn test_parse_minimal_config() {
        let f = write_toml(
            r#"
[queen]
name = "test-hatchery"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.queen.name, "test-hatchery");
        assert_eq!(config.queen.overseer_url, "http://localhost:3100");
        assert_eq!(config.queen.heartbeat_interval, 30);
        assert_eq!(config.queen.poll_interval, 10);
        assert_eq!(config.queen.max_concurrency, 4);
        assert_eq!(config.queen.drone_timeout, "2h");
        assert_eq!(config.queen.stall_threshold, 300);
        assert!(config.creep.enabled);
        assert_eq!(config.creep.binary, "./creep");
        assert_eq!(config.creep.health_port, 9090);
        assert_eq!(config.creep.restart_delay, 5);
        assert_eq!(config.notifications.backend, "log");
        assert_eq!(config.queen.drone_dir, "./drones");
    }

    #[test]
    fn test_parse_full_config() {
        let f = write_toml(
            r#"
[queen]
name = "prod-hatchery"
overseer_url = "http://overseer:3100"
heartbeat_interval = 60
poll_interval = 5
max_concurrency = 8
drone_timeout = "4h"
stall_threshold = 600

[creep]
enabled = false
binary = "/usr/local/bin/creep"
health_port = 9999
restart_delay = 10

[notifications]
backend = "slack"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.queen.name, "prod-hatchery");
        assert_eq!(config.queen.overseer_url, "http://overseer:3100");
        assert_eq!(config.queen.heartbeat_interval, 60);
        assert_eq!(config.queen.poll_interval, 5);
        assert_eq!(config.queen.max_concurrency, 8);
        assert_eq!(config.queen.drone_timeout, "4h");
        assert_eq!(config.queen.stall_threshold, 600);
        assert!(!config.creep.enabled);
        assert_eq!(config.creep.binary, "/usr/local/bin/creep");
        assert_eq!(config.creep.health_port, 9999);
        assert_eq!(config.creep.restart_delay, 10);
        assert_eq!(config.notifications.backend, "slack");
    }

    #[test]
    fn test_validate_empty_name() {
        let f = write_toml(
            r#"
[queen]
name = ""
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(err.to_string().contains("queen.name must not be empty"));
    }

    #[test]
    fn test_validate_zero_max_concurrency() {
        let f = write_toml(
            r#"
[queen]
name = "test"
max_concurrency = 0
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.max_concurrency must be greater than 0")
        );
    }

    #[test]
    fn test_validate_zero_heartbeat_interval() {
        let f = write_toml(
            r#"
[queen]
name = "test"
heartbeat_interval = 0
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.heartbeat_interval must be greater than 0")
        );
    }

    #[test]
    fn test_validate_zero_poll_interval() {
        let f = write_toml(
            r#"
[queen]
name = "test"
poll_interval = 0
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.poll_interval must be greater than 0")
        );
    }

    #[test]
    fn test_validate_negative_max_concurrency() {
        let f = write_toml(
            r#"
[queen]
name = "test"
max_concurrency = -1
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.max_concurrency must be greater than 0")
        );
    }

    #[test]
    fn test_parse_webhook_notifications() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
url = "http://localhost:8080/v2/send"
token = "my-secret-token"
events = ["drone_failed", "drone_stalled", "drone_timed_out"]

[notifications.body]
message = "{{message}}"
number = "+1234567890"
recipients = ["+0987654321"]
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.notifications.backend, "webhook");
        assert_eq!(
            config.notifications.url.as_deref(),
            Some("http://localhost:8080/v2/send")
        );
        assert_eq!(
            config.notifications.token.as_deref(),
            Some("my-secret-token")
        );
        assert_eq!(
            config.notifications.events,
            Some(vec![
                "drone_failed".to_string(),
                "drone_stalled".to_string(),
                "drone_timed_out".to_string()
            ])
        );
        assert!(config.notifications.body.is_some());
        let body = config.notifications.body.unwrap();
        assert_eq!(body["message"], "{{message}}");
        assert_eq!(body["number"], "+1234567890");
    }

    #[test]
    fn test_log_backend_ignores_webhook_fields() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[notifications]
backend = "log"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.notifications.backend, "log");
        assert!(config.notifications.url.is_none());
        assert!(config.notifications.token.is_none());
        assert!(config.notifications.events.is_none());
        assert!(config.notifications.body.is_none());
    }

    #[test]
    fn test_validate_webhook_missing_url() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(err.to_string().contains("notifications.url is required"));
    }

    #[test]
    fn test_validate_webhook_invalid_event() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
url = "http://localhost:8080"
events = ["drone_failed", "not_a_real_event"]
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(err.to_string().contains("unknown notification event"));
    }

    #[test]
    fn test_parse_evolution_config() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[evolution]
enabled = true
min_sessions = 10
run_interval = 20
time_interval = "12h"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert!(config.evolution.enabled);
        assert_eq!(config.evolution.min_sessions, 10);
        assert_eq!(config.evolution.run_interval, 20);
        assert_eq!(config.evolution.time_interval, "12h");
        assert_eq!(config.evolution.drone_definition, "evolve-from-analysis");
    }

    #[test]
    fn test_evolution_defaults_disabled() {
        let f = write_toml(
            r#"
[queen]
name = "test"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert!(!config.evolution.enabled);
        assert_eq!(config.evolution.min_sessions, 5);
    }

    #[test]
    fn test_cli_overrides() {
        let f = write_toml(
            r#"
[queen]
name = "base-hatchery"
overseer_url = "http://localhost:3100"
max_concurrency = 4
"#,
        );
        let mut config = Config::load(f.path()).unwrap();

        let cli = Cli {
            config: PathBuf::from("hatchery.toml"),
            name: Some("override-name".to_string()),
            overseer_url: Some("http://other:3100".to_string()),
            max_concurrency: Some(16),
            drone_dir: None,
        };

        config.apply_overrides(&cli);

        assert_eq!(config.queen.name, "override-name");
        assert_eq!(config.queen.overseer_url, "http://other:3100");
        assert_eq!(config.queen.max_concurrency, 16);
    }

    #[test]
    fn test_creep_lsp_config_empty_by_default() {
        let f = write_toml(
            r#"
[queen]
name = "test"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert!(config.creep.lsp.is_empty());
    }

    #[test]
    fn test_creep_lsp_config_parsing() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[creep.lsp.rust]
command = "rust-analyzer"
args = []
extensions = [".rs"]
language_id = "rust"

[creep.lsp.typescript]
command = "typescript-language-server"
args = ["--stdio"]
extensions = [".ts", ".tsx"]
language_id = "typescript"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.creep.lsp.len(), 2);

        let rust = &config.creep.lsp["rust"];
        assert_eq!(rust.command, "rust-analyzer");
        assert!(rust.args.is_empty());
        assert_eq!(rust.extensions, vec![".rs"]);
        assert_eq!(rust.language_id, "rust");

        let ts = &config.creep.lsp["typescript"];
        assert_eq!(ts.command, "typescript-language-server");
        assert_eq!(ts.args, vec!["--stdio"]);
        assert_eq!(ts.extensions, vec![".ts", ".tsx"]);
        assert_eq!(ts.language_id, "typescript");
    }
}
