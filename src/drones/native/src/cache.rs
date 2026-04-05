use std::path::{Path, PathBuf};

/// Manages a cache of bare git repos and creates worktrees for drone jobs.
pub struct RepoCache {
    cache_dir: PathBuf,
}

impl RepoCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get or create a cached bare repo, then create a worktree for this job.
    pub async fn checkout(
        &self,
        repo_url: &str,
        branch: &str,
        worktree_path: &Path,
    ) -> anyhow::Result<()> {
        let bare_path = self.bare_repo_path(repo_url);

        if bare_path.exists() {
            self.git_fetch(&bare_path).await?;
        } else {
            self.git_clone_bare(repo_url, &bare_path).await?;
        }

        self.git_worktree_add(&bare_path, branch, worktree_path)
            .await?;
        Ok(())
    }

    /// Remove a worktree previously created by `checkout`.
    pub async fn cleanup_worktree(
        &self,
        repo_url: &str,
        worktree_path: &Path,
    ) -> anyhow::Result<()> {
        let bare_path = self.bare_repo_path(repo_url);
        let _ = tokio::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(worktree_path)
            .current_dir(&bare_path)
            .output()
            .await;
        Ok(())
    }

    /// Deterministic path for a bare repo based on URL hash.
    pub fn bare_repo_path(&self, url: &str) -> PathBuf {
        let hash = blake3::hash(url.as_bytes()).to_hex();
        self.cache_dir
            .join("repos")
            .join(&hash[..16])
            .with_extension("git")
    }

    async fn git_clone_bare(&self, url: &str, path: &Path) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let output = tokio::process::Command::new("git")
            .args(["clone", "--bare", url])
            .arg(path)
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "git clone bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    async fn git_fetch(&self, bare_path: &Path) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(bare_path)
            .output()
            .await?;
        anyhow::ensure!(
            output.status.success(),
            "git fetch failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        anyhow::ensure!(
            output.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_repo_path_is_deterministic() {
        let cache = RepoCache::new(PathBuf::from("/tmp/cache"));
        let url = "https://github.com/org/repo.git";

        let path1 = cache.bare_repo_path(url);
        let path2 = cache.bare_repo_path(url);
        assert_eq!(path1, path2);
    }

    #[test]
    fn bare_repo_path_differs_by_url() {
        let cache = RepoCache::new(PathBuf::from("/tmp/cache"));

        let path1 = cache.bare_repo_path("https://github.com/org/repo1.git");
        let path2 = cache.bare_repo_path("https://github.com/org/repo2.git");
        assert_ne!(path1, path2);
    }

    #[test]
    fn bare_repo_path_structure() {
        let cache = RepoCache::new(PathBuf::from("/tmp/cache"));
        let path = cache.bare_repo_path("https://github.com/org/repo.git");

        assert!(path.starts_with("/tmp/cache/repos/"));
        assert_eq!(path.extension().unwrap(), "git");
        // Hash prefix is 16 hex chars
        let stem = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(stem.len(), 16);
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cleanup_worktree_is_idempotent() {
        // cleanup_worktree should not error even if the bare repo doesn't exist
        let cache = RepoCache::new(PathBuf::from("/tmp/nonexistent-cache"));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(cache.cleanup_worktree(
            "https://github.com/org/repo.git",
            Path::new("/tmp/nonexistent-worktree"),
        ));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn checkout_with_real_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let origin_path = tmp.path().join("origin.git");

        // Create a bare repo to use as origin
        let output = tokio::process::Command::new("git")
            .args(["init", "--bare"])
            .arg(&origin_path)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());

        // Create a regular repo, add a commit, and push to the bare origin
        let source_path = tmp.path().join("source");
        tokio::fs::create_dir_all(&source_path).await.unwrap();

        let cmds: Vec<(&str, Vec<&str>)> = vec![
            ("git", vec!["init"]),
            ("git", vec!["config", "user.email", "test@test.com"]),
            ("git", vec!["config", "user.name", "Test"]),
            ("git", vec!["checkout", "-b", "main"]),
        ];
        for (cmd, args) in &cmds {
            let o = tokio::process::Command::new(cmd)
                .args(args)
                .current_dir(&source_path)
                .output()
                .await
                .unwrap();
            assert!(o.status.success(), "{cmd} {args:?} failed");
        }

        tokio::fs::write(source_path.join("README.md"), "hello")
            .await
            .unwrap();

        let cmds2: Vec<(&str, Vec<&str>)> = vec![
            ("git", vec!["add", "."]),
            ("git", vec!["commit", "-m", "initial"]),
            (
                "git",
                vec!["remote", "add", "origin", origin_path.to_str().unwrap()],
            ),
            ("git", vec!["push", "origin", "main"]),
        ];
        for (cmd, args) in &cmds2 {
            let o = tokio::process::Command::new(cmd)
                .args(args)
                .current_dir(&source_path)
                .output()
                .await
                .unwrap();
            assert!(o.status.success(), "{cmd} {args:?} failed");
        }

        // Now test the RepoCache
        let cache_dir = tmp.path().join("cache");
        let cache = RepoCache::new(cache_dir);
        let worktree_path = tmp.path().join("worktree");

        cache
            .checkout(origin_path.to_str().unwrap(), "main", &worktree_path)
            .await
            .unwrap();

        // Verify worktree has the file
        assert!(worktree_path.join("README.md").exists());
        let content = tokio::fs::read_to_string(worktree_path.join("README.md"))
            .await
            .unwrap();
        assert_eq!(content, "hello");

        // Cleanup
        cache
            .cleanup_worktree(origin_path.to_str().unwrap(), &worktree_path)
            .await
            .unwrap();
        assert!(!worktree_path.exists());
    }
}
