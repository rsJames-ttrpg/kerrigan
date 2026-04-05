use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemBlock {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
}

#[derive(Debug)]
pub enum StreamEvent {
    TextDelta(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Usage(TokenUsage),
    MessageStop,
    Error(super::error::ApiError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_block_text_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_content_block_tool_use_roundtrip() {
        let block = ContentBlock::ToolUse {
            id: "tool_1".into(),
            name: "read_file".into(),
            input: serde_json::json!({"path": "/tmp/test.rs"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "tool_1");
                assert_eq!(name, "read_file");
                assert_eq!(input["path"], "/tmp/test.rs");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_content_block_tool_result_roundtrip() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "tool_1".into(),
            content: "file contents".into(),
            is_error: false,
        };
        let json = serde_json::to_string(&block).unwrap();
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "tool_1");
                assert_eq!(content, "file contents");
                assert!(!is_error);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_api_request_serialization() {
        let request = ApiRequest {
            system: vec![SystemBlock {
                text: "You are helpful.".into(),
            }],
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            }],
            tools: vec![],
            max_tokens: 1024,
            temperature: Some(0.0),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["max_tokens"], 1024);
        assert_eq!(json["messages"][0]["role"], "user");
    }

    #[test]
    fn test_role_serialization() {
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
    }
}
