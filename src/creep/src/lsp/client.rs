use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, oneshot};

use super::diagnostics::{DiagnosticSeverity, DiagnosticsCache, LspDiagnostic};
use super::jsonrpc::{
    JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, decode_header, encode_message,
};
use super::manager::LspServerConfig;

/// Default timeout for LSP requests in seconds.
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub file: PathBuf,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

pub struct LspClient {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, String>>>>>,
    next_id: AtomicI64,
    pub diagnostics: DiagnosticsCache,
    request_timeout: std::time::Duration,
    _reader_task: tokio::task::JoinHandle<()>,
    _stderr_task: tokio::task::JoinHandle<()>,
    child: Mutex<Child>,
}

impl LspClient {
    /// Spawn an LSP server process and connect via stdio.
    pub async fn connect(
        config: &LspServerConfig,
        workspace: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let mut child = tokio::process::Command::new(&config.command)
            .args(&config.args)
            .current_dir(workspace)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin must be piped");
        let stdout = child.stdout.take().expect("stdout must be piped");
        let stderr = child.stderr.take().expect("stderr must be piped");

        let diagnostics = DiagnosticsCache::new();
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let reader_task = tokio::spawn(Self::reader_loop(
            stdout,
            pending.clone(),
            diagnostics.clone(),
        ));

        let server_name = config.name.clone();
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        tracing::warn!(lsp_server = %server_name, "{}", line.trim_end());
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicI64::new(1),
            diagnostics,
            request_timeout: std::time::Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            _reader_task: reader_task,
            _stderr_task: stderr_task,
            child: Mutex::new(child),
        })
    }

    /// Send a request and wait for the response.
    pub async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let encoded = encode_message(&req);

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(&encoded).await?;
            stdin.flush().await?;
        }

        let result = tokio::time::timeout(self.request_timeout, rx)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "LSP request timed out after {:?} for {method}",
                    self.request_timeout
                )
            })?
            .map_err(|_| anyhow::anyhow!("LSP response channel closed for {method}"))?;
        result.map_err(|e| anyhow::anyhow!("LSP error for {method}: {e}"))
    }

    /// Send a notification (no response expected).
    pub async fn notify(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> anyhow::Result<()> {
        let notif = JsonRpcNotification::new(method, params);
        let encoded = encode_message(&notif);

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&encoded).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Perform the LSP initialize handshake.
    pub async fn initialize(&self, workspace_root: &str) -> anyhow::Result<()> {
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": format!("file://{workspace_root}"),
            "capabilities": {
                "textDocument": {
                    "synchronization": { "dynamicRegistration": false },
                    "definition": { "dynamicRegistration": false },
                    "references": { "dynamicRegistration": false },
                    "publishDiagnostics": {}
                }
            }
        });
        self.request("initialize", Some(params)).await?;
        self.notify("initialized", Some(serde_json::json!({})))
            .await?;
        Ok(())
    }

    /// Send textDocument/didOpen notification.
    pub async fn open_document(
        &self,
        path: &str,
        content: &str,
        language_id: &str,
    ) -> anyhow::Result<()> {
        self.notify(
            "textDocument/didOpen",
            Some(serde_json::json!({
                "textDocument": {
                    "uri": format!("file://{path}"),
                    "languageId": language_id,
                    "version": 1,
                    "text": content
                }
            })),
        )
        .await
    }

    /// Send textDocument/didChange notification with full content.
    pub async fn change_document(
        &self,
        path: &str,
        content: &str,
        version: i32,
    ) -> anyhow::Result<()> {
        self.notify(
            "textDocument/didChange",
            Some(serde_json::json!({
                "textDocument": { "uri": format!("file://{path}"), "version": version },
                "contentChanges": [{ "text": content }]
            })),
        )
        .await
    }

    /// Send textDocument/didClose notification.
    pub async fn close_document(&self, path: &str) -> anyhow::Result<()> {
        self.notify(
            "textDocument/didClose",
            Some(serde_json::json!({
                "textDocument": { "uri": format!("file://{path}") }
            })),
        )
        .await
    }

    /// Request textDocument/definition.
    pub async fn goto_definition(
        &self,
        file: &str,
        line: u32,
        column: u32,
    ) -> anyhow::Result<Vec<SymbolLocation>> {
        let result = self
            .request(
                "textDocument/definition",
                Some(serde_json::json!({
                    "textDocument": { "uri": format!("file://{file}") },
                    "position": { "line": line, "character": column }
                })),
            )
            .await?;
        Ok(parse_locations(result))
    }

    /// Request textDocument/references.
    pub async fn find_references(
        &self,
        file: &str,
        line: u32,
        column: u32,
        include_declaration: bool,
    ) -> anyhow::Result<Vec<SymbolLocation>> {
        let result = self
            .request(
                "textDocument/references",
                Some(serde_json::json!({
                    "textDocument": { "uri": format!("file://{file}") },
                    "position": { "line": line, "character": column },
                    "context": { "includeDeclaration": include_declaration }
                })),
            )
            .await?;
        Ok(parse_locations(result))
    }

    /// Send shutdown request and exit notification, then wait briefly for
    /// the server to exit gracefully before falling back to kill.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        let _ = self.request("shutdown", None).await;
        let _ = self.notify("exit", None).await;
        let mut child = self.child.lock().await;
        match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
            Ok(Ok(_)) => {}
            _ => {
                tracing::warn!("LSP server did not exit after 5s, killing");
                let _ = child.kill().await;
            }
        }
        Ok(())
    }

    /// Background reader task: reads JSON-RPC frames from stdout, routes responses
    /// to pending oneshots, and handles diagnostics notifications.
    async fn reader_loop(
        stdout: ChildStdout,
        pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, String>>>>>,
        diagnostics: DiagnosticsCache,
    ) {
        let mut reader = BufReader::new(stdout);
        let mut header_buf = String::new();

        loop {
            header_buf.clear();

            // Read headers until blank line.
            let mut content_length: Option<usize> = None;
            loop {
                let bytes_read = match reader.read_line(&mut header_buf).await {
                    Ok(0) => return, // EOF
                    Ok(n) => n,
                    Err(_) => return,
                };

                let line = &header_buf[header_buf.len() - bytes_read..];
                let trimmed = line.trim();

                if trimmed.is_empty() {
                    break;
                }

                if let Some(len) = decode_header(trimmed) {
                    content_length = Some(len);
                }
            }

            let content_length = match content_length {
                Some(len) => len,
                None => continue,
            };

            // Read body.
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }

            let msg: JsonRpcMessage = match serde_json::from_slice(&body) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("failed to parse LSP message: {e}");
                    continue;
                }
            };

            if let Some(id) = msg.id {
                // Response to a request.
                let mut pending = pending.lock().await;
                if let Some(tx) = pending.remove(&id) {
                    if let Some(err) = msg.error {
                        let _ = tx.send(Err(format!("[{}] {}", err.code, err.message)));
                    } else {
                        let _ = tx.send(Ok(msg.result.unwrap_or(serde_json::Value::Null)));
                    }
                }
            } else if let Some(method) = &msg.method {
                // Notification from server.
                if method == "textDocument/publishDiagnostics" {
                    if let Some(params) = msg.params {
                        Self::handle_diagnostics(&diagnostics, params);
                    }
                }
            }
        }
    }

    fn handle_diagnostics(cache: &DiagnosticsCache, params: serde_json::Value) {
        let uri = match params["uri"].as_str() {
            Some(u) => u,
            None => return,
        };
        let file = PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri));

        let diags = params["diagnostics"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|d| {
                        let range = &d["range"]["start"];
                        LspDiagnostic {
                            file: file.clone(),
                            line: range["line"].as_u64().unwrap_or(0) as u32,
                            column: range["character"].as_u64().unwrap_or(0) as u32,
                            severity: DiagnosticSeverity::from_lsp(
                                d["severity"].as_u64().unwrap_or(4),
                            ),
                            message: d["message"].as_str().unwrap_or("").to_string(),
                            source: d["source"].as_str().map(|s| s.to_string()),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        cache.update(file, diags);
    }
}

pub fn parse_locations(value: serde_json::Value) -> Vec<SymbolLocation> {
    if value.is_null() {
        return vec![];
    }

    // Single Location: { uri, range }
    if value.is_object() && value.get("uri").is_some() {
        return match parse_single_location(&value) {
            Some(loc) => vec![loc],
            None => vec![],
        };
    }

    // Array of Location or LocationLink
    if let Some(arr) = value.as_array() {
        let mut locations = Vec::new();
        for item in arr {
            if item.get("targetUri").is_some() {
                // LocationLink
                let uri = item["targetUri"].as_str().unwrap_or_default();
                let range = &item["targetRange"];
                locations.push(SymbolLocation {
                    file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
                    start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
                    start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
                    end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
                    end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
                });
            } else if let Some(loc) = parse_single_location(item) {
                locations.push(loc);
            }
        }
        return locations;
    }

    vec![]
}

fn parse_single_location(value: &serde_json::Value) -> Option<SymbolLocation> {
    let uri = value["uri"].as_str()?;
    let range = &value["range"];
    Some(SymbolLocation {
        file: PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri)),
        start_line: range["start"]["line"].as_u64().unwrap_or(0) as u32,
        start_column: range["start"]["character"].as_u64().unwrap_or(0) as u32,
        end_line: range["end"]["line"].as_u64().unwrap_or(0) as u32,
        end_column: range["end"]["character"].as_u64().unwrap_or(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_null_location() {
        let result = parse_locations(serde_json::Value::Null);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_single_location() {
        let json = serde_json::json!({
            "uri": "file:///src/main.rs",
            "range": {
                "start": { "line": 10, "character": 5 },
                "end": { "line": 10, "character": 15 }
            }
        });
        let result = parse_locations(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file, PathBuf::from("/src/main.rs"));
        assert_eq!(result[0].start_line, 10);
        assert_eq!(result[0].start_column, 5);
        assert_eq!(result[0].end_line, 10);
        assert_eq!(result[0].end_column, 15);
    }

    #[test]
    fn test_parse_location_array() {
        let json = serde_json::json!([
            {
                "uri": "file:///a.rs",
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end": { "line": 1, "character": 10 }
                }
            },
            {
                "uri": "file:///b.rs",
                "range": {
                    "start": { "line": 5, "character": 2 },
                    "end": { "line": 5, "character": 12 }
                }
            }
        ]);
        let result = parse_locations(json);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].file, PathBuf::from("/a.rs"));
        assert_eq!(result[1].file, PathBuf::from("/b.rs"));
    }

    #[test]
    fn test_parse_location_link_array() {
        let json = serde_json::json!([
            {
                "targetUri": "file:///target.rs",
                "targetRange": {
                    "start": { "line": 3, "character": 0 },
                    "end": { "line": 8, "character": 1 }
                },
                "targetSelectionRange": {
                    "start": { "line": 3, "character": 4 },
                    "end": { "line": 3, "character": 10 }
                }
            }
        ]);
        let result = parse_locations(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file, PathBuf::from("/target.rs"));
        assert_eq!(result[0].start_line, 3);
        assert_eq!(result[0].end_line, 8);
    }

    #[test]
    fn test_parse_empty_array() {
        let json = serde_json::json!([]);
        let result = parse_locations(json);
        assert!(result.is_empty());
    }

    #[test]
    fn test_handle_diagnostics() {
        let cache = DiagnosticsCache::new();
        let params = serde_json::json!({
            "uri": "file:///src/main.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 10, "character": 5 },
                        "end": { "line": 10, "character": 15 }
                    },
                    "severity": 1,
                    "message": "undefined variable",
                    "source": "rustc"
                },
                {
                    "range": {
                        "start": { "line": 20, "character": 0 },
                        "end": { "line": 20, "character": 10 }
                    },
                    "severity": 2,
                    "message": "unused import",
                    "source": "rustc"
                }
            ]
        });

        LspClient::handle_diagnostics(&cache, params);

        let file = PathBuf::from("/src/main.rs");
        let diags = cache.get_file(&file);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[0].message, "undefined variable");
        assert_eq!(diags[1].severity, DiagnosticSeverity::Warning);
        assert_eq!(diags[1].message, "unused import");
    }

    #[test]
    fn test_handle_empty_diagnostics_clears() {
        let cache = DiagnosticsCache::new();
        let file = PathBuf::from("/src/main.rs");

        // First set some diagnostics
        let params = serde_json::json!({
            "uri": "file:///src/main.rs",
            "diagnostics": [{
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "severity": 1,
                "message": "error"
            }]
        });
        LspClient::handle_diagnostics(&cache, params);
        assert_eq!(cache.get_file(&file).len(), 1);

        // Now clear them
        let params = serde_json::json!({
            "uri": "file:///src/main.rs",
            "diagnostics": []
        });
        LspClient::handle_diagnostics(&cache, params);
        assert!(cache.get_file(&file).is_empty());
    }
}
