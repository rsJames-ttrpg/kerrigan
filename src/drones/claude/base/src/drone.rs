use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use drone_sdk::harness::QueenChannel;
use drone_sdk::protocol::{DroneEnvironment, DroneOutput, GitRefs, JobSpec};
use drone_sdk::runner::DroneRunner;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::environment;

pub struct ClaudeDrone;

#[async_trait]
impl DroneRunner for ClaudeDrone {
    async fn setup(&self, job: &JobSpec) -> Result<DroneEnvironment> {
        let env = environment::create_home(&job.job_run_id).await?;
        environment::clone_repo(&job.repo_url, job.branch.as_deref(), &env.workspace).await?;
        environment::write_task(&env.home, &job.task).await?;

        // Configure Overseer MCP URL if provided in job config
        if let Some(overseer_url) = job.config.get("overseer_url").and_then(|v| v.as_str()) {
            environment::configure_mcp_url(&env.home, overseer_url).await?;
        }

        // Configure secrets from job config
        if let Some(secrets) = job.config.get("secrets") {
            if let Some(pat) = secrets.get("github_pat").and_then(|v| v.as_str()) {
                environment::configure_github_auth(&env.home, pat).await?;
            }
        }

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

        Ok(env)
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> Result<DroneOutput> {
        let task = fs::read_to_string(env.home.join(".task"))
            .await
            .context("failed to read .task file")?;

        let settings_path = env.home.join(".claude/settings.json");
        let claude_md_path = env.home.join("CLAUDE.md");

        let claude_bin = env.home.join(".claude/bin/claude");

        // Authenticate if no credentials exist
        let creds_path = env.home.join(".claude/.credentials.json");
        if !creds_path.exists() {
            tracing::info!("no credentials found, running claude auth login");
            authenticate(&claude_bin, env, channel).await?;
        }
        let extra_env = environment::read_env_vars(&env.home).await?;

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

        let mut child = cmd.spawn().context("failed to spawn claude CLI")?;

        // Write task to stdin, then close it
        {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("failed to acquire stdin pipe"))?;
            stdin
                .write_all(task.as_bytes())
                .await
                .context("failed to write task to claude stdin")?;
        }

        // Stream stderr in a background task to detect auth URLs
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to acquire stderr pipe"))?;
        let (auth_tx, mut auth_rx) = tokio::sync::mpsc::channel::<String>(4);
        let stderr_handle = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(stderr_line = %line, "claude CLI stderr");
                if line.contains("claude.ai/")
                    || line.contains("claude.com/")
                    || line.contains("console.anthropic.com/")
                {
                    if let Some(url) = extract_url(&line) {
                        let _ = auth_tx.send(url).await;
                    }
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        // Read stdout in background
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to acquire stdout pipe"))?;
        let stdout_handle = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            tokio::io::BufReader::new(stdout)
                .read_to_end(&mut buf)
                .await
                .map(|_| buf)
        });

        let timeout_duration = Duration::from_secs(7200);

        // Poll for auth URLs while waiting for process to finish, with timeout
        let result = timeout(timeout_duration, async {
            loop {
                tokio::select! {
                    Some(url) = auth_rx.recv() => {
                        tracing::info!(url = %url, "claude CLI requesting auth");
                        let _ = channel.progress("auth_required", &url);
                    }
                    status = child.wait() => {
                        let status = status.context("failed to wait for claude process")?;
                        let exit_code = status.code().unwrap_or(-1);

                        let stdout_bytes = stdout_handle.await
                            .context("stdout reader panicked")?
                            .context("failed to read stdout")?;
                        let stderr_text = stderr_handle.await.unwrap_or_default();

                        if !stderr_text.is_empty() {
                            tracing::debug!(stderr = %stderr_text, "claude CLI stderr");
                        }
                        if exit_code != 0 {
                            tracing::warn!(exit_code, stderr = %stderr_text, "claude CLI exited with non-zero status");
                        }

                        let stdout = String::from_utf8_lossy(&stdout_bytes);
                        let conversation = match serde_json::from_str::<serde_json::Value>(&stdout) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    stdout_len = stdout.len(),
                                    "failed to parse claude stdout as JSON; returning raw output"
                                );
                                serde_json::json!({
                                    "raw_output": stdout.to_string(),
                                    "parse_error": e.to_string()
                                })
                            }
                        };

                        let task_text = task.chars().take(200).collect::<String>();
                        let git_refs = ensure_pr(&env.workspace, &task_text).await;

                        return Ok(DroneOutput {
                            exit_code,
                            conversation,
                            artifacts: vec![],
                            git_refs,
                        });
                    }
                }
            }
        }).await;

        match result {
            Ok(output) => output,
            Err(_) => {
                tracing::error!("claude CLI timed out after {:?}", timeout_duration);
                anyhow::bail!("claude CLI timed out after {:?}", timeout_duration);
            }
        }
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        environment::cleanup(&env.home).await;
    }
}

/// Run `claude auth login --method claude-ai` and surface the auth URL via the channel.
async fn authenticate(
    claude_bin: &Path,
    env: &DroneEnvironment,
    channel: &mut QueenChannel,
) -> Result<()> {
    let mut child = Command::new(claude_bin)
        .args(["auth", "login", "--console"])
        .env("HOME", &env.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn claude auth login")?;

    let mut cli_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin"))?;

    // Stream both stdout and stderr looking for the auth URL
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stderr"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdout"))?;

    let (auth_tx, mut auth_rx) = tokio::sync::mpsc::channel::<String>(4);
    let auth_tx2 = auth_tx.clone();

    // Read stderr lines
    let stderr_handle = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(stderr_line = %line, "auth stderr");
            if let Some(url) = extract_url(&line) {
                if url.contains("claude.ai/")
                    || url.contains("claude.com/")
                    || url.contains("console.anthropic.com/")
                {
                    let _ = auth_tx.send(url).await;
                }
            }
        }
    });

    // Read stdout lines (auth URL might appear on either stream)
    let stdout_handle = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(stdout_line = %line, "auth stdout");
            if let Some(url) = extract_url(&line) {
                if url.contains("claude.ai/")
                    || url.contains("claude.com/")
                    || url.contains("console.anthropic.com/")
                {
                    let _ = auth_tx2.send(url).await;
                }
            }
        }
    });

    let timeout_duration = Duration::from_secs(600); // 10 min for auth

    let result = timeout(timeout_duration, async {
        loop {
            tokio::select! {
                Some(url) = auth_rx.recv() => {
                    tracing::info!(url = %url, "auth URL detected — requesting code from queen");
                    // Send AuthRequest to Queen. This blocks until Queen relays the
                    // code that the user submits via POST /api/jobs/runs/{id}/auth.
                    match channel.request_auth(&url, "Visit the URL and submit the auth code") {
                        Ok(resp) if resp.approved => {
                            if let Some(code) = resp.code {
                                tracing::info!("auth code received, writing to CLI stdin");
                                use tokio::io::AsyncWriteExt;
                                cli_stdin.write_all(code.as_bytes()).await
                                    .context("failed to write auth code to CLI stdin")?;
                                cli_stdin.write_all(b"\n").await
                                    .context("failed to write newline after auth code")?;
                                cli_stdin.flush().await
                                    .context("failed to flush CLI stdin")?;
                            } else {
                                tracing::warn!("auth approved but no code provided");
                            }
                        }
                        Ok(_) => tracing::warn!("auth denied by queen"),
                        Err(e) => tracing::warn!(error = %e, "auth request to queen failed"),
                    }
                }
                status = child.wait() => {
                    let status = status.context("failed to wait for auth process")?;
                    let _ = stderr_handle.await;
                    let _ = stdout_handle.await;
                    if status.success() {
                        tracing::info!("claude auth login succeeded");
                        return Ok(());
                    } else {
                        let code = status.code().unwrap_or(-1);
                        anyhow::bail!("claude auth login failed with exit code {code}");
                    }
                }
            }
        }
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_) => anyhow::bail!("claude auth login timed out after 5 minutes"),
    }
}

/// Collect git branch name and PR URL from the workspace.
async fn collect_git_refs(workspace: &Path) -> GitRefs {
    let branch = match Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if b.is_empty() || b == "HEAD" {
                None
            } else {
                Some(b)
            }
        }
        Ok(o) => {
            tracing::debug!(exit_code = ?o.status.code(), "git rev-parse failed");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to run git rev-parse");
            None
        }
    };

    let pr_url = match Command::new("gh")
        .args(["pr", "view", "--json", "url", "-q", ".url"])
        .current_dir(workspace)
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            let url = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if url.is_empty() { None } else { Some(url) }
        }
        Ok(_) => None, // gh pr view returns non-zero when no PR exists — expected
        Err(e) => {
            tracing::debug!(error = %e, "gh CLI not available");
            None
        }
    };

    GitRefs { branch, pr_url }
}

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
                .args([
                    "commit",
                    "-m",
                    "chore: commit remaining changes from drone session",
                ])
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

/// Extract a URL from a line of text.
fn extract_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
