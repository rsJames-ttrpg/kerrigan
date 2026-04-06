# Unified DroneToml Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate the native drone's `DroneConfig` and the SDK's `DroneToml` into a single struct in `drone-sdk`, eliminating duplication.

**Architecture:** Move all config section structs from `src/drones/native/src/config.rs` into `src/drone-sdk/src/drone_toml.rs`. Delete the native drone's `config.rs`. Update the native drone's imports to use the SDK. Keep `to_provider_config()` in the native drone since it depends on the `runtime` crate.

**Tech Stack:** Rust, serde, toml

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/drone-sdk/src/drone_toml.rs` | Modify | Add all config section structs, defaults, and tests |
| `src/drones/native/src/config.rs` | Delete | All content moves to SDK |
| `src/drones/native/src/main.rs` | Modify | Remove `mod config;` |
| `src/drones/native/src/drone.rs` | Modify | Update imports from `crate::config` to `drone_sdk::drone_toml` |
| `src/drones/native/src/resolve.rs` | Modify | Update imports, add `to_provider_config()` as local helper |

---

### Task 1: Add new section structs to SDK's DroneToml

**Files:**
- Modify: `src/drone-sdk/src/drone_toml.rs`

- [ ] **Step 1: Add the new structs and default functions after existing code**

Add the following after the existing `PromptsSection` struct (after line 65) in `src/drone-sdk/src/drone_toml.rs`. These are moved verbatim from `src/drones/native/src/config.rs`:

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct ProviderSection {
    pub kind: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RuntimeSection {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_compaction_strategy")]
    pub compaction_strategy: String,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold_tokens: u32,
    #[serde(default = "default_compaction_preserve")]
    pub compaction_preserve_recent: u32,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            max_iterations: default_max_iterations(),
            temperature: None,
            timeout_secs: default_timeout(),
            compaction_strategy: default_compaction_strategy(),
            compaction_threshold_tokens: default_compaction_threshold(),
            compaction_preserve_recent: default_compaction_preserve(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CacheSection {
    #[serde(default = "default_cache_dir")]
    pub dir: PathBuf,
    #[serde(default = "default_true")]
    pub repo_cache: bool,
    #[serde(default = "default_true")]
    pub tool_cache: bool,
    #[serde(default = "default_cache_size")]
    pub max_size_mb: u64,
}

impl Default for CacheSection {
    fn default() -> Self {
        Self {
            dir: default_cache_dir(),
            repo_cache: true,
            tool_cache: true,
            max_size_mb: default_cache_size(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolsSection {
    #[serde(default = "default_true")]
    pub sandbox: bool,
    #[serde(default)]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub denied: Vec<String>,
    #[serde(default)]
    pub external: HashMap<String, ExternalToolSection>,
}

impl Default for ToolsSection {
    fn default() -> Self {
        Self {
            sandbox: true,
            allowed: vec![],
            denied: vec![],
            external: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExternalToolSection {
    pub binary: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub description: String,
    #[serde(default)]
    pub input_schema_path: Option<String>,
    #[serde(default = "default_permission")]
    pub permission: String,
    #[serde(default = "default_output_format")]
    pub output_format: String,
    #[serde(default)]
    pub embedded: bool,
    #[serde(default = "default_tool_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct McpSection {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EnvironmentSection {
    #[serde(default)]
    pub extra_path: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

impl Default for EnvironmentSection {
    fn default() -> Self {
        Self {
            extra_path: vec![],
            env: HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CustomHealthCheck {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_health_timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OrchestratorSection {
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default = "default_max_fixup_iterations")]
    pub max_fixup_iterations: u32,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

impl Default for OrchestratorSection {
    fn default() -> Self {
        Self {
            test_command: None,
            max_fixup_iterations: default_max_fixup_iterations(),
            max_parallel: default_max_parallel(),
        }
    }
}
```

- [ ] **Step 2: Add the new default functions**

Add these after the existing default functions (after `default_true`):

```rust
fn default_max_tokens() -> u32 {
    8192
}
fn default_max_iterations() -> u32 {
    50
}
fn default_timeout() -> u64 {
    7200
}
fn default_compaction_strategy() -> String {
    "checkpoint".into()
}
fn default_compaction_threshold() -> u32 {
    80000
}
fn default_compaction_preserve() -> u32 {
    6
}
fn default_cache_dir() -> PathBuf {
    PathBuf::from("/var/cache/kerrigan/drone")
}
fn default_cache_size() -> u64 {
    2048
}
fn default_permission() -> String {
    "read-only".into()
}
fn default_output_format() -> String {
    "markdown".into()
}
fn default_tool_timeout() -> u64 {
    30
}
fn default_health_timeout() -> u64 {
    30
}
fn default_max_fixup_iterations() -> u32 {
    5
}
fn default_max_parallel() -> usize {
    2
}
```

- [ ] **Step 3: Add the `use` for `PathBuf`**

Add at the top of the file:

```rust
use std::path::PathBuf;
```

- [ ] **Step 4: Expand the `DroneToml` struct with new fields**

Replace the existing `DroneToml` struct with:

```rust
#[derive(Debug, Deserialize, Default)]
pub struct DroneToml {
    #[serde(default)]
    pub provider: Option<ProviderSection>,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub cache: CacheSection,
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub setup: SetupSection,
    #[serde(default)]
    pub prompts: PromptsSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub mcp: HashMap<String, McpSection>,
    #[serde(default)]
    pub environment: EnvironmentSection,
    #[serde(default)]
    pub orchestrator: OrchestratorSection,
    #[serde(default)]
    pub health_checks: Vec<CustomHealthCheck>,
}
```

- [ ] **Step 5: Run existing SDK tests to make sure nothing broke**

Run: `cd src/drone-sdk && cargo test`

Expected: All existing tests pass. The new fields all have defaults so existing test configs (empty string, partial git, etc.) still parse.

- [ ] **Step 6: Add tests for the new sections**

Add these tests in the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn parse_full_native_config() {
    let toml_str = r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-test-key"
base_url = "https://api.anthropic.com"

[runtime]
max_tokens = 4096
max_iterations = 30
temperature = 0.7
timeout_secs = 3600
compaction_strategy = "summarize"
compaction_threshold_tokens = 60000
compaction_preserve_recent = 4

[cache]
dir = "/tmp/drone-cache"
repo_cache = true
tool_cache = false
max_size_mb = 1024

[git]
default_branch = "develop"
branch_prefix = "drone/"
auto_commit = false
pr_on_complete = true
protected_paths = ["CLAUDE.md", ".buckconfig"]

[git.identity.claude]
user_name = "claude-bot"
user_email = "claude@myorg.com"

[tools]
sandbox = true
allowed = ["read_file", "write_file"]
denied = ["bash"]

[tools.external.creep]
binary = "creep-cli"
args = ["search"]
description = "File indexing search"
permission = "read-only"
output_format = "json"
embedded = true
timeout_secs = 10

[mcp.overseer]
kind = "http"
url = "http://localhost:3100/mcp"

[environment]
extra_path = ["/usr/local/bin"]

[environment.env]
RUST_LOG = "debug"

[orchestrator]
test_command = "cargo test --workspace"
max_fixup_iterations = 3
max_parallel = 4

[[health_checks]]
name = "cargo-check"
command = "cargo"
args = ["check"]
required = true
timeout_secs = 60

[setup]
commands = ["./tools/setup-hooks.sh"]

[prompts]
extra_rules = "Use buck2 build"
"#;
    let config: DroneToml = toml::from_str(toml_str).unwrap();

    // Provider
    let provider = config.provider.unwrap();
    assert_eq!(provider.kind, "anthropic");
    assert_eq!(provider.model, "claude-sonnet-4-20250514");
    assert_eq!(provider.api_key.as_deref(), Some("sk-test-key"));

    // Runtime
    assert_eq!(config.runtime.max_tokens, 4096);
    assert_eq!(config.runtime.max_iterations, 30);
    assert_eq!(config.runtime.temperature, Some(0.7));
    assert_eq!(config.runtime.compaction_strategy, "summarize");

    // Cache
    assert_eq!(config.cache.dir, std::path::PathBuf::from("/tmp/drone-cache"));
    assert!(!config.cache.tool_cache);
    assert_eq!(config.cache.max_size_mb, 1024);

    // Git (SDK superset — has identity)
    assert_eq!(config.git.default_branch, "develop");
    let claude_id = config.git_identity("claude");
    assert_eq!(claude_id.user_name, "claude-bot");

    // Tools
    assert!(config.tools.sandbox);
    assert_eq!(config.tools.allowed, vec!["read_file", "write_file"]);
    assert_eq!(config.tools.denied, vec!["bash"]);
    let creep = &config.tools.external["creep"];
    assert_eq!(creep.binary, "creep-cli");
    assert!(creep.embedded);

    // MCP
    assert_eq!(config.mcp["overseer"].kind, "http");
    assert_eq!(config.mcp["overseer"].url, "http://localhost:3100/mcp");

    // Environment
    assert_eq!(config.environment.extra_path, vec!["/usr/local/bin"]);
    assert_eq!(config.environment.env["RUST_LOG"], "debug");

    // Orchestrator
    assert_eq!(config.orchestrator.test_command.as_deref(), Some("cargo test --workspace"));
    assert_eq!(config.orchestrator.max_fixup_iterations, 3);
    assert_eq!(config.orchestrator.max_parallel, 4);

    // Health checks
    assert_eq!(config.health_checks.len(), 1);
    assert_eq!(config.health_checks[0].name, "cargo-check");

    // Setup + prompts (existing SDK sections)
    assert_eq!(config.setup.commands, vec!["./tools/setup-hooks.sh"]);
    assert!(config.prompts.extra_rules.contains("buck2 build"));
}

#[test]
fn parse_no_provider_uses_none() {
    let config: DroneToml = toml::from_str("").unwrap();
    assert!(config.provider.is_none());
    assert_eq!(config.runtime.max_tokens, 8192);
    assert_eq!(config.runtime.max_iterations, 50);
    assert_eq!(config.cache.dir, std::path::PathBuf::from("/var/cache/kerrigan/drone"));
    assert!(config.tools.sandbox);
    assert!(config.mcp.is_empty());
    assert!(config.health_checks.is_empty());
    assert_eq!(config.orchestrator.max_fixup_iterations, 5);
    assert_eq!(config.orchestrator.max_parallel, 2);
}
```

- [ ] **Step 7: Run all SDK tests**

Run: `cd src/drone-sdk && cargo test`

Expected: All tests pass (existing + new).

- [ ] **Step 8: Commit**

```bash
git add src/drone-sdk/src/drone_toml.rs
git commit -m "feat(drone-sdk): expand DroneToml with all config sections from native drone"
```

---

### Task 2: Delete native drone's config.rs and update imports

**Files:**
- Delete: `src/drones/native/src/config.rs`
- Modify: `src/drones/native/src/main.rs`
- Modify: `src/drones/native/src/drone.rs`
- Modify: `src/drones/native/src/resolve.rs`

- [ ] **Step 1: Remove `mod config;` from main.rs**

In `src/drones/native/src/main.rs`, delete line 2:

```rust
mod config;
```

- [ ] **Step 2: Update imports in drone.rs**

In `src/drones/native/src/drone.rs`, replace line 24:

```rust
use crate::config::DroneConfig;
```

with:

```rust
use drone_sdk::drone_toml::DroneToml;
```

Then replace every occurrence of `DroneConfig` with `DroneToml` in `drone.rs`. There are 6 occurrences:
- Line 101-102: `DroneConfig::load(...)` -> `DroneToml::load(...)`
- Line 105: `DroneConfig::default()` -> `DroneToml::default()`
- Line 213-214: `DroneConfig::load(...)` -> `DroneToml::load(...)`
- Line 216: `DroneConfig::default()` -> `DroneToml::default()`
- Line 406: `DroneConfig::load(...)` -> `DroneToml::load(...)`

Also update the `load` call pattern. The SDK's `DroneToml::load()` takes a workspace directory (not a file path) and looks for `drone.toml` inside it. The native drone currently passes a file path. Update the native drone's usage to match. In setup (around lines 99-106), replace:

```rust
let config_path =
    std::env::var("DRONE_CONFIG").unwrap_or_else(|_| "drone.toml".to_string());
let drone_toml = if std::path::Path::new(&config_path).exists() {
    DroneConfig::load(std::path::Path::new(&config_path))?
} else {
    tracing::warn!("No drone.toml found at {config_path}, using defaults");
    DroneConfig::default()
};
```

with:

```rust
let config_dir = std::env::var("DRONE_CONFIG_DIR")
    .unwrap_or_else(|_| ".".to_string());
let drone_toml = DroneToml::load(std::path::Path::new(&config_dir))?;
```

The SDK's `load()` already returns defaults when the file is missing and logs appropriately.

Apply the same pattern to the execute phase (around lines 207-217). Replace the config_meta reload + `DroneConfig::load` with the same `DroneToml::load` approach. Update what's persisted in `config_meta.json` to store the directory path instead of file path.

In setup, change the config_meta persistence (around line 170-171):

```rust
let config_meta = serde_json::json!({
    "drone_config_dir": config_dir,
});
```

In execute (around lines 207-217):

```rust
let config_meta: serde_json::Value = serde_json::from_str(
    &tokio::fs::read_to_string(env.home.join("config_meta.json")).await?,
)?;
let config_dir = config_meta["drone_config_dir"]
    .as_str()
    .unwrap_or(".");
let drone_toml = DroneToml::load(std::path::Path::new(config_dir))?;
let config = ResolvedConfig::resolve(&drone_toml, &job_config, stage);
```

In teardown (around lines 404-409), update similarly:

```rust
let config_dir =
    std::env::var("DRONE_CONFIG_DIR").unwrap_or_else(|_| ".".into());
if let Ok(drone_toml) = DroneToml::load(std::path::Path::new(&config_dir)) {
    let cache = RepoCache::new(drone_toml.cache.dir.clone());
    let _ = cache.cleanup_worktree(repo_url, &env.workspace).await;
}
```

- [ ] **Step 3: Update imports in resolve.rs**

In `src/drones/native/src/resolve.rs`, replace line 7:

```rust
use crate::config::{CacheSection, DroneConfig};
```

with:

```rust
use drone_sdk::drone_toml::{CacheSection, DroneToml};
```

Replace all `DroneConfig` with `DroneToml` throughout the file. Occurrences:
- Line 52: `drone_toml: &DroneConfig` -> `drone_toml: &DroneToml`
- All test functions that use `DroneConfig` in type annotations or `toml::from_str` turbofish

- [ ] **Step 4: Add `to_provider_config()` as a local function in resolve.rs**

The native drone's `DroneConfig` had `to_provider_config()`. Since it depends on `runtime::api::ProviderConfig`, keep it in the native drone. Add this function in `resolve.rs`:

```rust
use drone_sdk::drone_toml::ProviderSection;

/// Convert a ProviderSection from drone.toml into a runtime ProviderConfig.
fn to_provider_config(provider: &ProviderSection) -> ProviderConfig {
    match provider.kind.as_str() {
        "anthropic" => ProviderConfig::Anthropic {
            api_key: provider.api_key.clone().unwrap_or_default(),
            model: provider.model.clone(),
            base_url: provider.base_url.clone(),
        },
        _ => ProviderConfig::OpenAiCompat {
            base_url: provider
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1".into()),
            api_key: provider.api_key.clone(),
            model: provider.model.clone(),
        },
    }
}
```

Update `ResolvedConfig::resolve()` line 57:

```rust
let mut provider = drone_toml.to_provider_config();
```

becomes:

```rust
let mut provider = match &drone_toml.provider {
    Some(p) => to_provider_config(p),
    None => ProviderConfig::Anthropic {
        api_key: String::new(),
        model: "claude-sonnet-4-20250514".to_string(),
        base_url: None,
    },
};
```

- [ ] **Step 5: Update test helpers in resolve.rs**

The `minimal_drone_config()` helper currently requires `[provider]`. Since `provider` is now `Option`, update it:

```rust
fn minimal_drone_config() -> DroneToml {
    toml::from_str(
        r#"
[provider]
kind = "anthropic"
model = "claude-sonnet-4-20250514"
api_key = "sk-base"
"#,
    )
    .unwrap()
}
```

This still works as-is since `[provider]` parses into `Some(ProviderSection{...})`. No changes needed to the helper itself, just the type annotation from `DroneConfig` to `DroneToml`.

All other test TOML strings that start with `[provider]` continue to work — they just parse into `Some(...)` now.

- [ ] **Step 6: Delete config.rs**

Delete `src/drones/native/src/config.rs`.

- [ ] **Step 7: Run native drone tests**

Run: `cd src/drones/native && cargo test`

Expected: All tests pass. The resolve tests still work since they only changed import paths.

- [ ] **Step 8: Run claude drone tests**

Run: `cd src/drones/claude/base && cargo test`

Expected: All tests pass (claude drone already uses SDK's DroneToml).

- [ ] **Step 9: Run full build**

Run: `buck2 build root//...`

Expected: Build succeeds.

- [ ] **Step 10: Verify repo's drone.toml parses correctly**

Add a quick test in the SDK tests:

```rust
#[test]
fn parse_repo_drone_toml_format() {
    let toml_str = r#"
[git]
default_branch = "main"

[git.identity.claude]
user_name = "claude-drone"
user_email = "claude-drone@noreply"

[git.identity.native]
user_name = "native-drone"
user_email = "native-drone@noreply"

[setup]
commands = ["./tools/setup-hooks.sh"]

[prompts]
extra_rules = """
## Build & Test
Use buck2 build, not cargo build.
"""
"#;
    let config: DroneToml = toml::from_str(toml_str).unwrap();
    assert!(config.provider.is_none());
    assert_eq!(config.git.default_branch, "main");
    assert_eq!(config.setup.commands, vec!["./tools/setup-hooks.sh"]);
    assert!(config.prompts.extra_rules.contains("buck2 build"));
    let claude = config.git_identity("claude");
    assert_eq!(claude.user_name, "claude-drone");
}
```

- [ ] **Step 11: Commit**

```bash
git add -A
git commit -m "refactor(native-drone): use SDK DroneToml, delete duplicate config.rs"
```
