# Runtime: Tool System

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

Unified tool registry supporting built-in tools, MCP server tools, and external binary tools. All tools present the same interface to the conversation loop regardless of implementation.

## Core Types

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn permission(&self) -> PermissionLevel;
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolRegistry {
    builtin: HashMap<String, Box<dyn Tool>>,
    mcp: HashMap<String, McpToolProxy>,
    external: HashMap<String, ExternalTool>,
}

pub struct ToolContext {
    pub workspace: PathBuf,
    pub home: PathBuf,
    pub event_sink: Arc<dyn EventSink>,
    pub cache: Arc<ToolCache>,
    pub sandbox_config: SandboxConfig,
}

pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub format: OutputFormat,
    pub metadata: Option<serde_json::Value>,
}

pub enum OutputFormat {
    Markdown,
    Json,
    Raw,
}

pub enum PermissionLevel {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}
```

## Built-in Tools

### File Operations

| Tool | Permission | Description |
|------|-----------|-------------|
| `read_file` | ReadOnly | Read file contents with line numbers. Supports offset/limit for large files. |
| `write_file` | WorkspaceWrite | Create or overwrite a file. Requires prior read for existing files. |
| `edit_file` | WorkspaceWrite | Exact string replacement in files. Fails if old_string not unique. |
| `glob_search` | ReadOnly | File pattern matching. Returns paths sorted by modification time. |
| `grep_search` | ReadOnly | Regex content search via ripgrep semantics. Supports context lines, file type filters. |

All file ops are restricted to the workspace directory. Paths outside workspace are rejected.

### Bash

| Tool | Permission | Description |
|------|-----------|-------------|
| `bash` | FullAccess | Execute shell command. Returns stdout, stderr, exit code. Configurable timeout. |

Sandboxing via Linux namespaces when available:
- Filesystem: workspace + /tmp + read-only system paths
- Network: allowed by default, configurable deny
- Process: isolated PID namespace

Falls back to basic restriction (workspace-only cwd) when namespaces unavailable (e.g., inside containers).

### Git

| Tool | Permission | Description |
|------|-----------|-------------|
| `git` | WorkspaceWrite | Structured git operations. |

```rust
pub enum GitOperation {
    CreateBranch { name: String, from: Option<String> },
    Commit { message: String, paths: Vec<String> },
    Push { force: bool },
    Status,
    Diff { staged: bool },
    Log { count: u32 },
    CreatePr { title: String, body: String, base: Option<String> },
    CheckoutFile { path: String, ref_: String },
}
```

The LLM cannot run raw git via bash (if git workflow enforcement is enabled). All git operations go through this tool, which delegates to the drone's `GitWorkflow` for policy enforcement:
- Branch naming validated against config
- Force push denied unless explicitly allowed
- Commits to default branch rejected
- Protected paths enforced
- **Per-operation filtering**: `StageGitConfig` specifies `allowed_operations: Option<Vec<GitOperationKind>>`. When set, only listed operation kinds are permitted (e.g., `[Status, Diff, Log]` for read-only stages). When `None`, all operations allowed. This is enforced by `GitWorkflow`, not the tool registry — the `git` tool stays as one tool with operation-level gating inside.

Output is structured markdown:
```markdown
## Commit created
- **SHA:** abc1234
- **Branch:** kerrigan/implement-auth
- **Files:** 3 changed (+42, -7)
```

### Test Runner

| Tool | Permission | Description |
|------|-----------|-------------|
| `test` | FullAccess | Run tests with structured result parsing. |

```rust
pub struct TestRequest {
    pub command: String,
    pub filter: Option<String>,
    pub working_dir: Option<String>,
}

pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub failures: Vec<TestFailure>,
    pub duration_ms: u64,
}

pub struct TestFailure {
    pub name: String,
    pub message: String,
    pub location: Option<String>,
}
```

Parses `cargo test` output natively. Other frameworks fall back to raw output with exit code — additional parsers can be added later. Returns structured markdown:
```markdown
## Test Results: 14 passed, 1 failed

### Failures
- `test_parse_config` (src/config.rs:142) — assertion failed: expected 3, got 2
```

Unknown test frameworks fall back to raw output with exit code.

### Sub-agent

| Tool | Permission | Description |
|------|-----------|-------------|
| `agent` | FullAccess | Spawn a child conversation loop with scoped context. |

```rust
pub struct AgentRequest {
    pub task: String,
    pub tools: Option<Vec<String>>,    // tool allowlist, None = inherit parent
    pub max_iterations: Option<u32>,
    pub files: Option<Vec<String>>,    // relevant files to include in context
}
```

Creates a new `ConversationLoop` with its own session. Shares the same `ToolContext` (workspace, cache, event sink). Returns the agent's final text response. The parent's context gets only the task + result, not the full sub-agent conversation.

## MCP Tools

MCP servers are configured in `drone.toml` or via the drone's resolved config:

```rust
pub enum McpTransport {
    Stdio { command: String, args: Vec<String>, env: HashMap<String, String> },
    Http { url: String, headers: HashMap<String, String> },
}
```

At startup, the runtime:
1. Connects to each configured MCP server
2. Calls `tools/list` to discover available tools
3. Registers each as `McpToolProxy` in the registry with `mcp__{server}__{tool}` naming

Tool execution proxies the JSON input to the MCP server and returns the result. MCP tools get their own `PermissionLevel` based on config (default: FullAccess).

## External Binary Tools

Configured in `drone.toml`:

```toml
[tools.external.creep-search]
binary = "creep-cli"
args = ["search"]
description = "Search indexed files by glob pattern"
input_schema_path = "tools/creep-search.schema.json"
permission = "read-only"
output_format = "markdown"
timeout_secs = 30

[tools.external.custom-linter]
binary = "/opt/tools/lint"
embedded = false
description = "Run project linter"
input_schema_path = "tools/linter.schema.json"
permission = "workspace-write"
```

**Protocol:** JSON on stdin, JSON on stdout:
- Input: the tool's `input` value (whatever the LLM provided matching the schema)
- Output: `{ "output": "...", "is_error": false }`
- Non-zero exit code = error, stderr captured for metadata

**Embedded binaries:** When `embedded = true`, the binary is compiled into the drone via `include_bytes!` and extracted to the cache directory on first run. Resolved by name from cache before PATH.

**Output format:** When `output_format = "markdown"`, JSON output is converted to markdown. Optional `output_template` for custom formatting.

## Tool Definition Generation

The registry auto-generates tool definitions for the API request:

```rust
impl ToolRegistry {
    pub fn definitions(&self, allowed: &[String], denied: &[String]) -> Vec<ToolDefinition> {
        self.all_tools()
            .filter(|t| allowed.is_empty() || allowed.contains(&t.name().to_string()))
            .filter(|t| !denied.contains(&t.name().to_string()))
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}
```

This feeds both the API request (so the LLM knows what tools exist) and the prompt builder (auto-generated tool guide section).
