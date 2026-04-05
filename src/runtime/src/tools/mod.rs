pub mod agent;
pub mod bash;
pub mod external;
pub mod file_ops;
pub mod git;
pub mod mcp;
pub mod registry;
pub mod test_runner;
pub mod types;

pub use registry::{Tool, ToolRegistry};
pub use types::*;

use std::sync::Arc;

/// Create a registry with all built-in tools
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    file_ops::register_file_tools(&mut registry);
    registry.register(Arc::new(bash::BashTool));
    registry.register(Arc::new(git::GitTool));
    registry.register(Arc::new(test_runner::TestRunnerTool));
    registry
}
