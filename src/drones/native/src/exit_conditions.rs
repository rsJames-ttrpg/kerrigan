use std::path::Path;

use crate::pipeline::ExitCondition;

#[derive(Debug)]
pub struct ConditionResult {
    pub condition: String,
    pub met: bool,
    pub detail: String,
}

pub async fn check_exit_conditions(
    conditions: &[ExitCondition],
    workspace: &Path,
) -> Vec<ConditionResult> {
    let mut results = Vec::new();
    for cond in conditions {
        let result = match cond {
            ExitCondition::FileCreated { glob } => check_file_created(workspace, glob),
            ExitCondition::TestsPassing => check_tests_passing(workspace).await,
            ExitCondition::PrCreated => check_pr_exists(workspace).await,
            ExitCondition::ArtifactStored { kind } => ConditionResult {
                condition: format!("artifact:{kind}"),
                met: false,
                detail: "requires MCP check".into(),
            },
            ExitCondition::Custom(command) => check_custom_command(command).await,
        };
        results.push(result);
    }
    results
}

fn check_file_created(workspace: &Path, pattern: &str) -> ConditionResult {
    let matcher = match globset::Glob::new(pattern) {
        Ok(g) => g.compile_matcher(),
        Err(e) => {
            return ConditionResult {
                condition: format!("file:{pattern}"),
                met: false,
                detail: format!("invalid glob: {e}"),
            };
        }
    };

    let met = find_matching_file(workspace, workspace, &matcher);

    ConditionResult {
        condition: format!("file:{pattern}"),
        met,
        detail: if met {
            "matching file found".into()
        } else {
            "no matching file found".into()
        },
    }
}

fn find_matching_file(base: &Path, dir: &Path, matcher: &globset::GlobMatcher) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(relative) = path.strip_prefix(base) {
            if matcher.is_match(relative) {
                return true;
            }
        }
        if path.is_dir() {
            if find_matching_file(base, &path, matcher) {
                return true;
            }
        }
    }
    false
}

async fn check_tests_passing(workspace: &Path) -> ConditionResult {
    let output = tokio::process::Command::new("cargo")
        .args(["test"])
        .current_dir(workspace)
        .output()
        .await;

    match output {
        Ok(o) => ConditionResult {
            condition: "tests_passing".into(),
            met: o.status.success(),
            detail: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => ConditionResult {
            condition: "tests_passing".into(),
            met: false,
            detail: format!("failed to run cargo test: {e}"),
        },
    }
}

async fn check_pr_exists(workspace: &Path) -> ConditionResult {
    let output = tokio::process::Command::new("gh")
        .args(["pr", "view", "--json", "url"])
        .current_dir(workspace)
        .output()
        .await;

    match output {
        Ok(o) => ConditionResult {
            condition: "pr_created".into(),
            met: o.status.success(),
            detail: String::from_utf8_lossy(&o.stdout).to_string(),
        },
        Err(e) => ConditionResult {
            condition: "pr_created".into(),
            met: false,
            detail: format!("failed to run gh pr view: {e}"),
        },
    }
}

async fn check_custom_command(command: &str) -> ConditionResult {
    let output = tokio::process::Command::new("sh")
        .args(["-c", command])
        .output()
        .await;

    match output {
        Ok(o) => ConditionResult {
            condition: format!("custom:{command}"),
            met: o.status.success(),
            detail: String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr),
        },
        Err(e) => ConditionResult {
            condition: format!("custom:{command}"),
            met: false,
            detail: format!("failed to execute: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_created_matches_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("docs").join("specs");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("test.md"), "content").unwrap();

        let result = check_file_created(dir.path(), "docs/specs/*.md");
        assert!(result.met, "should find docs/specs/test.md");
    }

    #[test]
    fn file_created_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_file_created(dir.path(), "docs/specs/*.md");
        assert!(!result.met);
    }

    #[test]
    fn file_created_invalid_glob() {
        let dir = tempfile::tempdir().unwrap();
        let result = check_file_created(dir.path(), "[invalid");
        assert!(!result.met);
        assert!(result.detail.contains("invalid glob"));
    }

    #[tokio::test]
    async fn custom_command_success() {
        let result = check_custom_command("true").await;
        assert!(result.met);
    }

    #[tokio::test]
    async fn custom_command_failure() {
        let result = check_custom_command("false").await;
        assert!(!result.met);
    }

    #[tokio::test]
    async fn check_exit_conditions_empty() {
        let dir = tempfile::tempdir().unwrap();
        let results = check_exit_conditions(&[], dir.path()).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn check_exit_conditions_artifact_not_met() {
        let dir = tempfile::tempdir().unwrap();
        let conditions = vec![ExitCondition::ArtifactStored {
            kind: "spec".into(),
        }];
        let results = check_exit_conditions(&conditions, dir.path()).await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].met);
        assert!(results[0].detail.contains("MCP"));
    }

    #[tokio::test]
    async fn check_exit_conditions_file_created() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("output");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("result.txt"), "done").unwrap();

        let conditions = vec![ExitCondition::FileCreated {
            glob: "output/*.txt".into(),
        }];
        let results = check_exit_conditions(&conditions, dir.path()).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].met);
    }
}
