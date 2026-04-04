# Drone: Configuration & Prompts

**Date:** 2026-04-04
**Parent:** [00-overview.md](00-overview.md)

## Purpose

Configuration hierarchy from static file to per-run overrides, and system prompt construction that generates stage-specific instructions from code rather than injected markdown files.

## drone.toml

Per-deployment configuration file. Read at drone startup from `DRONE_CONFIG` env var or `/etc/kerrigan/drone.toml`.

```toml
[provider]
kind = "openai-compat"              # "anthropic" | "openai-compat"
base_url = "http://localhost:11434/v1"
model = "qwen3:32b"
# api_key = "sk-..."                # or env: DRONE_API_KEY

[runtime]
max_tokens = 8192
max_iterations = 50
temperature = 0.0
timeout_secs = 7200
compaction_strategy = "checkpoint"       # "checkpoint" | "summarize"
compaction_threshold_tokens = 80000      # maps to LoopConfig.max_context_tokens
compaction_preserve_recent = 6           # preserve last N messages on compaction

[cache]
dir = "/var/cache/kerrigan/drone"
repo_cache = true                    # bare repo cache, fetch-only on subsequent runs
tool_cache = true                    # cache tool artifacts by content hash
max_size_mb = 2048

[git]
default_branch = "main"
branch_prefix = "kerrigan/"
auto_commit = true
pr_on_complete = true
protected_paths = ["CLAUDE.md", "Cargo.lock"]

[tools]
sandbox = true
allowed = []                         # empty = all
denied = []

[tools.external.creep-search]
binary = "creep-cli"
args = ["search"]
description = "Search indexed files by glob pattern"
input_schema_path = "tools/creep-search.schema.json"
permission = "read-only"
output_format = "markdown"
timeout_secs = 30

[mcp.overseer]
kind = "http"
url = "http://localhost:3100/mcp"

# Creep is accessed via the creep-cli external tool, not MCP (it's gRPC, not MCP)
# See [tools.external.creep-search] above
```

## Configuration Hierarchy

Merge order (later wins):

```
1. Compiled defaults        — sensible defaults in Rust code
2. drone.toml               — per-deployment (machine-level)
3. Job spec config           — per-run (from Overseer via Queen)
4. Stage defaults            — per-stage tool/git policy
```

```rust
pub struct ResolvedConfig {
    pub provider: ProviderConfig,
    pub runtime: LoopConfig,
    pub cache: CacheConfig,
    pub git: StageGitConfig,
    pub tools: ToolConfig,
    pub mcp: Vec<McpServerConfig>,
    pub stage: StageConfig,
}

impl ResolvedConfig {
    pub fn resolve(
        drone_toml: DroneConfig,
        job: &JobSpec,
        stage: Stage,
    ) -> Self {
        let mut config = Self::from_defaults();
        config.merge_drone_toml(drone_toml);
        config.merge_job_spec(job);
        config.apply_stage_defaults(stage);
        config
    }
}
```

**Job spec overrides** — these fields in `job.config` override drone.toml:
- `provider`, `model`, `api_key` — switch providers per job
- `timeout_secs` — per-job timeout
- `max_iterations` — per-job iteration limit
- `branch_name` — explicit branch for this run
- `system_prompt` — additional prompt content (freeform stage)

## Cache Strategy

### Repo Cache

Instead of a fresh shallow clone per job:

```
/var/cache/kerrigan/drone/repos/
  {url-hash}/
    bare.git             # bare repo, maintained across runs
```

On job start:
1. If bare repo exists: `git fetch origin` (incremental)
2. If not: `git clone --bare {url}` (one-time)
3. Create worktree: `git worktree add /tmp/drone-{id}/workspace {branch}`
4. On teardown: `git worktree remove`

Saves clone time on large repos (minutes → seconds).

### Tool Cache

```
/var/cache/kerrigan/drone/tools/
  {tool-name}/
    {input-hash}.result  # cached tool result
```

Opt-in per tool. Useful for:
- Embedded binary extraction (extract once, reuse)
- Test results for unchanged code (keyed by source hash)
- MCP schema caching (tool definitions don't change per-session)

Cache eviction: LRU by access time, bounded by `max_size_mb`.

## System Prompt Construction

Prompts are built in Rust from composable, prioritized sections:

```rust
pub struct PromptBuilder {
    sections: Vec<PromptSection>,
}

pub struct PromptSection {
    pub name: String,
    pub content: String,
    pub priority: u8,
}
```

### Section Hierarchy

```
Priority 255 (never dropped):
  Identity       — "You are a software development agent working in the kerrigan platform."
  Environment    — cwd, date, git branch, model name, available tools summary

Priority 200:
  Stage Mission  — stage-specific instructions and goals

Priority 180:
  Tool Guide     — auto-generated from tool registry: name, description, when to use
  Git Rules      — branch/commit/PR policy for this stage

Priority 150:
  Project Context — CLAUDE.md / repo-level instructions (read from workspace)
  Task State      — current task, completed tasks, remaining tasks (from orchestrator)

Priority 100:
  Constraints    — protected paths, denied operations, scope limits

Priority 50:
  Checkpoint Ref — "Previous work in artifact {id}, summary: ..."
```

### Prompt Content Principles

Derived from superpowers skill patterns:

1. **Role framing** — clear identity and mission. "You are implementing task 3 of 7: Add auth middleware."
2. **Structured checklists** — not prose. "Before committing: 1) Run tests 2) Check diff 3) Write clear message."
3. **Anti-pattern guards** — explicit "do NOT" directives for known failure modes. "Do NOT add features beyond the task description. Do NOT refactor unrelated code."
4. **Scope anchoring** — remind the agent what's in and out of scope at every checkpoint.
5. **Exit criteria** — tell the agent exactly what "done" looks like. "This task is complete when: tests pass, file X exists, PR is created."
6. **Conciseness** — every token in the system prompt competes with working context. Be direct.

### Auto-generated Tool Guide

Built from the registry, not maintained by hand:

```rust
fn build_tool_guide(registry: &ToolRegistry, stage: &StageConfig) -> String {
    let tools = registry.available_tools(&stage.allowed_tools, &stage.denied_tools);
    let mut guide = String::from("## Available Tools\n\n");
    for tool in tools {
        guide.push_str(&format!("- **{}**: {}\n", tool.name(), tool.description()));
    }
    guide
}
```

This guarantees the prompt matches reality. No drift between tool implementations and documentation.

### Sub-agent Prompts

Sub-agents get a reduced prompt:
- Identity (inherited)
- Focused mission ("Implement: {task description}")
- Tool guide (scoped to allowed tools)
- Relevant file contents (pre-loaded, not discovered)
- Git rules (commit allowed, branch inherited)
- No project context, no checkpoint refs (parent handles that)

This keeps sub-agent context small and focused.
