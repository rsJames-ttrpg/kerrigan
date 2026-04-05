use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AbathurConfig {
    pub index: IndexConfig,
    pub sources: SourcesConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexConfig {
    pub doc_paths: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourcesConfig {
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GenerateConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            api_key_env: default_api_key_env(),
        }
    }
}

fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

impl AbathurConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = r#"
[index]
doc_paths = ["docs/abathur"]
exclude = ["drafts/**"]

[sources]
roots = ["src/"]
exclude = ["**/target/**"]

[generate]
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
"#;
        let config: AbathurConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.index.doc_paths, vec![PathBuf::from("docs/abathur")]);
        assert_eq!(config.sources.roots, vec![PathBuf::from("src/")]);
        assert_eq!(config.generate.model, "claude-sonnet-4-6");
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[index]
doc_paths = ["docs/abathur"]

[sources]
roots = ["src/"]
"#;
        let config: AbathurConfig = toml::from_str(toml).unwrap();
        assert!(config.index.exclude.is_empty());
        assert_eq!(config.generate.model, "claude-sonnet-4-6");
        assert_eq!(config.generate.api_key_env, "ANTHROPIC_API_KEY");
    }
}
