use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

use crate::event::EventSink;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OutputFormat {
    Markdown,
    Json,
    Raw,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub format: OutputFormat,
    pub metadata: Option<serde_json::Value>,
}

impl ToolResult {
    pub fn success(output: String) -> Self {
        Self {
            output,
            is_error: false,
            format: OutputFormat::Markdown,
            metadata: None,
        }
    }

    pub fn error(output: String) -> Self {
        Self {
            output,
            is_error: true,
            format: OutputFormat::Markdown,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PermissionLevel {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

pub struct ToolContext {
    pub workspace: PathBuf,
    pub home: PathBuf,
    pub event_sink: Arc<dyn EventSink>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_ordering() {
        assert!(PermissionLevel::ReadOnly < PermissionLevel::WorkspaceWrite);
        assert!(PermissionLevel::WorkspaceWrite < PermissionLevel::FullAccess);
    }

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("ok".into());
        assert!(!result.is_error);
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("bad".into());
        assert!(result.is_error);
    }
}
