# Plan 05: Drone Config & Prompts

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `drone.toml` configuration system, the config hierarchy (defaults → toml → job spec → stage), the repo/tool cache, and the system prompt builder with priority-based sections.

**Architecture:** `DroneConfig` parsed from TOML, merged with job spec overrides and stage defaults into `ResolvedConfig`. `PromptBuilder` assembles prioritized sections. Cache manager handles bare repo cloning and tool artifact caching.

**Tech Stack:** toml (config parsing), serde (deserialization), globset (protected paths)

**Spec:** `docs/specs/native-drone/05-drone-config-and-prompts.md`

---

### Task 1: drone.toml configuration types

**Files:**
- Create: `src/drones/native/src/config.rs`

- [ ] **Step 1: Define config types with deserialization tests**

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use serde::Deserialize;
use runtime::api::ProviderConfig;

#[derive(Debug, Deserialize)]
pub struct DroneConfig {
    pub provider: ProviderSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
    #[serde(default)]
    pub cache: CacheSection,
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub mcp: HashMap<String, McpSection>,
    #[serde(default)]
    pub environment: EnvironmentSection,
    #[serde(default)]
    pub health_checks: Vec<CustomHealthCheck>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderSection {
    pub kind: String,               // "anthropic" | "openai-compat"
    #[serde(default)]
    pub base_url: Option<String>,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
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

#[derive(Debug, Deserialize, Default)]
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

#[derive(Debug, Deserialize, Default)]
pub struct GitSection {
    #[serde(default = "default_branch")]
    pub default_branch: String,
    #[serde(default = "default_prefix")]
    pub branch_prefix: String,
    #[serde(default = "default_true")]
    pub auto_commit: bool,
    #[serde(default = "default_true")]
    pub pr_on_complete: bool,
    #[serde(default)]
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct McpSection {
    pub kind: String,
    pub url: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct EnvironmentSection {
    #[serde(default)]
    pub extra_path: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
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

// Default value functions
fn default_max_tokens() -> u32 { 8192 }
fn default_max_iterations() -> u32 { 50 }
fn default_timeout() -> u64 { 7200 }
fn default_compaction_strategy() -> String { "checkpoint".into() }
fn default_compaction_threshold() -> u32 { 80000 }
fn default_compaction_preserve() -> u32 { 6 }
fn default_cache_dir() -> PathBuf { PathBuf::from("/var/cache/kerrigan/drone") }
fn default_true() -> bool { true }
fn default_cache_size() -> u64 { 2048 }
fn default_branch() -> String { "main".into() }
fn default_prefix() -> String { "kerrigan/".into() }
fn default_permission() -> String { "read-only".into() }
fn default_output_format() -> String { "markdown".into() }
fn default_tool_timeout() -> u64 { 30 }
fn default_health_timeout() -> u64 { 30 }

impl DroneConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn to_provider_config(&self) -> ProviderConfig {
        match self.provider.kind.as_str() {
            "anthropic" => ProviderConfig::Anthropic {
                api_key: self.provider.api_key.clone().unwrap_or_default(),
                model: self.provider.model.clone(),
                base_url: self.provider.base_url.clone(),
            },
            _ => ProviderConfig::OpenAiCompat {
                base_url: self.provider.base_url.clone().unwrap_or_else(|| "http://localhost:11434/v1".into()),
                api_key: self.provider.api_key.clone(),
                model: self.provider.model.clone(),
            },
        }
    }
}
```

Write a test that parses a sample `drone.toml` string and verifies all fields. Write a test for `to_provider_config` for both provider kinds.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add drone.toml configuration types with defaults"
```

---

### Task 2: Configuration hierarchy and resolution

**Files:**
- Create: `src/drones/native/src/resolve.rs`

- [ ] **Step 1: Implement ResolvedConfig merging**

```rust
use crate::config::DroneConfig;
use crate::pipeline::{Stage, StageConfig, StageGitConfig};
use runtime::api::ProviderConfig;
use runtime::conversation::loop_core::{LoopConfig, CompactionStrategy};

pub struct ResolvedConfig {
    pub provider: ProviderConfig,
    pub loop_config: LoopConfig,
    pub stage_config: StageConfig,
    pub cache: CacheConfig,
    pub environment: EnvironmentConfig,
}

impl ResolvedConfig {
    pub fn resolve(
        drone_toml: &DroneConfig,
        job_config: &std::collections::HashMap<String, String>,
        stage: Stage,
    ) -> Self {
        // Start from drone.toml
        let mut provider = drone_toml.to_provider_config();

        // Override from job spec
        if let Some(model) = job_config.get("model") {
            match &mut provider {
                ProviderConfig::Anthropic { model: m, .. } => *m = model.clone(),
                ProviderConfig::OpenAiCompat { model: m, .. } => *m = model.clone(),
            }
        }
        // ... override timeout, max_iterations, etc from job_config

        // Apply stage defaults
        let stage_config = stage.default_config();

        // Merge git config (drone.toml + stage defaults)
        // ...

        Self { provider, loop_config, stage_config, cache, environment }
    }
}
```

Tests: verify job spec overrides win over drone.toml, stage defaults apply correctly.

- [ ] **Step 2: Run tests, commit**

```bash
git add src/drones/native/
git commit -m "add config resolution with hierarchy merging"
```

---

### Task 3: Repo cache manager

**Files:**
- Create: `src/drones/native/src/cache.rs`

- [ ] **Step 1: Implement bare repo cache with tests**

```rust
use std::path::{Path, PathBuf};

pub struct RepoCache {
    cache_dir: PathBuf,
}

impl RepoCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get or create a cached bare repo, then create a worktree for this job
    pub async fn checkout(
        &self,
        repo_url: &str,
        branch: &str,
        worktree_path: &Path,
    ) -> anyhow::Result<()> {
        let bare_path = self.bare_repo_path(repo_url);

        if bare_path.exists() {
            // Fetch latest
            self.git_fetch(&bare_path).await?;
        } else {
            // Clone bare
            self.git_clone_bare(repo_url, &bare_path).await?;
        }

        // Create worktree
        self.git_worktree_add(&bare_path, branch, worktree_path).await?;
        Ok(())
    }

    pub async fn cleanup_worktree(&self, repo_url: &str, worktree_path: &Path) -> anyhow::Result<()> {
        let bare_path = self.bare_repo_path(repo_url);
        // git worktree remove
        let _ = tokio::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(worktree_path)
            .current_dir(&bare_path)
            .output()
            .await;
        Ok(())
    }

    fn bare_repo_path(&self, url: &str) -> PathBuf {
        let hash = blake3::hash(url.as_bytes()).to_hex();
        self.cache_dir.join("repos").join(&hash[..16]).with_extension("git")
    }

    async fn git_clone_bare(&self, url: &str, path: &Path) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let output = tokio::process::Command::new("git")
            .args(["clone", "--bare", url])
            .arg(path)
            .output()
            .await?;
        anyhow::ensure!(output.status.success(), "git clone bare failed");
        Ok(())
    }

    async fn git_fetch(&self, bare_path: &Path) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(bare_path)
            .output()
            .await?;
        anyhow::ensure!(output.status.success(), "git fetch failed");
        Ok(())
    }

    async fn git_worktree_add(
        &self,
        bare_path: &Path,
        branch: &str,
        worktree_path: &Path,
    ) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(["worktree", "add"])
            .arg(worktree_path)
            .arg(branch)
            .current_dir(bare_path)
            .output()
            .await?;
        anyhow::ensure!(output.status.success(), "git worktree add failed");
        Ok(())
    }
}
```

Add `blake3 = "1"` to Cargo.toml.

Tests: bare_repo_path hashing is deterministic, cleanup is idempotent. Integration test with a real git repo is optional (can use a temp dir with `git init --bare`).

- [ ] **Step 2: Run tests, buckify, commit**

Run: `./tools/buckify.sh`

```bash
git add src/drones/native/ Cargo.lock third-party/BUCK
git commit -m "add repo cache with bare clone and worktree checkout"
```

---

### Task 4: System prompt builder

**Files:**
- Create: `src/drones/native/src/prompt.rs`

- [ ] **Step 1: Implement PromptBuilder with priority sections**

```rust
pub struct PromptSection {
    pub name: String,
    pub content: String,
    pub priority: u8,
}

pub struct PromptBuilder {
    sections: Vec<PromptSection>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self { sections: Vec::new() }
    }

    pub fn add(&mut self, name: &str, content: String, priority: u8) {
        self.sections.push(PromptSection {
            name: name.to_string(),
            content,
            priority,
        });
    }

    /// Build the full system prompt, sorted by priority (highest first)
    pub fn build(&self) -> Vec<String> {
        let mut sorted: Vec<_> = self.sections.iter().collect();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority));
        sorted.iter().map(|s| s.content.clone()).collect()
    }

    /// Build with a token budget — drop lowest priority sections first
    pub fn build_within_budget(&self, max_tokens: u32) -> Vec<String> {
        let mut sorted: Vec<_> = self.sections.iter().collect();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

        let mut result = Vec::new();
        let mut tokens = 0;
        for section in sorted {
            let section_tokens = (section.content.len() as u32) / 4;
            if tokens + section_tokens <= max_tokens {
                result.push(section.content.clone());
                tokens += section_tokens;
            }
        }
        result
    }
}
```

- [ ] **Step 2: Implement stage-specific prompt generators**

```rust
impl PromptBuilder {
    pub fn for_stage(
        stage: &Stage,
        stage_config: &StageConfig,
        registry: &runtime::tools::ToolRegistry,
        workspace_context: &str,       // CLAUDE.md contents
        task_state: Option<&str>,
        checkpoint_ref: Option<&str>,
    ) -> Self {
        let mut builder = Self::new();

        // Priority 255: Identity
        builder.add("identity", "You are a software development agent working in the kerrigan platform. You execute tasks precisely, commit frequently, and report progress.".into(), 255);

        // Priority 255: Environment
        builder.add("environment", format!(
            "Working directory: {}\nDate: {}\nModel: {}",
            std::env::current_dir().unwrap_or_default().display(),
            chrono::Utc::now().format("%Y-%m-%d"),
            "configured-model",
        ), 255);

        // Priority 200: Stage mission
        builder.add("mission", stage_config.system_prompt.clone(), 200);

        // Priority 180: Tool guide (auto-generated)
        let tool_guide = build_tool_guide(registry, stage_config);
        builder.add("tools", tool_guide, 180);

        // Priority 180: Git rules
        let git_rules = build_git_rules(&stage_config.git);
        builder.add("git_rules", git_rules, 180);

        // Priority 150: Project context
        if !workspace_context.is_empty() {
            builder.add("project_context", workspace_context.to_string(), 150);
        }

        // Priority 150: Task state
        if let Some(state) = task_state {
            builder.add("task_state", state.to_string(), 150);
        }

        // Priority 100: Constraints
        let constraints = build_constraints(stage_config);
        if !constraints.is_empty() {
            builder.add("constraints", constraints, 100);
        }

        // Priority 50: Checkpoint reference
        if let Some(ref_text) = checkpoint_ref {
            builder.add("checkpoint", ref_text.to_string(), 50);
        }

        builder
    }
}

fn build_tool_guide(registry: &runtime::tools::ToolRegistry, config: &StageConfig) -> String {
    let defs = registry.definitions(&config.allowed_tools, &config.denied_tools);
    let mut guide = "## Available Tools\n\n".to_string();
    for def in defs {
        guide.push_str(&format!("- **{}**: {}\n", def.name, def.description));
    }
    guide
}

fn build_git_rules(git: &StageGitConfig) -> String {
    let mut rules = "## Git Rules\n\n".to_string();
    if let Some(branch) = &git.branch_name {
        rules.push_str(&format!("- Work on branch: `{branch}`\n"));
    }
    if !git.protected_paths.is_empty() {
        rules.push_str(&format!("- Do NOT modify: {}\n", git.protected_paths.join(", ")));
    }
    rules.push_str("- Do NOT force push\n");
    rules.push_str("- Commit frequently with clear messages\n");
    rules
}

fn build_constraints(config: &StageConfig) -> String {
    let mut constraints = Vec::new();
    if !config.denied_tools.is_empty() {
        constraints.push(format!("Do NOT use these tools: {}", config.denied_tools.join(", ")));
    }
    constraints.push("Do NOT install system packages".into());
    constraints.push("Do NOT add features beyond what was asked".into());
    constraints.push("Do NOT refactor unrelated code".into());
    constraints.join("\n")
}
```

Add `chrono = "0.4"` to Cargo.toml.

Tests: verify section ordering by priority, budget trimming drops low-priority sections first, stage-specific content is correct.

- [ ] **Step 3: Run tests, buckify, verify build**

Run: `cd src/drones/native && cargo test`
Run: `./tools/buckify.sh`
Run: `buck2 build root//src/drones/native:native-drone`

- [ ] **Step 4: Commit**

```bash
git add src/drones/native/ Cargo.lock third-party/BUCK
git commit -m "add system prompt builder with priority sections and stage generators"
```
