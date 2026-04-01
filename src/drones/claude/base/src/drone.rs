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
        Ok(env)
    }

    async fn execute(
        &self,
        env: &DroneEnvironment,
        _channel: &mut QueenChannel,
    ) -> Result<DroneOutput> {
        let task = fs::read_to_string(env.home.join(".task"))
            .await
            .context("failed to read .task file")?;

        let settings_path = env.home.join(".claude/settings.json");
        let claude_md_path = env.home.join("CLAUDE.md");

        let claude_bin = env.home.join(".claude/bin/claude");
        let mut child = Command::new(&claude_bin)
            .arg("--print")
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
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude CLI")?;

        // Write task to stdin, then close it
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to acquire stdin pipe for claude process"))?;
        {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(task.as_bytes())
                .await
                .context("failed to write task to claude stdin")?;
            // stdin dropped here, closing the pipe
        }

        let timeout_duration = Duration::from_secs(7200); // 2 hours default
        let output: std::process::Output =
            match timeout(timeout_duration, child.wait_with_output()).await {
                Ok(result) => result.context("failed to wait for claude process")?,
                Err(_) => {
                    tracing::error!("claude CLI timed out after {:?}", timeout_duration);
                    // child is dropped here which sends SIGKILL
                    anyhow::bail!("claude CLI timed out after {:?}", timeout_duration);
                }
            };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr_text = String::from_utf8_lossy(&output.stderr);

        if !stderr_text.is_empty() {
            tracing::debug!(stderr = %stderr_text, "claude CLI stderr");
        }

        if exit_code != 0 {
            tracing::warn!(
                exit_code,
                stderr = %stderr_text,
                "claude CLI exited with non-zero status"
            );
        }

        // Parse stdout as JSON conversation; log and fall back to structured error on parse failure
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

        let git_refs = collect_git_refs(&env.workspace).await;

        Ok(DroneOutput {
            exit_code,
            conversation,
            artifacts: vec![],
            git_refs,
        })
    }

    async fn teardown(&self, env: &DroneEnvironment) {
        environment::cleanup(&env.home).await;
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
