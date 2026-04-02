use anyhow::Result;
use serde_json::Value;

/// Parsed summary from a conversation artifact.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub run_id: String,
    pub cost_usd: f64,
    pub num_turns: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

pub fn parse_conversation(run_id: &str, data: &[u8]) -> Result<ConversationSummary> {
    let v: Value = serde_json::from_slice(data)?;

    let cost_usd = match v.get("total_cost_usd").and_then(|v| v.as_f64()) {
        Some(c) => c,
        None => {
            tracing::warn!(run_id, "conversation missing total_cost_usd");
            0.0
        }
    };
    let num_turns = v.get("num_turns").and_then(|v| v.as_u64()).unwrap_or(0);
    let duration_ms = v.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let success = v.get("subtype").and_then(|v| v.as_str()) == Some("success");

    // Sum tokens across all models in modelUsage
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;

    if let Some(usage) = v.get("modelUsage").and_then(|v| v.as_object()) {
        for (_model, stats) in usage {
            input_tokens += stats
                .get("inputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            output_tokens += stats
                .get("outputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            cache_read_tokens += stats
                .get("cacheReadInputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            cache_creation_tokens += stats
                .get("cacheCreationInputTokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
    }

    Ok(ConversationSummary {
        run_id: run_id.to_string(),
        cost_usd,
        num_turns,
        duration_ms,
        success,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    })
}

/// Parsed detail from a session JSONL artifact.
#[derive(Debug, Clone)]
pub struct SessionDetail {
    pub run_id: String,
    pub tool_calls: Vec<ToolCall>,
    pub message_count: usize,
    pub compression_events: usize,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub first_arg: String,
    pub is_error: bool,
}

pub fn parse_session(run_id: &str, data: &[u8]) -> Result<SessionDetail> {
    let text = std::str::from_utf8(data)?;
    let mut tool_calls = Vec::new();
    let mut message_count = 0usize;
    let mut compression_events = 0usize;
    let mut skipped_lines = 0usize;

    // Track tool_use IDs to match with tool_results
    let mut pending_tool_ids: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            skipped_lines += 1;
            continue;
        };

        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                message_count += 1;
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            let name = block
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let first_arg = extract_first_arg(block.get("input"));
                            let tool_id = block
                                .get("id")
                                .and_then(|id| id.as_str())
                                .unwrap_or("")
                                .to_string();
                            let idx = tool_calls.len();
                            tool_calls.push(ToolCall {
                                name,
                                first_arg,
                                is_error: false,
                            });
                            if !tool_id.is_empty() {
                                pending_tool_ids.insert(tool_id, idx);
                            }
                        }
                    }
                }
            }
            "user" => {
                message_count += 1;
                // Check for tool_results with errors
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            let is_error = block
                                .get("is_error")
                                .and_then(|e| e.as_bool())
                                .unwrap_or(false);
                            if is_error {
                                let tool_id = block
                                    .get("tool_use_id")
                                    .and_then(|id| id.as_str())
                                    .unwrap_or("");
                                if let Some(&idx) = pending_tool_ids.get(tool_id) {
                                    tool_calls[idx].is_error = true;
                                }
                            }
                        }
                    }
                }
            }
            "system" => {
                // Detect context compression — match Claude Code's specific compression message
                if let Some(msg) = v.pointer("/message/content").and_then(|c| c.as_str())
                    && (msg.contains("summary of the conversation")
                        || msg.contains("context window is approaching"))
                {
                    compression_events += 1;
                }
            }
            _ => {}
        }
    }

    if skipped_lines > 0 {
        tracing::warn!(
            run_id,
            skipped_lines,
            total_lines = skipped_lines + message_count + compression_events,
            "skipped unparseable lines in session JSONL"
        );
    }

    Ok(SessionDetail {
        run_id: run_id.to_string(),
        tool_calls,
        message_count,
        compression_events,
    })
}

fn extract_first_arg(input: Option<&Value>) -> String {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return String::new();
    };
    // Prefer known identifying keys for stable retry detection
    const PRIORITY_KEYS: &[&str] = &["file_path", "command", "pattern", "path", "url"];
    for key in PRIORITY_KEYS {
        if let Some(s) = obj.get(*key).and_then(|v| v.as_str()) {
            return s.chars().take(100).collect();
        }
    }
    // Fall back to first string value
    for (_key, val) in obj {
        if let Some(s) = val.as_str() {
            return s.chars().take(100).collect();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_conversation() {
        let data = serde_json::json!({
            "total_cost_usd": 1.5,
            "num_turns": 30,
            "duration_ms": 120000,
            "subtype": "success",
            "session_id": "abc-123",
            "modelUsage": {
                "claude-sonnet-4-6": {
                    "inputTokens": 100,
                    "outputTokens": 500,
                    "cacheReadInputTokens": 50000,
                    "cacheCreationInputTokens": 10000
                }
            }
        });
        let bytes = serde_json::to_vec(&data).unwrap();
        let summary = parse_conversation("run-1", &bytes).unwrap();

        assert_eq!(summary.run_id, "run-1");
        assert!((summary.cost_usd - 1.5).abs() < f64::EPSILON);
        assert_eq!(summary.num_turns, 30);
        assert_eq!(summary.duration_ms, 120000);
        assert!(summary.success);
        assert_eq!(summary.input_tokens, 100);
        assert_eq!(summary.output_tokens, 500);
        assert_eq!(summary.cache_read_tokens, 50000);
        assert_eq!(summary.cache_creation_tokens, 10000);
    }

    #[test]
    fn test_parse_conversation_failed() {
        let data = serde_json::json!({
            "total_cost_usd": 0.5,
            "num_turns": 5,
            "duration_ms": 30000,
            "subtype": "error",
        });
        let bytes = serde_json::to_vec(&data).unwrap();
        let summary = parse_conversation("run-2", &bytes).unwrap();
        assert!(!summary.success);
    }

    #[test]
    fn test_parse_session_tool_calls() {
        let lines = [
            serde_json::json!({"type": "user", "message": {"role": "user", "content": "fix bug"}}),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/src/main.rs"}},
            ]}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "file contents..."},
            ]}}),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t2", "name": "Edit", "input": {"file_path": "/src/main.rs", "old_string": "x", "new_string": "y"}},
            ]}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t2", "is_error": true, "content": "String not found"},
            ]}}),
        ];
        let jsonl = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let detail = parse_session("run-3", jsonl.as_bytes()).unwrap();

        assert_eq!(detail.run_id, "run-3");
        assert_eq!(detail.tool_calls.len(), 2);
        assert_eq!(detail.tool_calls[0].name, "Read");
        assert_eq!(detail.tool_calls[0].first_arg, "/src/main.rs");
        assert!(!detail.tool_calls[0].is_error);
        assert_eq!(detail.tool_calls[1].name, "Edit");
        assert!(detail.tool_calls[1].is_error);
        assert_eq!(detail.message_count, 5);
        assert_eq!(detail.compression_events, 0);
    }

    #[test]
    fn test_parse_session_compression_detection() {
        let lines = [
            serde_json::json!({"type": "system", "message": {"content": "This is a summary of the conversation so far"}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": "continue"}}),
        ];
        let jsonl = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        let detail = parse_session("run-4", jsonl.as_bytes()).unwrap();

        assert_eq!(detail.compression_events, 1);
    }
}
