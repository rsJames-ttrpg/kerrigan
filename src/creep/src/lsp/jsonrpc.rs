use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: i64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(id: i64, method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<i64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// A raw incoming message from the LSP server — could be a response or notification.
#[derive(Debug, Deserialize)]
pub struct JsonRpcMessage {
    /// Present for responses, absent for notifications.
    pub id: Option<i64>,
    /// Present for notifications, absent for responses.
    pub method: Option<String>,
    /// Result payload (responses only).
    pub result: Option<serde_json::Value>,
    /// Error payload (responses only).
    pub error: Option<JsonRpcError>,
    /// Params payload (notifications only).
    pub params: Option<serde_json::Value>,
}

/// Encode a JSON-RPC message with Content-Length framing.
pub fn encode_message(msg: &impl Serialize) -> Vec<u8> {
    let body = serde_json::to_string(msg).expect("JSON-RPC serialization should not fail");
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

/// Parse Content-Length value from a header line.
pub fn decode_header(header: &str) -> Option<usize> {
    header
        .strip_prefix("Content-Length: ")
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_request() {
        let req = JsonRpcRequest::new(1, "initialize", None);
        let encoded = encode_message(&req);
        let s = String::from_utf8(encoded).unwrap();
        assert!(s.starts_with("Content-Length: "));
        assert!(s.contains("\r\n\r\n"));
        let body = s.split("\r\n\r\n").nth(1).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["method"], "initialize");
        assert!(parsed.get("params").is_none());
    }

    #[test]
    fn test_encode_request_with_params() {
        let req = JsonRpcRequest::new(
            2,
            "textDocument/definition",
            Some(serde_json::json!({"uri": "file:///foo.rs"})),
        );
        let encoded = encode_message(&req);
        let s = String::from_utf8(encoded).unwrap();
        let body = s.split("\r\n\r\n").nth(1).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["params"]["uri"], "file:///foo.rs");
    }

    #[test]
    fn test_encode_notification() {
        let notif = JsonRpcNotification::new("initialized", Some(serde_json::json!({})));
        let encoded = encode_message(&notif);
        let s = String::from_utf8(encoded).unwrap();
        let body = s.split("\r\n\r\n").nth(1).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["method"], "initialized");
        assert!(parsed.get("id").is_none());
    }

    #[test]
    fn test_decode_header() {
        assert_eq!(decode_header("Content-Length: 42"), Some(42));
        assert_eq!(decode_header("Content-Length: 0"), Some(0));
        assert_eq!(decode_header("Content-Length: 42\r"), Some(42));
        assert_eq!(decode_header("Content-Type: application/json"), None);
        assert_eq!(decode_header(""), None);
    }

    #[test]
    fn test_roundtrip_encode_decode() {
        let req = JsonRpcRequest::new(1, "test/method", Some(serde_json::json!({"key": "value"})));
        let encoded = encode_message(&req);
        let s = String::from_utf8(encoded).unwrap();

        // Parse header
        let header_line = s.lines().next().unwrap();
        let content_len = decode_header(header_line).unwrap();

        // Parse body
        let body = s.split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(body.len(), content_len);

        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["method"], "test/method");
    }

    #[test]
    fn test_deserialize_response() {
        let json = r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_deserialize_error_response() {
        let json =
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, Some(2));
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Invalid Request");
    }

    #[test]
    fn test_deserialize_notification_message() {
        let json = r#"{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///foo.rs","diagnostics":[]}}"#;
        let msg: JsonRpcMessage = serde_json::from_str(json).unwrap();
        assert!(msg.id.is_none());
        assert_eq!(
            msg.method.as_deref(),
            Some("textDocument/publishDiagnostics")
        );
        assert!(msg.params.is_some());
    }
}
