use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use drone_sdk::protocol::DroneEnvironment;
use tokio::fs;
use tokio::process::Command;

const SETTINGS_JSON: &[u8] = include_bytes!("config/settings.json");
const CLAUDE_MD: &[u8] = include_bytes!("config/CLAUDE.md");
const CLAUDE_CLI: &[u8] = include_bytes!("config/claude-cli");

/// Create an isolated drone home directory for the given job run.
///
/// Creates:
/// - `/tmp/drone-{id}/` — home directory
/// - `/tmp/drone-{id}/.claude/` — Claude config dir
/// - `/tmp/drone-{id}/.claude/settings.json` — embedded settings
/// - `/tmp/drone-{id}/CLAUDE.md` — embedded system prompt
/// - `/tmp/drone-{id}/.claude/.credentials.json` — symlink to real credentials
///
/// Note: workspace (`/tmp/drone-{id}/workspace/`) is NOT pre-created here —
/// `git clone` creates it during `clone_repo()`.
pub async fn create_home(job_run_id: &str) -> Result<DroneEnvironment> {
    if !job_run_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!("invalid job_run_id: contains disallowed characters: {job_run_id}");
    }

    let home = PathBuf::from(format!("/tmp/drone-{job_run_id}"));
    let claude_dir = home.join(".claude");
    let workspace = home.join("workspace");

    // Create directories
    fs::create_dir_all(&claude_dir)
        .await
        .with_context(|| format!("failed to create .claude dir at {}", claude_dir.display()))?;
    // Note: workspace is intentionally NOT pre-created — git clone creates it

    // Write embedded settings.json
    fs::write(claude_dir.join("settings.json"), SETTINGS_JSON)
        .await
        .context("failed to write settings.json")?;

    // Write embedded Claude CLI binary
    let claude_bin_dir = claude_dir.join("bin");
    fs::create_dir_all(&claude_bin_dir)
        .await
        .context("failed to create .claude/bin dir")?;
    let claude_bin = claude_bin_dir.join("claude");
    fs::write(&claude_bin, CLAUDE_CLI)
        .await
        .context("failed to write claude CLI binary")?;
    fs::set_permissions(&claude_bin, std::fs::Permissions::from_mode(0o755))
        .await
        .context("failed to set claude CLI permissions")?;

    // Write embedded CLAUDE.md
    fs::write(home.join("CLAUDE.md"), CLAUDE_MD)
        .await
        .context("failed to write CLAUDE.md")?;

    // Symlink auth credentials from the real home dir
    if let Some(real_home) = dirs::home_dir() {
        let real_creds = real_home.join(".claude").join(".credentials.json");
        let link_path = claude_dir.join(".credentials.json");
        if real_creds.exists() {
            // Remove existing symlink if any
            let _ = fs::remove_file(&link_path).await;
            tokio::fs::symlink(&real_creds, &link_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to symlink credentials from {} to {}",
                        real_creds.display(),
                        link_path.display()
                    )
                })?;
        } else {
            tracing::warn!(
                "credentials not found at {}; drone will run unauthenticated",
                real_creds.display()
            );
        }
    } else {
        tracing::warn!("could not determine home directory; skipping credentials symlink");
    }

    Ok(DroneEnvironment { home, workspace })
}

/// Write the task description to `.task` in the drone home.
pub async fn write_task(home: &Path, task: &str) -> Result<()> {
    fs::write(home.join(".task"), task)
        .await
        .context("failed to write .task file")
}

/// Rewrite the Overseer MCP URL placeholder in settings.json with the actual URL.
pub async fn configure_mcp_url(home: &Path, overseer_url: &str) -> Result<()> {
    let settings_path = home.join(".claude/settings.json");
    let content = fs::read_to_string(&settings_path)
        .await
        .context("failed to read settings.json for MCP URL rewrite")?;
    let mcp_url = format!("{}/mcp", overseer_url.trim_end_matches('/'));
    let updated = content.replace("OVERSEER_MCP_URL_PLACEHOLDER", &mcp_url);
    fs::write(&settings_path, updated)
        .await
        .context("failed to write updated settings.json")?;
    Ok(())
}

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
    let existing = fs::read_to_string(&gitconfig_path)
        .await
        .unwrap_or_default();
    fs::write(&gitconfig_path, format!("{existing}{gitconfig}"))
        .await
        .context("failed to write .gitconfig")?;

    Ok(())
}

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

/// Shallow-clone a git repository into `workspace`.
/// `home` is the drone's isolated HOME — needed for git credential helper.
pub async fn clone_repo(
    repo_url: &str,
    branch: Option<&str>,
    workspace: &Path,
    home: &Path,
) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.env("HOME", home);
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    cmd.arg("--");
    cmd.arg(repo_url);
    cmd.arg(workspace);

    let output = cmd.output().await.context("failed to spawn git clone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git clone failed: {stderr}");
    }

    Ok(())
}

/// Remove the drone home directory. Errors are swallowed (best-effort cleanup).
pub async fn cleanup(home: &Path) {
    if let Err(e) = fs::remove_dir_all(home).await {
        tracing::warn!("failed to clean up drone home {}: {e}", home.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_home_creates_dirs() {
        let id = format!("test-{}", std::process::id());
        let env = create_home(&id).await.expect("create_home should succeed");

        // Dirs exist
        assert!(env.home.exists(), "home dir should exist");
        assert!(
            env.home.join(".claude").exists(),
            ".claude dir should exist"
        );
        // workspace is intentionally NOT pre-created (git clone creates it)
        assert!(
            !env.workspace.exists(),
            "workspace dir should not be pre-created"
        );

        // Embedded files written
        let settings = fs::read(env.home.join(".claude/settings.json"))
            .await
            .expect("settings.json should exist");
        assert_eq!(
            settings, SETTINGS_JSON,
            "settings content should match embedded bytes"
        );

        let claude_md = fs::read(env.home.join("CLAUDE.md"))
            .await
            .expect("CLAUDE.md should exist");
        assert_eq!(
            claude_md, CLAUDE_MD,
            "CLAUDE.md content should match embedded bytes"
        );

        // Embedded Claude CLI written and executable
        let claude_bin = env.home.join(".claude/bin/claude");
        assert!(claude_bin.exists(), "claude CLI binary should exist");
        let metadata = std::fs::metadata(&claude_bin).expect("claude CLI metadata");
        assert!(
            metadata.permissions().mode() & 0o111 != 0,
            "claude CLI should be executable"
        );

        cleanup(&env.home).await;
        assert!(!env.home.exists(), "home should be removed after cleanup");
    }

    #[tokio::test]
    async fn test_create_home_rejects_invalid_id() {
        let result = create_home("../evil/path").await;
        assert!(
            result.is_err(),
            "should reject job_run_id with path traversal"
        );
        let result2 = create_home("id with spaces").await;
        assert!(result2.is_err(), "should reject job_run_id with spaces");
    }

    #[tokio::test]
    async fn test_cleanup_nonexistent_is_ok() {
        let path = PathBuf::from("/tmp/drone-does-not-exist-12345");
        // Should not panic or return an error
        cleanup(&path).await;
    }
}
