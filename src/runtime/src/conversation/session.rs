use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
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
        tool_name: String,
        output: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub blocks: Vec<ContentBlock>,
    pub token_estimate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub messages: Vec<Message>,
    pub total_tokens_estimate: u32,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            messages: Vec::new(),
            total_tokens_estimate: 0,
        }
    }

    pub fn push(&mut self, message: Message) {
        self.total_tokens_estimate += message.token_estimate;
        self.messages.push(message);
    }

    /// Estimate tokens for a string (~4 chars per token for English)
    pub fn estimate_tokens(text: &str) -> u32 {
        (text.len() as u32 / 4).max(1)
    }

    /// Recalculate total token estimate from all messages
    pub fn recalculate_tokens(&mut self) {
        self.total_tokens_estimate = self.messages.iter().map(|m| m.token_estimate).sum();
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = Session::new();
        assert!(!session.id.is_empty());
        assert!(session.messages.is_empty());
        assert_eq!(session.total_tokens_estimate, 0);
    }

    #[test]
    fn test_push_message() {
        let mut session = Session::new();
        session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            token_estimate: 2,
        });
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.total_tokens_estimate, 2);
    }

    #[test]
    fn test_push_multiple_messages() {
        let mut session = Session::new();
        session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            token_estimate: 2,
        });
        session.push(Message {
            role: Role::Assistant,
            blocks: vec![ContentBlock::Text {
                text: "hi there".into(),
            }],
            token_estimate: 3,
        });
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.total_tokens_estimate, 5);
    }

    #[test]
    fn test_token_estimation() {
        // ~4 chars per token
        assert_eq!(Session::estimate_tokens("hello world!"), 3); // 12 chars / 4 = 3
        assert_eq!(Session::estimate_tokens("hi"), 1); // 2 chars / 4 = 0, max(1) = 1
        assert_eq!(Session::estimate_tokens(""), 1); // empty = max(1) = 1
        assert_eq!(Session::estimate_tokens("abcdefgh"), 2); // 8 / 4 = 2
    }

    #[test]
    fn test_recalculate_tokens() {
        let mut session = Session::new();
        session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            token_estimate: 10,
        });
        session.push(Message {
            role: Role::Assistant,
            blocks: vec![ContentBlock::Text {
                text: "world".into(),
            }],
            token_estimate: 20,
        });
        assert_eq!(session.total_tokens_estimate, 30);

        // Manually remove first message
        session.messages.remove(0);
        session.recalculate_tokens();
        assert_eq!(session.total_tokens_estimate, 20);
    }

    #[test]
    fn test_json_roundtrip() {
        let mut session = Session::new();
        session.push(Message {
            role: Role::User,
            blocks: vec![ContentBlock::Text {
                text: "hello".into(),
            }],
            token_estimate: 2,
        });
        session.push(Message {
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    text: "I'll help".into(),
                },
                ContentBlock::ToolUse {
                    id: "tool_1".into(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": "/tmp/test"}),
                },
            ],
            token_estimate: 10,
        });
        session.push(Message {
            role: Role::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "tool_1".into(),
                tool_name: "read_file".into(),
                output: "file contents".into(),
                is_error: false,
            }],
            token_estimate: 4,
        });

        let json = serde_json::to_string(&session).unwrap();
        let decoded: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.id, session.id);
        assert_eq!(decoded.messages.len(), 3);
        assert_eq!(decoded.total_tokens_estimate, 16);

        // Verify role deserialization
        assert_eq!(decoded.messages[0].role, Role::User);
        assert_eq!(decoded.messages[1].role, Role::Assistant);
        assert_eq!(decoded.messages[2].role, Role::Tool);
    }

    #[test]
    fn test_role_serialization() {
        assert_eq!(serde_json::to_string(&Role::System).unwrap(), "\"system\"");
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
    }

    #[test]
    fn test_content_block_tool_result_roundtrip() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "t1".into(),
            tool_name: "bash".into(),
            output: "ok".into(),
            is_error: false,
        };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains("\"output\""));
        assert!(!json.contains("\"content\""));
        let decoded: ContentBlock = serde_json::from_str(&json).unwrap();
        match decoded {
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                output,
                is_error,
            } => {
                assert_eq!(tool_use_id, "t1");
                assert_eq!(tool_name, "bash");
                assert_eq!(output, "ok");
                assert!(!is_error);
            }
            _ => panic!("wrong variant"),
        }
    }
}
