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
}

impl Default for CreepConfig {
    fn default() -> Self {
        Self {
            grpc_port: default_grpc_port(),
            workspaces: Vec::new(),
            symbol_index: default_symbol_index(),
            languages: default_languages(),
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
}
