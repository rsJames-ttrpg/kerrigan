# Drone PR Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make drone jobs produce PRs — Claude Code branches, commits, pushes, and creates a PR; the drone verifies; Queen enforces and stores compressed artifacts.

**Architecture:** Four independent changes: (1) drone CLAUDE.md + settings.json with PR instructions and Overseer MCP config, (2) drone secrets setup from job config, (3) drone post-execute PR verification safety net, (4) Queen gzip compression of conversation artifacts + PR URL enforcement.

**Tech Stack:** Rust 2024, flate2 (gzip), serde_json, tokio

**Spec:** `docs/specs/2026-04-01-drone-pr-workflow-design.md`

---

## File Structure

### Modified files

| File | Change |
|------|--------|
| `src/drones/claude/base/src/config/CLAUDE.md` | Full PR workflow instructions for Claude Code |
| `src/drones/claude/base/src/config/settings.json` | Overseer MCP server config (URL is a placeholder, rewritten at runtime) |
| `src/drones/claude/base/src/environment.rs` | Secrets setup: gh auth, git credentials, env vars |
| `src/drones/claude/base/src/drone.rs` | Post-execute PR verification safety net |
| `src/queen/src/actors/supervisor.rs` | Gzip conversation artifacts, PR URL enforcement |
| `src/queen/Cargo.toml` | Add `flate2` dependency |
| `src/queen/BUCK` | Add `//third-party:flate2` to deps |

### No new files

All changes are modifications to existing files.

---

## Task 1: Update drone CLAUDE.md with PR workflow instructions

**Files:**
- Modify: `src/drones/claude/base/src/config/CLAUDE.md`

- [ ] **Step 1: Write the updated CLAUDE.md**

Replace the contents of `src/drones/claude/base/src/config/CLAUDE.md`:

```markdown
# Claude Drone

You are a Claude Code drone operating within the Kerrigan agentic platform. You execute tasks assigned by the Queen process manager.

## Git Workflow

You MUST follow this git workflow for every task:

1. Create a new branch from the current HEAD with a descriptive name
2. Make your changes, committing frequently with clear messages
3. Push the branch to origin
4. Create a pull request with:
   - A clear title summarizing the change
   - A description explaining what was done and why
   - A test plan section

Do NOT merge the PR. The operator will review and merge.

## Rules

- Focus exclusively on the assigned task
- Do not modify files outside the scope of the task
- Commit work frequently with descriptive messages
- If you encounter a blocker, document it clearly in your output
- Do not install system packages or modify system configuration
```

- [ ] **Step 2: Verify the drone crate still compiles**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles (CLAUDE.md is embedded via `include_bytes!`, content doesn't affect compilation)

Note: This will fail if the `claude-cli` binary isn't present. That's a pre-existing issue — the binary is fetched by Buck2 at build time. If cargo check fails on the missing file, verify the CLAUDE.md change is correct by inspecting it, and move on.

- [ ] **Step 3: Commit**

```bash
git add src/drones/claude/base/src/config/CLAUDE.md
git commit -m "feat(drone): add PR workflow instructions to CLAUDE.md"
```

---

## Task 2: Configure Overseer MCP in drone settings.json

**Files:**
- Modify: `src/drones/claude/base/src/config/settings.json`
- Modify: `src/drones/claude/base/src/environment.rs`

The settings.json embeds a placeholder MCP URL. At runtime, `environment.rs` rewrites it with the actual Overseer URL from the job config.

- [ ] **Step 1: Update settings.json with MCP config**

Replace the contents of `src/drones/claude/base/src/config/settings.json`:

```json
{
  "permissions": {
    "allow": [],
    "deny": []
  },
  "model": "sonnet",
  "mcpServers": {
    "overseer": {
      "type": "url",
      "url": "OVERSEER_MCP_URL_PLACEHOLDER"
    }
  }
}
```

- [ ] **Step 2: Add MCP URL rewriting to environment.rs**

In `src/drones/claude/base/src/environment.rs`, after writing `settings.json` (line 43-45), add a function to rewrite the placeholder URL. Add this public function:

```rust
/// Rewrite the Overseer MCP URL placeholder in settings.json with the actual URL.
pub async fn configure_mcp_url(home: &Path, overseer_url: &str) -> Result<()> {
    let settings_path = home.join(".claude/settings.json");
    let content = fs::read_to_string(&settings_path)
        .await
        .context("failed to read settings.json for MCP URL rewrite")?;
    // The MCP endpoint for streamable HTTP is at /mcp on the Overseer URL
    let mcp_url = format!("{}/mcp", overseer_url.trim_end_matches('/'));
    let updated = content.replace("OVERSEER_MCP_URL_PLACEHOLDER", &mcp_url);
    fs::write(&settings_path, updated)
        .await
        .context("failed to write updated settings.json")?;
    Ok(())
}
```

- [ ] **Step 3: Call configure_mcp_url from drone setup**

In `src/drones/claude/base/src/drone.rs`, in the `setup` method (lines 19-23), after `environment::write_task`, add the MCP URL configuration. The Overseer URL is read from `job.config`:

```rust
    async fn setup(&self, job: &JobSpec) -> Result<DroneEnvironment> {
        let env = environment::create_home(&job.job_run_id).await?;
        environment::clone_repo(&job.repo_url, job.branch.as_deref(), &env.workspace).await?;
        environment::write_task(&env.home, &job.task).await?;

        // Configure Overseer MCP URL if provided in job config
        if let Some(overseer_url) = job.config.get("overseer_url").and_then(|v| v.as_str()) {
            environment::configure_mcp_url(&env.home, overseer_url).await?;
        }

        Ok(env)
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles (or fails only on missing claude-cli, not on our changes)

- [ ] **Step 5: Commit**

```bash
git add src/drones/claude/base/src/config/settings.json src/drones/claude/base/src/environment.rs src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): configure Overseer MCP server in settings.json"
```

---

## Task 3: Secrets setup from job config

**Files:**
- Modify: `src/drones/claude/base/src/environment.rs`
- Modify: `src/drones/claude/base/src/drone.rs`

The drone reads `config.secrets.github_pat` and `config.secrets.buildbuddy_api_key` from the job config and sets up the environment.

- [ ] **Step 1: Add secrets setup function to environment.rs**

Add to `src/drones/claude/base/src/environment.rs`:

```rust
/// Configure GitHub authentication from a PAT token.
///
/// Sets up:
/// - `~/.config/gh/hosts.yml` for `gh` CLI
/// - Git credential helper for HTTPS push
pub async fn configure_github_auth(home: &Path, pat: &str) -> Result<()> {
    // gh CLI config
    let gh_config_dir = home.join(".config/gh");
    fs::create_dir_all(&gh_config_dir)
        .await
        .context("failed to create gh config dir")?;
    let hosts_yml = format!(
        "github.com:\n    oauth_token: {pat}\n    user: kerrigan-drone\n    git_protocol: https\n"
    );
    fs::write(gh_config_dir.join("hosts.yml"), hosts_yml)
        .await
        .context("failed to write gh hosts.yml")?;

    // Git credential helper — store the PAT for HTTPS operations
    let git_credentials = format!("https://kerrigan-drone:{pat}@github.com\n");
    let creds_file = home.join(".git-credentials");
    fs::write(&creds_file, git_credentials)
        .await
        .context("failed to write .git-credentials")?;

    // Configure git to use the credential store
    let gitconfig = "[credential]\n    helper = store\n";
    let gitconfig_path = home.join(".gitconfig");
    // Append to existing .gitconfig if present, or create new
    let existing = fs::read_to_string(&gitconfig_path).await.unwrap_or_default();
    fs::write(&gitconfig_path, format!("{existing}{gitconfig}"))
        .await
        .context("failed to write .gitconfig")?;

    Ok(())
}
```

- [ ] **Step 2: Call secrets setup from drone setup**

In `src/drones/claude/base/src/drone.rs`, extend the `setup` method to handle secrets. After the MCP URL configuration added in Task 2:

```rust
        // Configure secrets from job config
        if let Some(secrets) = job.config.get("secrets") {
            if let Some(pat) = secrets.get("github_pat").and_then(|v| v.as_str()) {
                environment::configure_github_auth(&env.home, pat).await?;
            }
        }
```

- [ ] **Step 3: Pass BuildBuddy API key as environment variable**

The BuildBuddy key needs to be set as an environment variable when spawning Claude Code. In `src/drones/claude/base/src/drone.rs`, in the `execute` method, before spawning the `claude` CLI process (line 46), extract the key and pass it:

Change the Command builder to conditionally add the env var. After `.env("HOME", &env.home)` (line 55), add:

```rust
        // Read BuildBuddy API key from job config and set env var
        // We need to read the task file's parent config — but we don't have job config here.
        // Instead, store the env vars during setup and read them back.
```

Actually, `execute()` doesn't have access to `JobSpec`. The cleanest fix: during `setup()`, write a small env file that `execute()` reads. Add to `environment.rs`:

```rust
/// Write environment variables to a file that the drone reads during execute.
pub async fn write_env_vars(home: &Path, vars: &[(String, String)]) -> Result<()> {
    let content = vars
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(home.join(".drone-env"), content)
        .await
        .context("failed to write .drone-env")?;
    Ok(())
}

/// Read environment variables from the .drone-env file.
pub async fn read_env_vars(home: &Path) -> Result<Vec<(String, String)>> {
    let path = home.join(".drone-env");
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = fs::read_to_string(&path)
        .await
        .context("failed to read .drone-env")?;
    Ok(content
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect())
}
```

In `setup()` in `drone.rs`, collect env vars and write them:

```rust
        // Collect environment variables for the drone session
        let mut env_vars = Vec::new();
        if let Some(secrets) = job.config.get("secrets") {
            if let Some(bb_key) = secrets.get("buildbuddy_api_key").and_then(|v| v.as_str()) {
                env_vars.push((
                    "BUCK2_RE_HTTP_HEADERS".to_string(),
                    format!("x-buildbuddy-api-key:{bb_key}"),
                ));
            }
        }
        if !env_vars.is_empty() {
            environment::write_env_vars(&env.home, &env_vars).await?;
        }
```

In `execute()` in `drone.rs`, read env vars and apply them to the Command. Before the `let mut child = Command::new(...)` block:

```rust
        let extra_env = environment::read_env_vars(&env.home).await?;
```

Then in the Command builder, after `.env("HOME", &env.home)`:

```rust
        let mut cmd = Command::new(&claude_bin);
        cmd.arg("--print")
            .arg("--output-format")
            .arg("json")
            .arg("--dangerously-skip-permissions")
            .arg("--settings")
            .arg(&settings_path)
            .arg("--append-system-prompt-file")
            .arg(&claude_md_path)
            .env("HOME", &env.home)
            .current_dir(&env.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in &extra_env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn()
            .context("failed to spawn claude CLI")?;
```

- [ ] **Step 4: Verify compilation**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles (or fails only on missing claude-cli)

- [ ] **Step 5: Commit**

```bash
git add src/drones/claude/base/
git commit -m "feat(drone): configure GitHub auth and BuildBuddy from job config secrets"
```

---

## Task 4: Drone post-execute PR verification safety net

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`

After Claude Code exits, the drone verifies that a PR was created. If not, it attempts to create one as a fallback.

- [ ] **Step 1: Add ensure_pr function to drone.rs**

Add this function to `src/drones/claude/base/src/drone.rs`, after the existing `collect_git_refs` function:

```rust
/// Safety net: ensure uncommitted changes are pushed and a PR exists.
/// Returns updated GitRefs.
async fn ensure_pr(workspace: &Path, task: &str) -> GitRefs {
    // 1. Check for uncommitted changes
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workspace)
        .output()
        .await;

    if let Ok(output) = &status_output {
        let status_text = String::from_utf8_lossy(&output.stdout);
        if !status_text.trim().is_empty() {
            tracing::warn!("uncommitted changes found after Claude Code exit, committing");
            let _ = Command::new("git")
                .args(["add", "-A"])
                .current_dir(workspace)
                .output()
                .await;
            let _ = Command::new("git")
                .args(["commit", "-m", "chore: commit remaining changes from drone session"])
                .current_dir(workspace)
                .output()
                .await;
        }
    }

    // 2. Check if we're on a non-default branch (something to push)
    let refs = collect_git_refs(workspace).await;
    let branch = match &refs.branch {
        Some(b) if b != "main" && b != "master" => b.clone(),
        _ => {
            tracing::warn!("drone ended on default branch, no PR possible");
            return refs;
        }
    };

    // 3. Push if not already pushed
    let push_output = Command::new("git")
        .args(["push", "-u", "origin", &branch])
        .current_dir(workspace)
        .output()
        .await;
    if let Ok(output) = &push_output {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(stderr = %stderr, "git push failed in safety net");
        }
    }

    // 4. Check if PR exists
    if refs.pr_url.is_some() {
        return refs;
    }

    // 5. Create PR as fallback
    tracing::warn!("no PR found after Claude Code exit, creating one as fallback");
    let title = if task.len() > 60 {
        format!("{}...", &task[..57])
    } else {
        task.to_string()
    };
    let pr_output = Command::new("gh")
        .args([
            "pr", "create",
            "--title", &title,
            "--body", "Automated PR created by Kerrigan drone.\n\nClaude Code did not create a PR during its session. This PR was created as a safety net.",
        ])
        .current_dir(workspace)
        .output()
        .await;

    match pr_output {
        Ok(output) if output.status.success() => {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            tracing::info!(pr_url = %url, "safety net PR created");
            GitRefs {
                branch: Some(branch),
                pr_url: if url.is_empty() { None } else { Some(url) },
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!(stderr = %stderr, "failed to create safety net PR");
            GitRefs {
                branch: Some(branch),
                pr_url: None,
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "gh CLI not available for safety net PR");
            GitRefs {
                branch: Some(branch),
                pr_url: None,
            }
        }
    }
}
```

- [ ] **Step 2: Replace collect_git_refs call with ensure_pr in execute()**

In `src/drones/claude/base/src/drone.rs`, in the `execute` method, find the line (approximately 159):

```rust
                        let git_refs = collect_git_refs(&env.workspace).await;
```

Replace with:

```rust
                        let task_text = task.chars().take(200).collect::<String>();
                        let git_refs = ensure_pr(&env.workspace, &task_text).await;
```

Note: `task` is the variable already defined at the top of `execute()` (line 31).

- [ ] **Step 3: Verify compilation**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles (or fails only on missing claude-cli)

- [ ] **Step 4: Commit**

```bash
git add src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): add post-execute PR verification safety net"
```

---

## Task 5: Queen — gzip conversation artifacts

**Files:**
- Modify: `src/queen/Cargo.toml`
- Modify: `src/queen/BUCK`
- Modify: `src/queen/src/actors/supervisor.rs`

- [ ] **Step 1: Add flate2 dependency**

In `src/queen/Cargo.toml`, add under `[dependencies]`:

```toml
flate2 = "1"
```

In `src/queen/BUCK`, add to `QUEEN_DEPS`:

```starlark
    "//third-party:flate2",
```

- [ ] **Step 2: Run buckify**

Run: `./tools/buckify.sh`
Expected: completes, third-party/BUCK regenerated

- [ ] **Step 3: Add gzip helper function to supervisor.rs**

At the top of `src/queen/src/actors/supervisor.rs`, add the import:

```rust
use flate2::Compression;
use flate2::write::GzEncoder;
use std::io::Write;
```

Add a helper function near the top of the file (after the imports, before the `run` function):

```rust
fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("gzip write failed");
    encoder.finish().expect("gzip finish failed")
}
```

- [ ] **Step 4: Update artifact storage in drain_protocol_messages**

In `src/queen/src/actors/supervisor.rs`, in the `drain_protocol_messages` function, find the conversation artifact storage block (approximately lines 331-348):

```rust
                            // Store conversation as artifact
                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            let artifact_name = format!("{id}-conversation.json");
                            if let Err(e) = client
                                .store_artifact(
                                    &artifact_name,
                                    "application/json",
                                    &conversation_bytes,
                                    Some(id),
                                )
```

Replace with:

```rust
                            // Store conversation as gzipped artifact
                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            let compressed = gzip_bytes(&conversation_bytes);
                            let artifact_name = format!("{id}-conversation.jsonl.gz");
                            if let Err(e) = client
                                .store_artifact(
                                    &artifact_name,
                                    "application/gzip",
                                    &compressed,
                                    Some(id),
                                )
```

- [ ] **Step 5: Update artifact storage in check_drones**

In the same file, find the second conversation artifact storage block in `check_drones` (approximately lines 439-451):

```rust
                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            if let Err(e) = client
                                .store_artifact(
                                    &format!("{id}-conversation.json"),
                                    "application/json",
                                    &conversation_bytes,
                                    Some(id),
                                )
```

Replace with:

```rust
                            let conversation_bytes =
                                serde_json::to_vec_pretty(&output.conversation).unwrap_or_default();
                            let compressed = gzip_bytes(&conversation_bytes);
                            if let Err(e) = client
                                .store_artifact(
                                    &format!("{id}-conversation.jsonl.gz"),
                                    "application/gzip",
                                    &compressed,
                                    Some(id),
                                )
```

- [ ] **Step 6: Verify compilation and tests**

Run: `cd src/queen && cargo check && cargo test`
Expected: compiles, all 12 tests pass

- [ ] **Step 7: Commit**

```bash
git add src/queen/ third-party/BUCK
git commit -m "feat(queen): gzip conversation artifacts before storing"
```

---

## Task 6: Queen — PR URL enforcement

**Files:**
- Modify: `src/queen/src/actors/supervisor.rs`

When a drone reports `exit_code == 0` but `git_refs.pr_url` is `None`, Queen should mark the run as failed.

- [ ] **Step 1: Add PR URL check in drain_protocol_messages**

In `src/queen/src/actors/supervisor.rs`, in `drain_protocol_messages`, after storing the conversation artifact and before updating the job run status (approximately line 352), change the status determination logic:

Find:

```rust
                            // Update job run status
                            let status = if output.exit_code == 0 {
                                "completed"
                            } else {
                                "failed"
                            };
                            let result_value = serde_json::to_value(&output).ok();
                            if let Err(e) = client
                                .update_run(id, Some(status), result_value, None)
```

Replace with:

```rust
                            // Update job run status — require PR URL for success
                            let (status, error) = if output.exit_code == 0 && output.git_refs.pr_url.is_none() {
                                ("failed", Some("drone completed but no PR was created"))
                            } else if output.exit_code == 0 {
                                ("completed", None)
                            } else {
                                ("failed", None)
                            };
                            let result_value = serde_json::to_value(&output).ok();
                            if let Err(e) = client
                                .update_run(id, Some(status), result_value, error)
```

- [ ] **Step 2: Add same check in check_drones**

In the `check_drones` function, find the similar status determination block (approximately lines 454-463):

Find:

```rust
                            let result_value = serde_json::to_value(&output).ok();
                            let run_status = if output.exit_code == 0 {
                                "completed"
                            } else {
                                "failed"
                            };
                            let error = if output.exit_code != 0 {
                                Some(format!("drone exited with code {}", output.exit_code))
                            } else {
                                None
                            };
```

Replace with:

```rust
                            let result_value = serde_json::to_value(&output).ok();
                            let (run_status, error) = if output.exit_code == 0 && output.git_refs.pr_url.is_none() {
                                ("failed", Some("drone completed but no PR was created".to_string()))
                            } else if output.exit_code == 0 {
                                ("completed", None)
                            } else {
                                ("failed", Some(format!("drone exited with code {}", output.exit_code)))
                            };
```

- [ ] **Step 3: Update notifier calls to match**

In `drain_protocol_messages`, the notifier calls after the status update (approximately lines 366-380) check `exit_code`. These should now also account for the PR check. Find:

```rust
                            let exit_code = output.exit_code;
                            if exit_code == 0 {
                                notifier
                                    .notify(QueenEvent::DroneCompleted {
```

Replace with:

```rust
                            if status == "completed" {
                                notifier
                                    .notify(QueenEvent::DroneCompleted {
                                        job_run_id: id.clone(),
                                        exit_code: output.exit_code,
                                    })
                                    .await;
                            } else {
                                let error_msg = error.map(|s| s.to_string()).unwrap_or_else(|| format!("exit code {}", output.exit_code));
                                notifier
                                    .notify(QueenEvent::DroneFailed {
                                        job_run_id: id.clone(),
                                        error: error_msg,
                                    })
                                    .await;
                            }
```

And remove the old `else` block that follows.

Similarly in `check_drones`, update the notifier call to use `run_status` instead of `output.exit_code`:

```rust
                            if run_status == "completed" {
                                notifier
                                    .notify(QueenEvent::DroneCompleted {
                                        job_run_id: id.clone(),
                                        exit_code: output.exit_code,
                                    })
                                    .await;
                            } else {
                                notifier
                                    .notify(QueenEvent::DroneFailed {
                                        job_run_id: id.clone(),
                                        error: error.clone().unwrap_or_else(|| format!("exit code {}", output.exit_code)),
                                    })
                                    .await;
                            }
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cd src/queen && cargo check && cargo test`
Expected: compiles, all 12 tests pass

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/actors/supervisor.rs
git commit -m "feat(queen): enforce PR URL requirement — fail runs without a PR"
```

---

## Task 7: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Cargo check all affected crates**

Run: `cargo check -p queen -p overseer -p drone-sdk`
Expected: all compile

Note: `claude-drone` may not compile with cargo check due to the embedded claude-cli binary (fetched by Buck2). This is a pre-existing issue.

- [ ] **Step 2: Run tests**

Run: `cargo test -p queen -p overseer -p drone-sdk`
Expected: all tests pass

- [ ] **Step 3: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass

- [ ] **Step 4: Verify CLAUDE.md content**

Run: `cat src/drones/claude/base/src/config/CLAUDE.md`
Expected: contains PR workflow instructions (branch, commit, push, create PR)

- [ ] **Step 5: Verify settings.json content**

Run: `cat src/drones/claude/base/src/config/settings.json`
Expected: contains `mcpServers.overseer` with placeholder URL
