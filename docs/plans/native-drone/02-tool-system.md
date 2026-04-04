# Plan 02: Runtime Tool System

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the unified tool registry supporting built-in tools, MCP server tools, and external binary tools. All tools present the same interface to the conversation loop.

**Architecture:** `Tool` trait with `ToolRegistry` dispatching by name. Built-in tools implement the trait directly. MCP and external tools are wrapped in proxy types. This plan is split into three dispatchable chunks: core + file ops (02a), bash + git + test runner (02b), MCP + external tools (02c).

**Tech Stack:** tokio (process spawning), serde_json (tool I/O), globset (glob matching), grep-regex or regex (content search)

**Spec:** `docs/specs/native-drone/02-runtime-tool-system.md`

**Reference:** `rust/crates/tools/src/` in Claw Code repo

---

## Part A: Core Types + File Operations

### Task 1: Tool trait and registry

**Files:**
- Create: `src/runtime/src/tools/registry.rs`
- Create: `src/runtime/src/tools/types.rs`
- Modify: `src/runtime/src/tools/mod.rs`

- [ ] **Step 1: Define core types**

Create `src/runtime/src/tools/types.rs`:

```rust
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
```

- [ ] **Step 2: Define Tool trait and ToolRegistry**

Create `src/runtime/src/tools/registry.rs`:

```rust
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
        fn name(&self) -> &str { "mock" }
        fn description(&self) -> &str { "a mock tool" }
        fn input_schema(&self) -> serde_json::Value { serde_json::json!({"type": "object"}) }
        fn permission(&self) -> PermissionLevel { PermissionLevel::ReadOnly }
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
```

- [ ] **Step 3: Update mod.rs**

```rust
pub mod registry;
pub mod types;

pub use registry::{Tool, ToolRegistry};
pub use types::*;
```

- [ ] **Step 4: Create EventSink trait stub**

Update `src/runtime/src/event.rs`:

```rust
use crate::api::TokenUsage;
use crate::tools::ToolResult;

#[derive(Debug)]
pub enum RuntimeEvent {
    TurnStart { task: String },
    TextDelta(String),
    ToolUseStart { id: String, name: String, input: serde_json::Value },
    ToolUseEnd { id: String, name: String, result: ToolResult, duration_ms: u64 },
    Usage(TokenUsage),
    Heartbeat,
    CompactionTriggered { reason: String, tokens_before: u32 },
    CheckpointCreated { artifact_id: String },
    TurnEnd { iterations: u32, total_usage: TokenUsage },
    Error(String),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent);
}

/// No-op event sink for testing
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _event: RuntimeEvent) {}
}
```

- [ ] **Step 5: Run tests, verify build**

Run: `cd src/runtime && cargo test`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/runtime/
git commit -m "add Tool trait, ToolRegistry, and core tool types"
```

---

### Task 2: File operation tools

**Files:**
- Create: `src/runtime/src/tools/file_ops.rs`
- Modify: `src/runtime/src/tools/mod.rs`
- Modify: `src/runtime/Cargo.toml`

- [ ] **Step 1: Implement read_file tool with tests**

Create `src/runtime/src/tools/file_ops.rs`. Implement `ReadFileTool`:

- Input: `{ "file_path": string, "offset": optional int, "limit": optional int }`
- Validates path is within workspace
- Reads file, adds line numbers (`cat -n` style)
- Returns markdown output
- Tests: read existing file, read with offset/limit, reject path outside workspace

- [ ] **Step 2: Implement write_file tool with tests**

`WriteFileTool`:
- Input: `{ "file_path": string, "content": string }`
- Validates path within workspace
- Creates parent directories
- Writes content
- Returns markdown confirmation
- Tests: write new file, overwrite existing, create nested dirs, reject outside workspace

- [ ] **Step 3: Implement edit_file tool with tests**

`EditFileTool`:
- Input: `{ "file_path": string, "old_string": string, "new_string": string, "replace_all": optional bool }`
- Validates path within workspace
- Reads file, finds exact match of old_string
- Fails if old_string not found or not unique (unless replace_all)
- Returns markdown diff summary
- Tests: single replacement, replace_all, not found, ambiguous match

- [ ] **Step 4: Implement glob_search tool with tests**

Add `globset = "0.4"` to Cargo.toml.

`GlobSearchTool`:
- Input: `{ "pattern": string, "path": optional string }`
- Walks workspace directory matching pattern
- Returns file paths sorted by modification time
- Tests: match .rs files, match nested patterns, no matches

- [ ] **Step 5: Implement grep_search tool with tests**

Add `regex = "1"` to Cargo.toml.

`GrepSearchTool`:
- Input: `{ "pattern": string, "path": optional string, "glob": optional string, "context": optional int }`
- Regex search through files
- Returns matching lines with file path, line number, context
- Markdown output format
- Tests: simple match, regex pattern, with context lines, file filter

- [ ] **Step 6: Register all file tools in a builder function**

```rust
pub fn register_file_tools(registry: &mut ToolRegistry) {
    registry.register(Box::new(ReadFileTool));
    registry.register(Box::new(WriteFileTool));
    registry.register(Box::new(EditFileTool));
    registry.register(Box::new(GlobSearchTool));
    registry.register(Box::new(GrepSearchTool));
}
```

- [ ] **Step 7: Run all tests, buckify, verify build**

Run: `cd src/runtime && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 8: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "add file operation tools (read, write, edit, glob, grep)"
```

---

## Part B: Bash, Git, Test Runner

### Task 3: Bash tool

**Files:**
- Create: `src/runtime/src/tools/bash.rs`

- [ ] **Step 1: Implement bash tool with tests**

`BashTool`:
- Input: `{ "command": string, "timeout": optional int, "working_dir": optional string }`
- Spawns shell process via `tokio::process::Command`
- Captures stdout, stderr, exit code
- Timeout via `tokio::time::timeout`
- Returns markdown output with stdout/stderr/exit code sections
- Working dir defaults to workspace
- Tests: simple command, timeout, non-zero exit code, working dir

- [ ] **Step 2: Run tests, commit**

Run: `cd src/runtime && cargo test tools::bash`

```bash
git add src/runtime/
git commit -m "add bash tool with timeout and workspace restriction"
```

---

### Task 4: Git tool

**Files:**
- Create: `src/runtime/src/tools/git.rs`

- [ ] **Step 1: Define GitOperation enum and implement git tool with tests**

`GitTool`:
- Input: `{ "operation": string, ...operation-specific fields }`
- Operations: `status`, `diff`, `log`, `create_branch`, `commit`, `push`, `create_pr`, `checkout_file`
- Each operation shells out to `git` or `gh` CLI
- Returns structured markdown output
- The tool itself just translates input to git commands — policy enforcement (branch naming, force push blocking) is done by the drone layer, not here. The runtime tool is a clean interface.
- Tests: parse operation input, format output (mock git not needed — test the parsing/formatting)

- [ ] **Step 2: Run tests, commit**

Run: `cd src/runtime && cargo test tools::git`

```bash
git add src/runtime/
git commit -m "add git tool with structured operations"
```

---

### Task 5: Test runner tool

**Files:**
- Create: `src/runtime/src/tools/test_runner.rs`

- [ ] **Step 1: Implement test runner with cargo test parser and tests**

`TestRunnerTool`:
- Input: `{ "command": string, "filter": optional string, "working_dir": optional string }`
- Runs command, parses output
- Cargo test parser: regex for `test result: ok. N passed; M failed; K ignored`
- Individual failure extraction: `test name ... FAILED` lines
- Falls back to raw output for unknown formats
- Returns structured markdown: pass/fail/skip counts, failure details
- Tests: parse cargo test output (success, mixed, all fail), unknown format fallback

- [ ] **Step 2: Run tests, commit**

Run: `cd src/runtime && cargo test tools::test_runner`

```bash
git add src/runtime/
git commit -m "add test runner tool with cargo test output parsing"
```

---

### Task 6: Register Part B tools

**Files:**
- Modify: `src/runtime/src/tools/mod.rs`

- [ ] **Step 1: Add modules and registration**

Update `src/runtime/src/tools/mod.rs`:

```rust
pub mod registry;
pub mod types;
pub mod file_ops;
pub mod bash;
pub mod git;
pub mod test_runner;

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
```

- [ ] **Step 2: Run all tests, buckify, verify build**

Run: `cd src/runtime && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 3: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "register all built-in tools in default registry"
```

---

## Part C: MCP + External Binary Tools

### Task 7: MCP client tool proxy

**Files:**
- Create: `src/runtime/src/tools/mcp.rs`

- [ ] **Step 1: Implement MCP JSON-RPC client with tests**

This is the runtime's MCP client for connecting to MCP servers (like Overseer). Implement:

- `McpClient` — manages a connection to a single MCP server
  - HTTP transport: POST JSON-RPC to `{url}` endpoint
  - Stdio transport: spawn process, JSON-RPC over stdin/stdout with `Content-Length` framing
- `initialize` handshake (send `initialize`, wait for response, send `initialized` notification)
- `tools/list` — discover available tools
- `tools/call` — execute a tool

- `McpToolProxy` — wraps an MCP tool as a `Tool` impl
  - Name: `mcp__{server_name}__{tool_name}`
  - Schema: from MCP `tools/list` response
  - Execute: proxy to `tools/call`

Tests:
- JSON-RPC request/response serialization
- Tool name namespacing
- Error handling (server disconnect, invalid response)

- [ ] **Step 2: Add McpManager for multi-server management**

```rust
pub struct McpManager {
    clients: HashMap<String, McpClient>,
}

impl McpManager {
    pub async fn connect_all(configs: &[McpServerConfig]) -> Result<Self, ...> { ... }
    pub fn register_tools(&self, registry: &mut ToolRegistry) { ... }
    pub async fn shutdown_all(&mut self) { ... }
}
```

- [ ] **Step 3: Run tests, commit**

```bash
git add src/runtime/
git commit -m "add MCP client with tool proxy and multi-server management"
```

---

### Task 8: External binary tool

**Files:**
- Create: `src/runtime/src/tools/external.rs`

- [ ] **Step 1: Implement external tool with tests**

`ExternalTool`:
- Configured from `drone.toml` `[tools.external.*]` sections
- Spawns binary with JSON on stdin, reads JSON from stdout
- Protocol: stdin receives tool input, stdout returns `{ "output": "...", "is_error": false }`
- Non-zero exit = error, stderr captured for metadata
- Configurable timeout
- Output format conversion (JSON → markdown if configured)
- Embedded binary support: extract from `include_bytes!` to cache dir on first use

Tests:
- Mock external tool (write a small test binary or use `echo`)
- JSON protocol roundtrip
- Timeout handling
- Non-zero exit code
- Output format conversion

- [ ] **Step 2: Run tests, buckify, verify full build**

Run: `cd src/runtime && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/runtime:runtime`

- [ ] **Step 3: Commit**

```bash
git add src/runtime/ Cargo.lock third-party/BUCK
git commit -m "add external binary tool support with JSON protocol"
```
