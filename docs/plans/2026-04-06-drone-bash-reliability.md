# Drone Bash Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce drone bash failure rate from 19.1% to ~7-8% by fixing git identity, toolchain PATH, pre-commit hook handling, push-before-PR, and repo-specific prompts.

**Architecture:** Add a `DroneToml` struct to `drone-sdk` for the shared `drone.toml` schema. The Claude drone reads this after clone, uses it to configure git identity, run setup commands, and inject repo-specific prompts into the generated CLAUDE.md. Base CLAUDE.md gets new sections for pre-commit hooks and CLI guidance.

**Tech Stack:** Rust (edition 2024), serde + toml for config parsing, tokio for async command execution.

**Spec:** `docs/specs/2026-04-06-drone-bash-reliability.md`

---

### Task 1: Add `toml` dependency to `drone-sdk`

**Files:**
- Modify: `src/drone-sdk/Cargo.toml`
- Modify: `src/drone-sdk/BUCK`

- [ ] **Step 1: Add toml crate to Cargo.toml**

In `src/drone-sdk/Cargo.toml`, add to `[dependencies]`:

```toml
toml = "0.8"
```

- [ ] **Step 2: Regenerate third-party BUCK**

Run: `./tools/buckify.sh`
Expected: exits 0, `third-party/BUCK` updated with `toml` crate

- [ ] **Step 3: Add toml dep to drone-sdk BUCK**

In `src/drone-sdk/BUCK`, add `"//third-party:toml"` to `SDK_DEPS`:

```python
SDK_DEPS = [
    "//third-party:anyhow",
    "//third-party:async-trait",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:toml",
    "//third-party:tracing",
]
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src/drone-sdk && cargo check`
Expected: compiles cleanly

- [ ] **Step 5: Commit**

```bash
git add src/drone-sdk/Cargo.toml src/drone-sdk/BUCK Cargo.lock
git commit -m "deps(drone-sdk): add toml crate for drone.toml parsing"
```

---

### Task 2: Define `DroneToml` struct in `drone-sdk`

**Files:**
- Create: `src/drone-sdk/src/drone_toml.rs`
- Modify: `src/drone-sdk/src/lib.rs`

- [ ] **Step 1: Write failing test for DroneToml parsing**

Create `src/drone-sdk/src/drone_toml.rs` with the struct definitions and tests:

```rust
use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Shared drone.toml configuration read from the target repo's workspace root.
/// All fields are optional with sensible defaults. If the file is absent,
/// `DroneToml::default()` applies.
#[derive(Debug, Deserialize, Default)]
pub struct DroneToml {
    #[serde(default)]
    pub git: GitSection,
    #[serde(default)]
    pub setup: SetupSection,
    #[serde(default)]
    pub prompts: PromptsSection,
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
    #[serde(default)]
    pub identity: HashMap<String, IdentitySection>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct IdentitySection {
    pub user_name: String,
    pub user_email: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SetupSection {
    #[serde(default)]
    pub commands: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PromptsSection {
    #[serde(default)]
    pub extra_rules: String,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_prefix() -> String {
    "feat/".to_string()
}

fn default_true() -> bool {
    true
}

impl DroneToml {
    /// Load drone.toml from a workspace directory. Returns `Ok(default)` if
    /// the file doesn't exist. Returns `Err` only on parse failures.
    pub fn load(workspace: &Path) -> anyhow::Result<Self> {
        let path = workspace.join("drone.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&content)?)
    }

    /// Get the git identity for a specific drone type, with fallback defaults.
    pub fn git_identity(&self, drone_type: &str) -> IdentitySection {
        self.git
            .identity
            .get(drone_type)
            .cloned()
            .unwrap_or_else(|| IdentitySection {
                user_name: format!("{drone_type}-drone"),
                user_email: format!("{drone_type}-drone@noreply"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[git]
default_branch = "develop"
branch_prefix = "kerrigan/"
auto_commit = false
pr_on_complete = true
protected_paths = ["README.md"]

[git.identity.claude]
user_name = "claude-bot"
user_email = "claude@myorg.com"

[git.identity.native]
user_name = "native-bot"
user_email = "native@myorg.com"

[setup]
commands = ["./tools/setup-hooks.sh", "npm install"]

[prompts]
extra_rules = """
## Build
Use buck2 build, not cargo build.
"""
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        assert_eq!(config.git.default_branch, "develop");
        assert_eq!(config.git.branch_prefix, "kerrigan/");
        assert!(!config.git.auto_commit);
        assert_eq!(config.git.protected_paths, vec!["README.md"]);

        let claude_id = config.git_identity("claude");
        assert_eq!(claude_id.user_name, "claude-bot");
        assert_eq!(claude_id.user_email, "claude@myorg.com");

        let native_id = config.git_identity("native");
        assert_eq!(native_id.user_name, "native-bot");

        assert_eq!(config.setup.commands.len(), 2);
        assert!(config.prompts.extra_rules.contains("buck2 build"));
    }

    #[test]
    fn parse_minimal_config() {
        let config: DroneToml = toml::from_str("").unwrap();
        assert_eq!(config.git.default_branch, "main");
        assert_eq!(config.git.branch_prefix, "feat/");
        assert!(config.git.auto_commit);
        assert!(config.git.pr_on_complete);
        assert!(config.setup.commands.is_empty());
        assert!(config.prompts.extra_rules.is_empty());
    }

    #[test]
    fn identity_fallback_for_unknown_type() {
        let config: DroneToml = toml::from_str("").unwrap();
        let id = config.git_identity("claude");
        assert_eq!(id.user_name, "claude-drone");
        assert_eq!(id.user_email, "claude-drone@noreply");
    }

    #[test]
    fn identity_with_partial_config() {
        let toml_str = r#"
[git.identity.claude]
user_name = "my-claude"
user_email = "claude@example.com"
"#;
        let config: DroneToml = toml::from_str(toml_str).unwrap();
        let claude = config.git_identity("claude");
        assert_eq!(claude.user_name, "my-claude");

        // native not defined — falls back
        let native = config.git_identity("native");
        assert_eq!(native.user_name, "native-drone");
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = std::path::PathBuf::from("/tmp/nonexistent-workspace-test");
        let config = DroneToml::load(&dir).unwrap();
        assert_eq!(config.git.default_branch, "main");
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("drone.toml"),
            r#"
[git.identity.claude]
user_name = "test-claude"
user_email = "test@example.com"

[setup]
commands = ["echo hello"]
"#,
        )
        .unwrap();

        let config = DroneToml::load(dir.path()).unwrap();
        let id = config.git_identity("claude");
        assert_eq!(id.user_name, "test-claude");
        assert_eq!(config.setup.commands, vec!["echo hello"]);
    }
}
```

- [ ] **Step 2: Add tempfile dev-dependency for tests**

Add `tempfile = "3"` to `src/drone-sdk/Cargo.toml` under `[dependencies]` (drone-sdk doesn't have dev-dependencies since reindeer skips them — add as regular dep):

```toml
tempfile = "3"
```

Run: `./tools/buckify.sh`

Then add `"//third-party:tempfile"` to `SDK_DEPS` in `src/drone-sdk/BUCK`.

- [ ] **Step 3: Register module in lib.rs**

In `src/drone-sdk/src/lib.rs`, add:

```rust
pub mod drone_toml;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/drone-sdk && cargo test drone_toml`
Expected: all 6 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/drone-sdk/src/drone_toml.rs src/drone-sdk/src/lib.rs src/drone-sdk/Cargo.toml src/drone-sdk/BUCK Cargo.lock
git commit -m "feat(drone-sdk): add DroneToml struct for shared drone.toml config"
```

---

### Task 3: Configure git identity in Claude drone setup

**Files:**
- Modify: `src/drones/claude/base/src/environment.rs`
- Modify: `src/drones/claude/base/src/drone.rs`

- [ ] **Step 1: Write failing test for git identity in .gitconfig**

In `src/drones/claude/base/src/environment.rs`, add a test at the bottom of the `mod tests` block:

```rust
#[tokio::test]
async fn test_configure_git_identity() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    // Simulate existing .gitconfig from configure_github_auth
    fs::write(home.join(".gitconfig"), "[credential]\n    helper = store\n")
        .await
        .unwrap();

    configure_git_identity(home, "claude-drone", "claude-drone@noreply")
        .await
        .unwrap();

    let content = fs::read_to_string(home.join(".gitconfig")).await.unwrap();
    assert!(content.contains("[user]"));
    assert!(content.contains("name = claude-drone"));
    assert!(content.contains("email = claude-drone@noreply"));
    // credential section preserved
    assert!(content.contains("[credential]"));
}
```

Also add `tempfile` to `src/drones/claude/base/Cargo.toml` and BUCK deps (same pattern as task 2 — add as regular dep, buckify, add to BUCK).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/drones/claude/base && cargo test test_configure_git_identity`
Expected: FAIL with "cannot find function `configure_git_identity`"

- [ ] **Step 3: Implement `configure_git_identity`**

In `src/drones/claude/base/src/environment.rs`, add:

```rust
/// Write git user identity to .gitconfig.
/// Appends to any existing content (e.g. credential helper already written).
pub async fn configure_git_identity(home: &Path, name: &str, email: &str) -> Result<()> {
    let gitconfig_path = home.join(".gitconfig");
    let existing = fs::read_to_string(&gitconfig_path)
        .await
        .unwrap_or_default();
    let identity = format!("[user]\n    name = {name}\n    email = {email}\n");
    fs::write(&gitconfig_path, format!("{existing}{identity}"))
        .await
        .context("failed to write git identity to .gitconfig")?;
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src/drones/claude/base && cargo test test_configure_git_identity`
Expected: PASS

- [ ] **Step 5: Wire identity into `setup()` in drone.rs**

In `src/drones/claude/base/src/drone.rs`, in the `setup()` method, after the `clone_repo()` call (line 35) and before `write_task()` (line 36), add:

```rust
        // Read drone.toml from workspace and configure git identity
        let drone_toml = drone_sdk::drone_toml::DroneToml::load(&env.workspace)?;
        let identity = drone_toml.git_identity("claude");
        environment::configure_git_identity(&env.home, &identity.user_name, &identity.user_email)
            .await?;
        tracing::info!(
            user_name = %identity.user_name,
            user_email = %identity.user_email,
            "configured git identity from drone.toml"
        );
```

- [ ] **Step 6: Run full test suite**

Run: `cd src/drones/claude/base && cargo test`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/drones/claude/base/src/environment.rs src/drones/claude/base/src/drone.rs src/drones/claude/base/Cargo.toml src/drones/claude/base/BUCK Cargo.lock
git commit -m "feat(claude-drone): configure git identity from drone.toml during setup"
```

---

### Task 4: Run setup commands from `drone.toml`

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`

- [ ] **Step 1: Write failing test for setup command execution**

In `src/drones/claude/base/src/drone.rs`, add at the bottom of the file (outside the impl block, in a `#[cfg(test)] mod tests` block):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_run_setup_commands() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path();
        let home = dir.path();

        // Create a script that writes a marker file
        let script = workspace.join("setup.sh");
        tokio::fs::write(&script, "#!/bin/sh\ntouch marker.txt\n")
            .await
            .unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        let commands = vec!["./setup.sh".to_string()];
        run_setup_commands(&commands, workspace, home).await;

        assert!(workspace.join("marker.txt").exists());
    }

    #[tokio::test]
    async fn test_run_setup_commands_failure_continues() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path();
        let home = dir.path();

        // First command fails, second should still run
        let script = workspace.join("second.sh");
        tokio::fs::write(&script, "#!/bin/sh\ntouch second_ran.txt\n")
            .await
            .unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        let commands = vec![
            "false".to_string(), // exits non-zero
            "./second.sh".to_string(),
        ];
        run_setup_commands(&commands, workspace, home).await;

        assert!(workspace.join("second_ran.txt").exists());
    }
}
```

Add the import at the top of the test module: `use std::os::unix::fs::PermissionsExt;`

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/drones/claude/base && cargo test test_run_setup_commands`
Expected: FAIL with "cannot find function `run_setup_commands`"

- [ ] **Step 3: Implement `run_setup_commands`**

In `src/drones/claude/base/src/drone.rs`, add this function (before the `impl DroneRunner for ClaudeDrone` block):

```rust
use std::path::PathBuf;

/// Run setup commands from drone.toml sequentially.
/// Non-zero exits are logged as warnings but do not abort.
/// Each command gets a 5-minute timeout.
async fn run_setup_commands(commands: &[String], workspace: &Path, home: &Path) {
    for (i, cmd) in commands.iter().enumerate() {
        tracing::info!(command = %cmd, index = i, "running setup command");
        let result = timeout(
            Duration::from_secs(300),
            Command::new("sh")
                .args(["-c", cmd])
                .env("HOME", home)
                .current_dir(workspace)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stdout.is_empty() {
                    tracing::info!(stdout = %stdout.trim(), "setup command output");
                }
                if !output.status.success() {
                    tracing::warn!(
                        command = %cmd,
                        exit_code = output.status.code(),
                        stderr = %stderr.trim(),
                        "setup command failed, continuing"
                    );
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(command = %cmd, error = %e, "setup command spawn failed, continuing");
            }
            Err(_) => {
                tracing::warn!(command = %cmd, "setup command timed out after 5 minutes, continuing");
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/drones/claude/base && cargo test test_run_setup_commands`
Expected: both tests pass

- [ ] **Step 5: Wire into `setup()` in drone.rs**

In the `setup()` method, after the `configure_git_identity` block added in Task 3 and before `write_task()`, add:

```rust
        // Run post-clone setup commands from drone.toml
        if !drone_toml.setup.commands.is_empty() {
            run_setup_commands(&drone_toml.setup.commands, &env.workspace, &env.home).await;
        }
```

- [ ] **Step 6: Run full test suite**

Run: `cd src/drones/claude/base && cargo test`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/drones/claude/base/src/drone.rs
git commit -m "feat(claude-drone): run setup commands from drone.toml after clone"
```

---

### Task 5: Inject repo-specific prompts into CLAUDE.md

**Files:**
- Modify: `src/drones/claude/base/src/stages.rs`
- Modify: `src/drones/claude/base/src/drone.rs`

- [ ] **Step 1: Write failing test for extra_rules injection**

In `src/drones/claude/base/src/stages.rs`, add to the existing `mod tests` block:

```rust
    #[test]
    fn test_extra_rules_appended_to_stage_md() {
        let config = json!({"plan_path": "docs/plans/test.md"});
        let extra = "## Build\nUse buck2 build, not cargo build.";
        let md = generate_claude_md_with_extra("plan", &config, extra);
        assert!(md.is_some());
        let content = md.unwrap();
        // Extra rules appear between stage content and base rules
        let extra_pos = content.find("## Build").unwrap();
        let rules_pos = content.find("## Rules").unwrap();
        assert!(extra_pos < rules_pos, "extra_rules should appear before base rules");
    }

    #[test]
    fn test_extra_rules_in_review_stage() {
        let config = json!({"pr_url": "https://github.com/org/repo/pull/42"});
        let extra = "## Build\nUse buck2.";
        let md = generate_claude_md_with_extra("review", &config, extra);
        assert!(md.is_some());
        let content = md.unwrap();
        // Review stage has "## Git Workflow" instead of "## Rules"
        let extra_pos = content.find("## Build").unwrap();
        let workflow_pos = content.find("## Git Workflow").unwrap();
        assert!(extra_pos < workflow_pos, "extra_rules should appear before Git Workflow in review");
    }

    #[test]
    fn test_empty_extra_rules_no_change() {
        let config = json!({"plan_path": "docs/plans/test.md"});
        let with_extra = generate_claude_md_with_extra("plan", &config, "");
        let without = generate_claude_md("plan", &config);
        assert_eq!(with_extra, without);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/drones/claude/base && cargo test test_extra_rules`
Expected: FAIL with "cannot find function `generate_claude_md_with_extra`"

- [ ] **Step 3: Implement `generate_claude_md_with_extra`**

In `src/drones/claude/base/src/stages.rs`, add:

```rust
/// Generate stage-specific CLAUDE.md with optional repo-specific extra rules
/// injected between the stage content and the base rules.
pub fn generate_claude_md_with_extra(stage: &str, config: &Value, extra_rules: &str) -> Option<String> {
    let base = generate_claude_md(stage, config)?;
    if extra_rules.is_empty() {
        return Some(base);
    }
    // Insert extra_rules before the rules/workflow section.
    // Stages using BASE_RULES have "## Rules" first. The review stage
    // has "## Git Workflow" inline. Find whichever comes first.
    let marker = base.find("## Rules\n")
        .into_iter()
        .chain(base.find("## Git Workflow\n"))
        .min();
    if let Some(pos) = marker {
        let mut result = String::with_capacity(base.len() + extra_rules.len() + 2);
        result.push_str(&base[..pos]);
        result.push_str(extra_rules);
        result.push_str("\n\n");
        result.push_str(&base[pos..]);
        Some(result)
    } else {
        // No BASE_RULES found (e.g. evolve stage) — append at end
        Some(format!("{base}\n\n{extra_rules}"))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src/drones/claude/base && cargo test test_extra_rules`
Expected: both tests pass

- [ ] **Step 5: Wire into drone.rs setup()**

In `src/drones/claude/base/src/drone.rs`, modify the stage-specific CLAUDE.md generation block (currently around line 39-49). Change:

```rust
        if let Some(stage) = job.config.get("stage").and_then(|v| v.as_str()) {
            if let Some(claude_md) = crate::stages::generate_claude_md(stage, &job.config) {
```

To:

```rust
        if let Some(stage) = job.config.get("stage").and_then(|v| v.as_str()) {
            if let Some(claude_md) = crate::stages::generate_claude_md_with_extra(
                stage,
                &job.config,
                &drone_toml.prompts.extra_rules,
            ) {
```

- [ ] **Step 6: Run full test suite**

Run: `cd src/drones/claude/base && cargo test`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add src/drones/claude/base/src/stages.rs src/drones/claude/base/src/drone.rs
git commit -m "feat(claude-drone): inject repo-specific prompts from drone.toml into CLAUDE.md"
```

---

### Task 6: Update base CLAUDE.md with pre-commit, push, and CLI guidance

**Files:**
- Modify: `src/drones/claude/base/src/config/CLAUDE.md`
- Modify: `src/drones/claude/base/src/stages.rs` (update `BASE_RULES` constant)

- [ ] **Step 1: Update the embedded `config/CLAUDE.md`**

Replace the full content of `src/drones/claude/base/src/config/CLAUDE.md` with:

```markdown
# Claude Drone

You are a Claude Code drone operating within the Kerrigan agentic platform. You execute tasks assigned by the Queen process manager.

## Git Workflow

You MUST follow this git workflow for every task:

1. Create a new branch from the current HEAD with a descriptive name
2. Make your changes, committing frequently with clear messages
3. ALWAYS run `git push -u origin HEAD` before `gh pr create`. The PR command will fail if you haven't pushed.
4. Create a pull request with:
   - A clear title summarizing the change
   - A description explaining what was done and why
   - A test plan section

Do NOT merge the PR. The operator will review and merge.

## Pre-commit Hooks

This repo may use pre-commit hooks that auto-fix files (trailing whitespace,
end-of-file newlines, formatting). When a commit fails because hooks modified files:

1. Run `git add -u` to re-stage the modified files
2. Run `git commit` again with the same message
3. Do NOT use `--no-verify` to skip hooks

## CLI Usage

When unsure about a command's flags or arguments, run `<command> --help` first
rather than guessing. Common mistakes to avoid:
- `gh pr diff` has no `--stat` flag
- `cargo test` accepts only ONE test name filter as a positional argument

## Rules

- Focus exclusively on the assigned task
- Do not modify files outside the scope of the task
- Commit work frequently with descriptive messages
- If you encounter a blocker, document it clearly in your output
- Do not install system packages or modify system configuration
```

- [ ] **Step 2: Update `BASE_RULES` in stages.rs**

Replace the `BASE_RULES` constant in `src/drones/claude/base/src/stages.rs` with:

```rust
const BASE_RULES: &str = r#"## Rules

- Focus exclusively on the assigned task
- Do not modify files outside the scope of the task
- Commit work frequently with descriptive messages
- If you encounter a blocker, document it clearly in your output
- Do not install system packages or modify system configuration

## Git Workflow

You MUST follow this git workflow:

1. Create a new branch from the current HEAD with a descriptive name
2. Make your changes, committing frequently with clear messages
3. ALWAYS run `git push -u origin HEAD` before `gh pr create`. The PR command will fail if you haven't pushed.
4. Create a pull request to main with a clear title, description, and test plan

Do NOT merge the PR. The operator will review and merge.

## Pre-commit Hooks

This repo may use pre-commit hooks that auto-fix files (trailing whitespace,
end-of-file newlines, formatting). When a commit fails because hooks modified files:

1. Run `git add -u` to re-stage the modified files
2. Run `git commit` again with the same message
3. Do NOT use `--no-verify` to skip hooks

## CLI Usage

When unsure about a command's flags or arguments, run `<command> --help` first
rather than guessing. Common mistakes to avoid:
- `gh pr diff` has no `--stat` flag
- `cargo test` accepts only ONE test name filter as a positional argument

## Artifacts

When you produce a key output (spec, plan, review), store it as an Overseer artifact
using the Overseer MCP tools available to you (if configured). This ensures traceability
alongside the git commit."#;
```

- [ ] **Step 3: Run existing stage tests**

Run: `cd src/drones/claude/base && cargo test stages::tests`
Expected: all tests pass. If any test asserts exact string content that changed, update the assertion.

- [ ] **Step 4: Commit**

```bash
git add src/drones/claude/base/src/config/CLAUDE.md src/drones/claude/base/src/stages.rs
git commit -m "feat(claude-drone): add pre-commit hook, push-before-PR, and CLI guidance to base instructions"
```

---

### Task 7: Create `drone.toml` for the kerrigan repo

**Files:**
- Create: `drone.toml` (repo root)

- [ ] **Step 1: Create drone.toml**

Create `drone.toml` in the repo root:

```toml
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

- Use `buck2 build root//src/<crate>:<crate>` to build, NOT `cargo build`
- Use `buck2 test root//src/<crate>:<crate>-test` to test, NOT `cargo test`
- `cargo check` and `cargo clippy` are OK for quick feedback
- Clippy CI-equivalent: `buck2 build 'root//src/<crate>:<crate>[clippy.txt]'`
- Run `buck2 targets root//...` to discover available targets
"""
```

- [ ] **Step 2: Verify it parses**

Run a quick parse check:

```bash
cd src/drone-sdk && cargo test drone_toml::tests::load_from_file
```

Expected: passes (tests already written in Task 2 cover parsing)

Also manually verify the repo root file parses:

```bash
python3 -c "import tomllib; print(tomllib.load(open('drone.toml', 'rb')))"
```

Expected: prints the parsed dict without errors

- [ ] **Step 3: Commit**

```bash
git add drone.toml
git commit -m "feat: add drone.toml with kerrigan-specific drone configuration"
```

---

### Task 8: Verify full build and test

**Files:** None (verification only)

- [ ] **Step 1: Run drone-sdk tests**

Run: `cd src/drone-sdk && cargo test`
Expected: all tests pass

- [ ] **Step 2: Run claude-drone tests**

Run: `cd src/drones/claude/base && cargo test`
Expected: all tests pass

- [ ] **Step 3: Run buck2 build**

Run: `buck2 build root//src/drone-sdk:drone-sdk root//src/drones/claude/base:claude-drone`
Expected: builds successfully

- [ ] **Step 4: Run buck2 tests**

Run: `buck2 test root//src/drone-sdk:drone-sdk-test root//src/drones/claude/base:claude-drone-test`
Expected: all tests pass

- [ ] **Step 5: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all checks pass (fmt, clippy, tests)
