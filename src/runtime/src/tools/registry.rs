use std::collections::HashMap;

use async_trait::async_trait;

use super::types::*;
use crate::api::ToolDefinition;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn permission(&self) -> PermissionLevel;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        match self.get(name) {
            Some(tool) => tool.execute(input, ctx).await,
            None => ToolResult::error(format!("unknown tool: {name}")),
        }
    }

    /// Generate tool definitions for the API request, respecting allow/deny lists
    pub fn definitions(&self, allowed: &[String], denied: &[String]) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .filter(|t| allowed.is_empty() || allowed.contains(&t.name().to_string()))
            .filter(|t| !denied.contains(&t.name().to_string()))
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    /// List available tool names after filtering
    pub fn available_names(&self, allowed: &[String], denied: &[String]) -> Vec<String> {
        self.definitions(allowed, denied)
            .into_iter()
            .map(|d| d.name)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTool;

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "mock"
        }
        fn description(&self) -> &str {
            "a mock tool"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::ReadOnly
        }
        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success("mock result".into())
        }
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        assert!(registry.get("mock").is_some());
        assert!(registry.get("missing").is_none());
    }

    #[test]
    fn test_registry_definitions_filtering() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(MockTool));
        let defs = registry.definitions(&[], &["mock".into()]);
        assert!(defs.is_empty());
    }
}
