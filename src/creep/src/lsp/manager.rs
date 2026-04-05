use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::client::LspClient;
use super::diagnostics::{DiagnosticSeverity, LspDiagnostic};

/// Configuration for a single LSP server.
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub extensions: Vec<String>,
    pub language_id: String,
}

/// Manages LSP server processes per workspace, providing lazy startup
/// and warm-across-runs semantics.
pub struct LspManager {
    configs: Vec<LspServerConfig>,
    clients: HashMap<String, LspClient>,
    extension_map: HashMap<String, String>,
}

impl LspManager {
    pub fn new(configs: Vec<LspServerConfig>) -> Self {
        let mut extension_map = HashMap::new();
        for config in &configs {
            for ext in &config.extensions {
                extension_map.insert(ext.clone(), config.name.clone());
            }
        }
        Self {
            configs,
            clients: HashMap::new(),
            extension_map,
        }
    }

    /// Get the language_id for a file extension, if any LSP server handles it.
    pub fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        let server_name = self.extension_map.get(ext)?;
        self.configs
            .iter()
            .find(|c| &c.name == server_name)
            .map(|c| c.language_id.as_str())
    }

    /// Start language server for a workspace if not already running.
    /// Returns None if no server handles the given file extension.
    pub async fn ensure_server(
        &mut self,
        workspace: &Path,
        file_ext: &str,
    ) -> anyhow::Result<Option<&LspClient>> {
        let server_name = match self.extension_map.get(file_ext) {
            Some(name) => name.clone(),
            None => return Ok(None),
        };

        let key = format!("{server_name}:{}", workspace.display());
        if !self.clients.contains_key(&key) {
            let config = self
                .configs
                .iter()
                .find(|c| c.name == server_name)
                .expect("extension_map references a config that must exist");
            let client = LspClient::connect(config, workspace).await?;
            client.initialize(&workspace.to_string_lossy()).await?;
            self.clients.insert(key.clone(), client);
        }

        Ok(self.clients.get(&key))
    }

    /// Get a client by server name and workspace, without starting it.
    pub fn get_client(&self, workspace: &Path, file_ext: &str) -> Option<&LspClient> {
        let server_name = self.extension_map.get(file_ext)?;
        let key = format!("{server_name}:{}", workspace.display());
        self.clients.get(&key)
    }

    /// Find the workspace that contains the given file path, among active clients.
    pub fn find_workspace_for_file(&self, file: &Path) -> Option<PathBuf> {
        for key in self.clients.keys() {
            if let Some(ws) = key.splitn(2, ':').nth(1) {
                let ws_path = PathBuf::from(ws);
                if file.starts_with(&ws_path) {
                    return Some(ws_path);
                }
            }
        }
        None
    }

    /// Shutdown all language servers.
    pub async fn shutdown_all(&mut self) {
        for (_, client) in self.clients.drain() {
            let _ = client.shutdown().await;
        }
    }

    /// Get diagnostics across all active servers for a workspace.
    pub fn diagnostics(
        &self,
        workspace: &Path,
        min_severity: DiagnosticSeverity,
    ) -> Vec<LspDiagnostic> {
        self.clients
            .iter()
            .filter(|(k, _)| k.ends_with(&format!(":{}", workspace.display())))
            .flat_map(|(_, client)| client.diagnostics.get_all(min_severity))
            .collect()
    }

    /// Get diagnostics for a specific file across all active servers.
    pub fn file_diagnostics(&self, file: &PathBuf) -> Vec<LspDiagnostic> {
        self.clients
            .values()
            .flat_map(|client| client.diagnostics.get_file(file))
            .collect()
    }

    /// Check if any LSP configs are registered.
    pub fn has_configs(&self) -> bool {
        !self.configs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_configs() -> Vec<LspServerConfig> {
        vec![
            LspServerConfig {
                name: "rust-analyzer".to_string(),
                command: "rust-analyzer".to_string(),
                args: vec![],
                extensions: vec![".rs".to_string()],
                language_id: "rust".to_string(),
            },
            LspServerConfig {
                name: "typescript-language-server".to_string(),
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                extensions: vec![".ts".to_string(), ".tsx".to_string()],
                language_id: "typescript".to_string(),
            },
        ]
    }

    #[test]
    fn test_extension_map() {
        let mgr = LspManager::new(sample_configs());
        assert_eq!(
            mgr.extension_map.get(".rs"),
            Some(&"rust-analyzer".to_string())
        );
        assert_eq!(
            mgr.extension_map.get(".ts"),
            Some(&"typescript-language-server".to_string())
        );
        assert_eq!(
            mgr.extension_map.get(".tsx"),
            Some(&"typescript-language-server".to_string())
        );
        assert!(mgr.extension_map.get(".py").is_none());
    }

    #[test]
    fn test_language_id_for_extension() {
        let mgr = LspManager::new(sample_configs());
        assert_eq!(mgr.language_id_for_extension(".rs"), Some("rust"));
        assert_eq!(mgr.language_id_for_extension(".ts"), Some("typescript"));
        assert_eq!(mgr.language_id_for_extension(".py"), None);
    }

    #[test]
    fn test_get_client_none_when_not_started() {
        let mgr = LspManager::new(sample_configs());
        assert!(mgr.get_client(Path::new("/workspace"), ".rs").is_none());
    }

    #[test]
    fn test_has_configs() {
        let mgr = LspManager::new(sample_configs());
        assert!(mgr.has_configs());

        let empty = LspManager::new(vec![]);
        assert!(!empty.has_configs());
    }

    #[test]
    fn test_diagnostics_empty_when_no_clients() {
        let mgr = LspManager::new(sample_configs());
        let diags = mgr.diagnostics(Path::new("/workspace"), DiagnosticSeverity::Hint);
        assert!(diags.is_empty());
    }

    #[test]
    fn test_file_diagnostics_empty_when_no_clients() {
        let mgr = LspManager::new(sample_configs());
        let diags = mgr.file_diagnostics(&PathBuf::from("/src/main.rs"));
        assert!(diags.is_empty());
    }

    #[test]
    fn test_find_workspace_for_file_none_when_no_clients() {
        let mgr = LspManager::new(sample_configs());
        assert!(
            mgr.find_workspace_for_file(Path::new("/workspace/src/main.rs"))
                .is_none()
        );
    }
}
