use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use drone_sdk::harness::QueenChannel;
use drone_sdk::protocol::{DroneEnvironment, DroneOutput, GitRefs, JobSpec};
use drone_sdk::runner::DroneRunner;
use tokio::fs;
use tokio::process::Command;

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
        _channel: &QueenChannel,
    ) -> Result<DroneOutput> {
        let task = fs::read_to_string(env.home.join(".task"))
            .await
            .context("failed to read .task file")?;

        let settings_path = env.home.join(".claude/settings.json");
        let claude_md_path = env.home.join("CLAUDE.md");

        let mut child = Command::new("claude")
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
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(task.as_bytes())
                .await
                .context("failed to write task to claude stdin")?;
            // stdin dropped here, closing the pipe
        }

        let output = child
            .wait_with_output()
            .await
            .context("failed to wait for claude process")?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse stdout as JSON conversation; fall back to raw string on parse failure
        let conversation = serde_json::from_str::<serde_json::Value>(&stdout)
            .unwrap_or_else(|_| serde_json::Value::String(stdout.into_owned()));

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
    let branch = get_branch(workspace).await;
    let pr_url = get_pr_url(workspace).await;
    GitRefs { branch, pr_url }
}

async fn get_branch(workspace: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() || branch == "HEAD" {
            None
        } else {
            Some(branch)
        }
    } else {
        None
    }
}

async fn get_pr_url(workspace: &Path) -> Option<String> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "url", "-q", ".url"])
        .current_dir(workspace)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() { None } else { Some(url) }
    } else {
        None
    }
}
