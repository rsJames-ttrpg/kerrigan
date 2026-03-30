use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_mcp_transport")]
    pub mcp_transport: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            http_port: default_http_port(),
            mcp_transport: default_mcp_transport(),
        }
    }
}

fn default_http_port() -> u16 {
    3100
}
fn default_mcp_transport() -> String {
    "stdio".to_string()
}

#[derive(Debug, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,
    #[serde(default = "default_artifact_path")]
    pub artifact_path: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_path: default_db_path(),
            artifact_path: default_artifact_path(),
        }
    }
}

fn default_db_path() -> PathBuf {
    PathBuf::from("data/overseer.db")
}
fn default_artifact_path() -> PathBuf {
    PathBuf::from("data/artifacts")
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
        }
    }
}

fn default_provider() -> String {
    "stub".to_string()
}

#[derive(Debug, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(toml::from_str("")?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_defaults_when_file_missing() {
        let config = Config::load(std::path::Path::new("nonexistent-config.toml"))
            .expect("should fall back to defaults");
        assert_eq!(config.server.http_port, 3100);
        assert_eq!(config.server.mcp_transport, "stdio");
        assert_eq!(
            config.storage.database_path,
            PathBuf::from("data/overseer.db")
        );
        assert_eq!(
            config.storage.artifact_path,
            PathBuf::from("data/artifacts")
        );
        assert_eq!(config.embedding.provider, "stub");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "[server]\nhttp_port = 9000\n").unwrap();
        let config = Config::load(f.path()).expect("should parse");
        assert_eq!(config.server.http_port, 9000);
        assert_eq!(config.server.mcp_transport, "stdio"); // default
        assert_eq!(config.embedding.provider, "stub"); // default
    }

    #[test]
    fn test_full_toml() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[server]
http_port = 8080
mcp_transport = "http"

[storage]
database_path = "/tmp/test.db"
artifact_path = "/tmp/arts"

[embedding]
provider = "local"

[logging]
level = "debug"
"#
        )
        .unwrap();
        let config = Config::load(f.path()).expect("should parse");
        assert_eq!(config.server.http_port, 8080);
        assert_eq!(config.server.mcp_transport, "http");
        assert_eq!(config.storage.database_path, PathBuf::from("/tmp/test.db"));
        assert_eq!(config.embedding.provider, "local");
        assert_eq!(config.logging.level, "debug");
    }
}
