use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::registry::Tool;
use super::types::*;

// --- JSON-RPC types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// --- MCP tool info from tools/list ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

// --- MCP server config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpTransport {
    #[serde(rename = "http")]
    Http { url: String },
    #[serde(rename = "stdio")]
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
}

// --- MCP Client ---

enum ClientTransport {
    Http {
        client: reqwest::Client,
        url: String,
    },
    Stdio {
        child: Child,
        stdin: tokio::process::ChildStdin,
        reader: BufReader<tokio::process::ChildStdout>,
    },
}

pub struct McpClient {
    name: String,
    transport: Mutex<ClientTransport>,
    next_id: Mutex<u64>,
    tools: Vec<McpToolInfo>,
}

impl McpClient {
    pub async fn connect(config: &McpServerConfig) -> Result<Self, String> {
        let transport = match &config.transport {
            McpTransport::Http { url } => ClientTransport::Http {
                client: reqwest::Client::new(),
                url: url.clone(),
            },
            McpTransport::Stdio { command, args, env } => {
                let mut cmd = Command::new(command);
                cmd.args(args)
                    .envs(env.iter())
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null());

                let mut child = cmd
                    .spawn()
                    .map_err(|e| format!("failed to spawn MCP server: {e}"))?;
                let stdin = child.stdin.take().ok_or("failed to get stdin")?;
                let stdout = child.stdout.take().ok_or("failed to get stdout")?;
                let reader = BufReader::new(stdout);

                ClientTransport::Stdio {
                    child,
                    stdin,
                    reader,
                }
            }
        };

        let mut client = Self {
            name: config.name.clone(),
            transport: Mutex::new(transport),
            next_id: Mutex::new(1),
            tools: Vec::new(),
        };

        // Initialize handshake
        client.initialize().await?;

        // Discover tools
        client.discover_tools().await?;

        Ok(client)
    }

    async fn next_request_id(&self) -> u64 {
        let mut id = self.next_id.lock().await;
        let current = *id;
        *id += 1;
        current
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, String> {
        let id = self.next_request_id().await;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(id),
            method: method.into(),
            params,
        };

        let mut transport = self.transport.lock().await;
        match &mut *transport {
            ClientTransport::Http { client, url } => {
                let resp = client
                    .post(url.as_str())
                    .json(&request)
                    .send()
                    .await
                    .map_err(|e| format!("HTTP request failed: {e}"))?;

                let body = resp
                    .json::<JsonRpcResponse>()
                    .await
                    .map_err(|e| format!("invalid JSON-RPC response: {e}"))?;

                Ok(body)
            }
            ClientTransport::Stdio { stdin, reader, .. } => {
                let body = serde_json::to_string(&request)
                    .map_err(|e| format!("failed to serialize request: {e}"))?;

                // Content-Length framing
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                stdin
                    .write_all(header.as_bytes())
                    .await
                    .map_err(|e| format!("failed to write header: {e}"))?;
                stdin
                    .write_all(body.as_bytes())
                    .await
                    .map_err(|e| format!("failed to write body: {e}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| format!("failed to flush: {e}"))?;

                // Read response with Content-Length framing
                let content_length = read_content_length(reader)
                    .await
                    .map_err(|e| format!("failed to read response header: {e}"))?;

                let mut buf = vec![0u8; content_length];
                reader
                    .read_exact(&mut buf)
                    .await
                    .map_err(|e| format!("failed to read response body: {e}"))?;

                let response: JsonRpcResponse = serde_json::from_slice(&buf)
                    .map_err(|e| format!("invalid JSON-RPC response: {e}"))?;

                Ok(response)
            }
        }
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: None,
            method: method.into(),
            params,
        };

        let mut transport = self.transport.lock().await;
        match &mut *transport {
            ClientTransport::Http { client, url } => {
                client
                    .post(url.as_str())
                    .json(&request)
                    .send()
                    .await
                    .map_err(|e| format!("HTTP notification failed: {e}"))?;
                Ok(())
            }
            ClientTransport::Stdio { stdin, .. } => {
                let body = serde_json::to_string(&request)
                    .map_err(|e| format!("failed to serialize: {e}"))?;
                let header = format!("Content-Length: {}\r\n\r\n", body.len());
                stdin
                    .write_all(header.as_bytes())
                    .await
                    .map_err(|e| format!("failed to write: {e}"))?;
                stdin
                    .write_all(body.as_bytes())
                    .await
                    .map_err(|e| format!("failed to write: {e}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| format!("failed to flush: {e}"))?;
                Ok(())
            }
        }
    }

    async fn initialize(&self) -> Result<(), String> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "kerrigan-runtime",
                "version": "0.1.0"
            }
        });

        let resp = self.send_request("initialize", Some(params)).await?;
        if let Some(err) = resp.error {
            return Err(format!("initialize failed: {}", err.message));
        }

        // Send initialized notification
        self.send_notification("notifications/initialized", None)
            .await?;

        Ok(())
    }

    async fn discover_tools(&mut self) -> Result<(), String> {
        let resp = self.send_request("tools/list", None).await?;

        if let Some(err) = resp.error {
            return Err(format!("tools/list failed: {}", err.message));
        }

        if let Some(result) = resp.result {
            if let Some(tools) = result.get("tools") {
                self.tools = serde_json::from_value(tools.clone())
                    .map_err(|e| format!("failed to parse tools: {e}"))?;
            }
        }

        Ok(())
    }

    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });

        let resp = self.send_request("tools/call", Some(params)).await?;

        if let Some(err) = resp.error {
            return Err(format!("tools/call failed: {}", err.message));
        }

        resp.result.ok_or_else(|| "no result in response".into())
    }

    pub fn tool_infos(&self) -> &[McpToolInfo] {
        &self.tools
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn shutdown(&self) {
        // Send MCP shutdown request before killing the process
        let _ = self.send_request("shutdown", None).await;
        let _ = self.send_notification("notifications/exit", None).await;

        let mut transport = self.transport.lock().await;
        if let ClientTransport::Stdio { child, .. } = &mut *transport {
            let _ = child.kill().await;
        }
    }
}

async fn read_content_length(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<usize, String> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(val) = trimmed.strip_prefix("Content-Length:") {
            let len: usize = val
                .trim()
                .parse()
                .map_err(|e| format!("invalid Content-Length: {e}"))?;
            // Read the empty line after headers
            let mut empty = String::new();
            reader
                .read_line(&mut empty)
                .await
                .map_err(|e| format!("read error: {e}"))?;
            return Ok(len);
        }
    }
}

// --- MCP Tool Proxy ---

pub struct McpToolProxy {
    namespaced_name: String,
    tool_name: String,
    description: String,
    schema: serde_json::Value,
    client: Arc<McpClient>,
}

impl McpToolProxy {
    pub fn new(server_name: &str, info: &McpToolInfo, client: Arc<McpClient>) -> Self {
        Self {
            namespaced_name: Self::namespaced_name(server_name, &info.name),
            tool_name: info.name.clone(),
            description: info.description.clone(),
            schema: info.input_schema.clone(),
            client,
        }
    }

    pub fn namespaced_name(server_name: &str, tool_name: &str) -> String {
        format!("mcp__{server_name}__{tool_name}")
    }
}

#[async_trait]
impl Tool for McpToolProxy {
    fn name(&self) -> &str {
        &self.namespaced_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.schema.clone()
    }

    fn permission(&self) -> PermissionLevel {
        PermissionLevel::FullAccess
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        match self.client.call_tool(&self.tool_name, input).await {
            Ok(result) => {
                // MCP tool results have content array
                if let Some(content) = result.get("content") {
                    if let Some(arr) = content.as_array() {
                        let text: Vec<String> = arr
                            .iter()
                            .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                            .map(|s| s.to_string())
                            .collect();
                        let is_error = result
                            .get("isError")
                            .and_then(|e| e.as_bool())
                            .unwrap_or(false);
                        if is_error {
                            return ToolResult::error(text.join("\n"));
                        }
                        return ToolResult::success(text.join("\n"));
                    }
                }
                ToolResult::success(result.to_string())
            }
            Err(e) => ToolResult::error(e),
        }
    }
}

// --- MCP Manager ---

pub struct McpManager {
    clients: HashMap<String, Arc<McpClient>>,
}

impl McpManager {
    pub async fn connect_all(configs: &[McpServerConfig]) -> Result<Self, String> {
        let mut clients = HashMap::new();
        for config in configs {
            let client = McpClient::connect(config).await?;
            clients.insert(config.name.clone(), Arc::new(client));
        }
        Ok(Self { clients })
    }

    pub fn register_tools(&self, registry: &mut super::ToolRegistry) {
        for (server_name, client) in &self.clients {
            for info in client.tool_infos() {
                let proxy = McpToolProxy::new(server_name, info, Arc::clone(client));
                registry.register(Box::new(proxy));
            }
        }
    }

    pub async fn shutdown_all(&self) {
        for client in self.clients.values() {
            client.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(1),
            method: "tools/list".into(),
            params: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
        assert!(json.get("params").is_none());
    }

    #[test]
    fn test_jsonrpc_response_deserialization() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"tools": []}
        });
        let resp: JsonRpcResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_jsonrpc_error_response() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        });
        let resp: JsonRpcResponse = serde_json::from_value(json).unwrap();
        assert!(resp.error.is_some());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }

    #[test]
    fn test_tool_name_namespacing() {
        let name = McpToolProxy::namespaced_name("overseer", "store_memory");
        assert_eq!(name, "mcp__overseer__store_memory");
    }

    #[test]
    fn test_mcp_server_config_http() {
        let json = serde_json::json!({
            "name": "test",
            "transport": {
                "type": "http",
                "url": "http://localhost:3100/mcp"
            }
        });
        let config: McpServerConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.name, "test");
        assert!(matches!(config.transport, McpTransport::Http { .. }));
    }

    #[test]
    fn test_mcp_server_config_stdio() {
        let json = serde_json::json!({
            "name": "test",
            "transport": {
                "type": "stdio",
                "command": "/usr/bin/server",
                "args": ["--mode", "mcp"]
            }
        });
        let config: McpServerConfig = serde_json::from_value(json).unwrap();
        assert!(matches!(config.transport, McpTransport::Stdio { .. }));
    }

    #[test]
    fn test_mcp_tool_info_deserialization() {
        let json = serde_json::json!({
            "name": "store_memory",
            "description": "Store a memory",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {"type": "string"}
                }
            }
        });
        let info: McpToolInfo = serde_json::from_value(json).unwrap();
        assert_eq!(info.name, "store_memory");
        assert_eq!(info.description, "Store a memory");
    }

    #[test]
    fn test_mcp_tool_info_missing_fields() {
        let json = serde_json::json!({"name": "minimal"});
        let info: McpToolInfo = serde_json::from_value(json).unwrap();
        assert_eq!(info.name, "minimal");
        assert_eq!(info.description, "");
    }
}
