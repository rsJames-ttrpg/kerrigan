use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Spec,
    Plan,
    Implement,
    Review,
    Evolve,
    Freeform,
}

#[derive(Debug, Clone)]
pub struct StageConfig {
    pub stage: Stage,
    pub system_prompt: String,
    pub allowed_tools: Vec<String>,
    pub denied_tools: Vec<String>,
    pub entry_requirements: Vec<Requirement>,
    pub exit_conditions: Vec<ExitCondition>,
    pub git: StageGitConfig,
    pub max_turns: u32,
}

#[derive(Debug, Clone)]
pub enum Requirement {
    ArtifactExists { kind: String },
    FileExists { path: String },
    BranchExists { name: String },
}

#[derive(Debug, Clone)]
pub enum ExitCondition {
    FileCreated { glob: String },
    TestsPassing,
    PrCreated,
    ArtifactStored { kind: String },
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct StageGitConfig {
    pub branch_name: Option<String>,
    pub allowed_operations: Option<Vec<GitOperationKind>>,
    pub commit_on_checkpoint: bool,
    pub commit_on_task_complete: bool,
    pub pr_on_stage_complete: bool,
    pub protected_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOperationKind {
    Status,
    Diff,
    Log,
    CreateBranch,
    Commit,
    Push,
    CreatePr,
    CheckoutFile,
}

impl Stage {
    pub fn resolve(config: &serde_json::Value) -> Self {
        match config.get("stage").and_then(|v| v.as_str()) {
            Some("spec") => Stage::Spec,
            Some("plan") => Stage::Plan,
            Some("implement") => Stage::Implement,
            Some("review") => Stage::Review,
            Some("evolve") => Stage::Evolve,
            _ => Stage::Freeform,
        }
    }

    pub fn default_config(&self) -> StageConfig {
        match self {
            Stage::Spec => StageConfig {
                stage: Stage::Spec,
                system_prompt: String::new(),
                allowed_tools: vec![
                    "read_file",
                    "glob_search",
                    "grep_search",
                    "write_file",
                    "edit_file",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                denied_tools: vec!["bash", "git", "test", "agent"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                entry_requirements: vec![],
                exit_conditions: vec![
                    ExitCondition::FileCreated {
                        glob: "docs/specs/*.md".into(),
                    },
                    ExitCondition::ArtifactStored {
                        kind: "spec".into(),
                    },
                ],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![
                        GitOperationKind::Status,
                        GitOperationKind::Diff,
                        GitOperationKind::Log,
                    ]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Plan => StageConfig {
                stage: Stage::Plan,
                system_prompt: String::new(),
                allowed_tools: vec![
                    "read_file",
                    "glob_search",
                    "grep_search",
                    "write_file",
                    "edit_file",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
                denied_tools: vec!["bash", "git", "test", "agent"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                entry_requirements: vec![Requirement::ArtifactExists {
                    kind: "spec".into(),
                }],
                exit_conditions: vec![
                    ExitCondition::FileCreated {
                        glob: "docs/plans/*.md".into(),
                    },
                    ExitCondition::ArtifactStored {
                        kind: "plan".into(),
                    },
                ],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![
                        GitOperationKind::Status,
                        GitOperationKind::Diff,
                        GitOperationKind::Log,
                    ]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Implement => StageConfig {
                stage: Stage::Implement,
                system_prompt: String::new(),
                allowed_tools: vec![],
                denied_tools: vec![],
                entry_requirements: vec![Requirement::ArtifactExists {
                    kind: "plan".into(),
                }],
                exit_conditions: vec![ExitCondition::TestsPassing, ExitCondition::PrCreated],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: None,
                    commit_on_checkpoint: true,
                    commit_on_task_complete: true,
                    pr_on_stage_complete: true,
                    protected_paths: vec!["CLAUDE.md".into()],
                },
                max_turns: 100,
            },
            Stage::Review => StageConfig {
                stage: Stage::Review,
                system_prompt: String::new(),
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "git", "bash"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                denied_tools: vec!["write_file", "edit_file"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                entry_requirements: vec![],
                exit_conditions: vec![ExitCondition::ArtifactStored {
                    kind: "review".into(),
                }],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![
                        GitOperationKind::Status,
                        GitOperationKind::Diff,
                        GitOperationKind::Log,
                        GitOperationKind::Commit,
                        GitOperationKind::Push,
                    ]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Evolve => StageConfig {
                stage: Stage::Evolve,
                system_prompt: String::new(),
                allowed_tools: vec!["read_file", "glob_search", "grep_search", "bash"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                denied_tools: vec!["write_file", "edit_file", "git", "test", "agent"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
                entry_requirements: vec![],
                exit_conditions: vec![],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: Some(vec![]),
                    commit_on_checkpoint: false,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 25,
            },
            Stage::Freeform => StageConfig {
                stage: Stage::Freeform,
                system_prompt: String::new(),
                allowed_tools: vec![],
                denied_tools: vec![],
                entry_requirements: vec![],
                exit_conditions: vec![],
                git: StageGitConfig {
                    branch_name: None,
                    allowed_operations: None,
                    commit_on_checkpoint: true,
                    commit_on_task_complete: false,
                    pr_on_stage_complete: false,
                    protected_paths: vec![],
                },
                max_turns: 50,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn resolve_known_stages() {
        assert_eq!(Stage::resolve(&json!({"stage": "spec"})), Stage::Spec);
        assert_eq!(Stage::resolve(&json!({"stage": "plan"})), Stage::Plan);
        assert_eq!(
            Stage::resolve(&json!({"stage": "implement"})),
            Stage::Implement
        );
        assert_eq!(Stage::resolve(&json!({"stage": "review"})), Stage::Review);
        assert_eq!(Stage::resolve(&json!({"stage": "evolve"})), Stage::Evolve);
    }

    #[test]
    fn resolve_unknown_defaults_to_freeform() {
        assert_eq!(
            Stage::resolve(&json!({"stage": "unknown"})),
            Stage::Freeform
        );
        assert_eq!(Stage::resolve(&json!({})), Stage::Freeform);
        assert_eq!(Stage::resolve(&json!({"other": "field"})), Stage::Freeform);
    }

    #[test]
    fn spec_stage_denies_bash_and_git() {
        let config = Stage::Spec.default_config();
        assert!(config.denied_tools.contains(&"bash".to_string()));
        assert!(config.denied_tools.contains(&"git".to_string()));
    }

    #[test]
    fn plan_stage_denies_bash_and_git() {
        let config = Stage::Plan.default_config();
        assert!(config.denied_tools.contains(&"bash".to_string()));
        assert!(config.denied_tools.contains(&"git".to_string()));
    }

    #[test]
    fn implement_stage_allows_all_tools() {
        let config = Stage::Implement.default_config();
        assert!(config.allowed_tools.is_empty(), "empty = all allowed");
        assert!(config.denied_tools.is_empty());
    }

    #[test]
    fn implement_stage_protects_claude_md() {
        let config = Stage::Implement.default_config();
        assert!(
            config
                .git
                .protected_paths
                .contains(&"CLAUDE.md".to_string())
        );
    }

    #[test]
    fn implement_stage_requires_plan_artifact() {
        let config = Stage::Implement.default_config();
        assert!(config.entry_requirements.iter().any(|r| matches!(
            r,
            Requirement::ArtifactExists { kind } if kind == "plan"
        )));
    }

    #[test]
    fn review_stage_denies_write_tools() {
        let config = Stage::Review.default_config();
        assert!(config.denied_tools.contains(&"write_file".to_string()));
        assert!(config.denied_tools.contains(&"edit_file".to_string()));
    }

    #[test]
    fn evolve_stage_has_no_git_ops() {
        let config = Stage::Evolve.default_config();
        assert_eq!(config.git.allowed_operations, Some(vec![]));
    }

    #[test]
    fn freeform_stage_allows_all_operations() {
        let config = Stage::Freeform.default_config();
        assert!(
            config.git.allowed_operations.is_none(),
            "None = all allowed"
        );
        assert!(config.allowed_tools.is_empty());
        assert!(config.denied_tools.is_empty());
    }

    #[test]
    fn stage_serde_roundtrip() {
        let stage = Stage::Implement;
        let json = serde_json::to_string(&stage).unwrap();
        assert_eq!(json, "\"implement\"");
        let decoded: Stage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, Stage::Implement);
    }

    #[test]
    fn max_turns_vary_by_stage() {
        assert_eq!(Stage::Spec.default_config().max_turns, 25);
        assert_eq!(Stage::Implement.default_config().max_turns, 100);
        assert_eq!(Stage::Freeform.default_config().max_turns, 50);
    }
}
