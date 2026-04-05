use std::collections::HashMap;

use serde::Deserialize;

fn default_grpc_port() -> u16 {
    9090
}

fn default_symbol_index() -> bool {
    true
}

fn default_languages() -> Vec<String> {
    vec!["rust".to_string()]
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub creep: CreepConfig,
}

/// Configuration for a single LSP server.
#[derive(Debug, Clone, Deserialize)]
pub struct LspConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub extensions: Vec<String>,
    pub language_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreepConfig {
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default)]
    pub workspaces: Vec<String>,
    #[serde(default = "default_symbol_index")]
    pub symbol_index: bool,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
    /// LSP server configurations, keyed by server name.
    #[serde(default)]
    pub lsp: HashMap<String, LspConfig>,
}

impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            grpc_port: default_grpc_port(),
            workspaces: Vec::new(),
            symbol_index: default_symbol_index(),
            languages: default_languages(),
            lsp: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&contents)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let config: Config = toml::from_str("[creep]").unwrap();
        assert_eq!(config.creep.grpc_port, 9090);
        assert!(config.creep.workspaces.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[creep]
grpc_port = 8080
workspaces = ["/home/user/repo1", "/home/user/repo2"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.creep.grpc_port, 8080);
        assert_eq!(config.creep.workspaces.len(), 2);
    }

    #[test]
    fn test_empty_config_uses_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.creep.grpc_port, 9090);
    }

    #[test]
    fn test_symbol_index_config() {
        let toml_str = r#"
[creep]
grpc_port = 9090
symbol_index = false
languages = ["rust"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.creep.symbol_index);
        assert_eq!(config.creep.languages, vec!["rust"]);
    }

    #[test]
    fn test_symbol_index_defaults() {
        let config: Config = toml::from_str("[creep]").unwrap();
        assert!(config.creep.symbol_index);
        assert_eq!(config.creep.languages, vec!["rust"]);
    }

    #[test]
    fn test_lsp_config_empty_by_default() {
        let config: Config = toml::from_str("[creep]").unwrap();
        assert!(config.creep.lsp.is_empty());
    }

    #[test]
    fn test_lsp_config_parsing() {
        let toml_str = r#"
[creep]
grpc_port = 9090

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
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
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
