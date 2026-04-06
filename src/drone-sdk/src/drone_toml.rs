use std::collections::HashMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;

/// Shared drone.toml configuration read from the target repo's workspace root.
/// All fields are optional with sensible defaults. If the file is absent,
/// `DroneToml::default()` applies.
#[derive(Debug, Deserialize, Default)]
pub struct DroneToml {
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub setup: SetupSection,
    #[serde(default)]
    pub prompts: PromptsSection,
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

fn default_branch() -> String {
    "main".to_string()
}

fn default_prefix() -> String {
    "feat/".to_string()
}

fn default_true() -> bool {
    true
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
