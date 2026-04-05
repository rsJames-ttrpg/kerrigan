use runtime::tools::ToolRegistry;

use crate::pipeline::{Stage, StageConfig, StageGitConfig};

/// A named, prioritized section of the system prompt.
pub struct PromptSection {
    pub name: String,
    pub content: String,
    pub priority: u8,
}

/// Builds a system prompt from prioritized sections.
///
/// Higher priority sections appear first. When a token budget is applied,
/// lowest priority sections are dropped first.
pub struct PromptBuilder {
    sections: Vec<PromptSection>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    pub fn add(&mut self, name: &str, content: String, priority: u8) {
        self.sections.push(PromptSection {
            name: name.to_string(),
            content,
            priority,
        });
    }

    /// Build the full system prompt, sorted by priority (highest first).
    pub fn build(&self) -> Vec<String> {
        let mut sorted: Vec<_> = self.sections.iter().collect();
        sorted.sort_by_key(|b| std::cmp::Reverse(b.priority));
        sorted.iter().map(|s| s.content.clone()).collect()
    }

    /// Build with a token budget — drop lowest priority sections first.
    /// Token estimate: len / 4 (rough char-to-token ratio).
    pub fn build_within_budget(&self, max_tokens: u32) -> Vec<String> {
        let mut sorted: Vec<_> = self.sections.iter().collect();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

        let mut result = Vec::new();
        let mut tokens: u32 = 0;
        for section in sorted {
            let section_tokens = (section.content.len() as u32) / 4;
            if tokens + section_tokens <= max_tokens {
                result.push(section.content.clone());
                tokens += section_tokens;
            }
        }
        result
    }

    /// Build a prompt for a specific pipeline stage.
    pub fn for_stage(
        stage: &Stage,
        stage_config: &StageConfig,
        registry: &ToolRegistry,
        workspace_context: &str,
        task_state: Option<&str>,
        checkpoint_ref: Option<&str>,
    ) -> Self {
        let mut builder = Self::new();

        // Priority 255: Identity
        builder.add(
            "identity",
            "You are a software development agent working in the kerrigan platform. \
             You execute tasks precisely, commit frequently, and report progress."
                .into(),
            255,
        );

        // Priority 255: Environment
        builder.add(
            "environment",
            format!(
                "Working directory: {}\nDate: {}\nStage: {stage:?}",
                std::env::current_dir().unwrap_or_default().display(),
                chrono::Utc::now().format("%Y-%m-%d"),
            ),
            255,
        );

        // Priority 200: Stage mission
        if !stage_config.system_prompt.is_empty() {
            builder.add("mission", stage_config.system_prompt.clone(), 200);
        }

        // Priority 180: Tool guide (auto-generated)
        let tool_guide = build_tool_guide(registry, stage_config);
        if !tool_guide.is_empty() {
            builder.add("tools", tool_guide, 180);
        }

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

fn build_tool_guide(registry: &ToolRegistry, config: &StageConfig) -> String {
    let defs = registry.definitions(&config.allowed_tools, &config.denied_tools);
    if defs.is_empty() {
        return String::new();
    }
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
        rules.push_str(&format!(
            "- Do NOT modify: {}\n",
            git.protected_paths.join(", ")
        ));
    }
    rules.push_str("- Do NOT force push\n");
    rules.push_str("- Commit frequently with clear messages\n");
    rules
}

fn build_constraints(config: &StageConfig) -> String {
    let mut constraints = Vec::new();
    if !config.denied_tools.is_empty() {
        constraints.push(format!(
            "Do NOT use these tools: {}",
            config.denied_tools.join(", ")
        ));
    }
    constraints.push("Do NOT install system packages".into());
    constraints.push("Do NOT add features beyond what was asked".into());
    constraints.push("Do NOT refactor unrelated code".into());
    constraints.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_sorted_by_priority_descending() {
        let mut builder = PromptBuilder::new();
        builder.add("low", "low priority".into(), 10);
        builder.add("high", "high priority".into(), 200);
        builder.add("mid", "mid priority".into(), 100);

        let result = builder.build();
        assert_eq!(
            result,
            vec!["high priority", "mid priority", "low priority"]
        );
    }

    #[test]
    fn budget_drops_low_priority_first() {
        let mut builder = PromptBuilder::new();
        // "high" = 4 chars => 1 token
        builder.add("high", "high".into(), 200);
        // "mid-content" = 11 chars => 2 tokens
        builder.add("mid", "mid-content".into(), 100);
        // "low-priority-content" = 20 chars => 5 tokens
        builder.add("low", "low-priority-content".into(), 10);

        // Budget of 3 tokens should include high (1) + mid (2) = 3, drop low (5)
        let result = builder.build_within_budget(3);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "high");
        assert_eq!(result[1], "mid-content");
    }

    #[test]
    fn budget_zero_returns_empty() {
        let mut builder = PromptBuilder::new();
        builder.add("any", "content".into(), 100);

        let result = builder.build_within_budget(0);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_builder_returns_empty() {
        let builder = PromptBuilder::new();
        assert!(builder.build().is_empty());
        assert!(builder.build_within_budget(1000).is_empty());
    }

    #[test]
    fn for_stage_includes_identity_and_environment() {
        let stage = Stage::Implement;
        let config = stage.default_config();
        let registry = ToolRegistry::new();

        let builder = PromptBuilder::for_stage(&stage, &config, &registry, "", None, None);

        let result = builder.build();
        assert!(result.len() >= 2);
        // Identity is first (priority 255)
        assert!(result[0].contains("software development agent"));
        // Environment is second (also priority 255, but added second)
        assert!(result[1].contains("Working directory"));
    }

    #[test]
    fn for_stage_includes_project_context() {
        let stage = Stage::Implement;
        let config = stage.default_config();
        let registry = ToolRegistry::new();

        let builder = PromptBuilder::for_stage(
            &stage,
            &config,
            &registry,
            "# My Project\nThis is a test project.",
            None,
            None,
        );

        let result = builder.build();
        assert!(result.iter().any(|s| s.contains("My Project")));
    }

    #[test]
    fn for_stage_includes_task_state() {
        let stage = Stage::Implement;
        let config = stage.default_config();
        let registry = ToolRegistry::new();

        let builder = PromptBuilder::for_stage(
            &stage,
            &config,
            &registry,
            "",
            Some("Task 3 of 5: implement foo module"),
            None,
        );

        let result = builder.build();
        assert!(result.iter().any(|s| s.contains("Task 3 of 5")));
    }

    #[test]
    fn for_stage_includes_checkpoint_at_lowest_priority() {
        let stage = Stage::Implement;
        let config = stage.default_config();
        let registry = ToolRegistry::new();

        let builder = PromptBuilder::for_stage(
            &stage,
            &config,
            &registry,
            "",
            None,
            Some("Checkpoint: 3 files changed, tests passing"),
        );

        let result = builder.build();
        // Checkpoint should be last (priority 50)
        assert!(result.last().unwrap().contains("Checkpoint"));
    }

    #[test]
    fn for_stage_omits_empty_sections() {
        let stage = Stage::Implement;
        let mut config = stage.default_config();
        config.system_prompt = String::new(); // empty mission

        let registry = ToolRegistry::new();

        let builder = PromptBuilder::for_stage(&stage, &config, &registry, "", None, None);

        let result = builder.build();
        // No empty strings in output
        assert!(result.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn git_rules_include_protected_paths() {
        let git = StageGitConfig {
            branch_name: Some("feat/test".into()),
            allowed_operations: None,
            commit_on_checkpoint: true,
            commit_on_task_complete: true,
            pr_on_stage_complete: true,
            protected_paths: vec!["CLAUDE.md".into(), ".buckconfig".into()],
        };

        let rules = build_git_rules(&git);
        assert!(rules.contains("feat/test"));
        assert!(rules.contains("CLAUDE.md, .buckconfig"));
        assert!(rules.contains("Do NOT force push"));
    }

    #[test]
    fn constraints_include_denied_tools() {
        let config = Stage::Spec.default_config();
        let constraints = build_constraints(&config);
        assert!(constraints.contains("bash"));
        assert!(constraints.contains("git"));
    }

    #[test]
    fn constraints_always_include_base_rules() {
        let config = Stage::Implement.default_config();
        let constraints = build_constraints(&config);
        assert!(constraints.contains("Do NOT install system packages"));
        assert!(constraints.contains("Do NOT add features beyond what was asked"));
        assert!(constraints.contains("Do NOT refactor unrelated code"));
    }
}
