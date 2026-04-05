pub mod bash;
pub mod file_ops;
pub mod git;
pub mod registry;
pub mod test_runner;
pub mod types;

pub use registry::{Tool, ToolRegistry};
pub use types::*;

/// Create a registry with all built-in tools
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    file_ops::register_file_tools(&mut registry);
    registry.register(Box::new(bash::BashTool));
    registry.register(Box::new(git::GitTool));
    registry.register(Box::new(test_runner::TestRunnerTool));
    registry
}
