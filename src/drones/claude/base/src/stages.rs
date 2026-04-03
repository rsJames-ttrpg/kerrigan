use serde_json::Value;

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
3. Push the branch to origin
4. Create a pull request with a clear title, description, and test plan

Do NOT merge the PR. The operator will review and merge.

## Artifacts

When you produce a key output (spec, plan, review), store it as an Overseer artifact
using the Overseer MCP tools available to you (if configured). This ensures traceability
alongside the git commit."#;

pub fn generate_claude_md(stage: &str, config: &Value) -> Option<String> {
    match stage {
        "spec" => Some(generate_spec(config)),
        "plan" => Some(generate_plan(config)),
        "implement" => Some(generate_implement(config)),
        "review" => Some(generate_review(config)),
        "evolve" => Some(generate_evolve(config)),
        _ => None,
    }
}

fn generate_spec(_config: &Value) -> String {
    format!(
        r#"# Claude Drone — Spec Writer

You are a Claude Code drone tasked with writing a design specification.

## First Step

Invoke `/superpowers:brainstorming` to turn the problem description into a fully formed
design spec. You MUST use this skill — do not skip it or attempt to write the spec directly.

Since you are an autonomous drone, answer any clarifying questions yourself based on
what you learn from the codebase. Follow the skill's full process through to writing and
committing the spec.

Save the spec to `docs/specs/` following the naming convention `YYYY-MM-DD-<topic>-design.md`.

Also store the spec as an Overseer artifact via MCP if available.

{BASE_RULES}
"#
    )
}

fn generate_plan(config: &Value) -> String {
    let spec_path = config
        .get("spec_path")
        .and_then(|v| v.as_str())
        .unwrap_or("(spec path not provided — check docs/specs/ for the relevant spec)");

    format!(
        r#"# Claude Drone — Plan Writer

You are a Claude Code drone tasked with writing an implementation plan.

## First Step

Read the design spec at `{spec_path}`, then invoke `/superpowers:writing-plans` to create
a detailed implementation plan. You MUST use this skill — do not skip it or write the plan
directly.

The plan should be comprehensive enough that another engineer (or drone) can implement it
without additional context.

Save the plan to `docs/plans/` following the naming convention `YYYY-MM-DD-<topic>.md`.

Also store the plan as an Overseer artifact via MCP if available.

{BASE_RULES}
"#
    )
}

fn generate_implement(config: &Value) -> String {
    let plan_path = config
        .get("plan_path")
        .and_then(|v| v.as_str())
        .unwrap_or("(plan path not provided — check docs/plans/ for the relevant plan)");

    format!(
        r#"# Claude Drone — Implementer

You are a Claude Code drone tasked with implementing code from a plan.

## First Step

Invoke `/superpowers:using-superpowers` to initialize your skill system. Then read the
implementation plan at `{plan_path}` and invoke `/superpowers:subagent-driven-development`
to execute it task by task.

You MUST use these skills — do not skip them or implement without the structured workflow.

Follow TDD: write tests first, then implement. Run tests after each task. Commit frequently.

When all tasks are complete, ensure all tests pass and create the PR.

{BASE_RULES}
"#
    )
}

fn generate_review(config: &Value) -> String {
    let pr_url = config
        .get("pr_url")
        .and_then(|v| v.as_str())
        .unwrap_or("(PR URL not provided — check for open PRs)");

    format!(
        r#"# Claude Drone — Reviewer

You are a Claude Code drone tasked with reviewing a pull request.

## First Step

The PR to review: {pr_url}

Invoke `/pr-review-toolkit:review-pr` to perform a thorough code review. You MUST use
this skill — do not skip it or review manually.

You have full access to the codebase — run tests, trace code paths, check types,
verify behavior.

Post your review feedback as PR comments using `gh`. Also store the review as an
Overseer artifact via MCP if available.

## Git Workflow

You are reviewing an existing PR. Do NOT create a new branch or a new PR.

1. You are already on the PR branch — work here
2. If you have review fix suggestions, commit them directly to this branch
3. Push any commits to origin
4. Post review comments on the existing PR using `gh pr review`

Do NOT merge the PR. The operator will review and merge.

## Artifacts

When you produce a key output (spec, plan, review), store it as an Overseer artifact
using the Overseer MCP tools available to you (if configured). This ensures traceability
alongside the git commit.
"#
    )
}

fn generate_evolve(_config: &Value) -> String {
    format!(
        r#"# Evolution Chamber Analysis

You are reviewing an Evolution Chamber analysis report. Your task is to create GitHub issues for actionable recommendations.

## Instructions

1. Read the analysis report provided in the task
2. For each recommendation with severity High or Medium:
   - Create a GitHub issue as a problem spec
   - Title: the recommendation title
   - Body: Include the detail, evidence, and your proposed approach
   - Label: `evolution-chamber`
3. Skip Low severity recommendations unless they have compelling evidence
4. Group related recommendations into a single issue when they share a root cause

## Output

Create the issues using `gh issue create`. Report what you created.

{BASE_RULES}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generate_spec_stage() {
        let config = json!({"task": "fix auth bug"});
        let md = generate_claude_md("spec", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("Spec Writer"));
        assert!(content.contains("/superpowers:brainstorming"));
        assert!(content.contains("Git Workflow"));
    }

    #[test]
    fn test_generate_plan_stage_with_spec_path() {
        let config = json!({"spec_path": "docs/specs/2026-04-01-auth-design.md"});
        let md = generate_claude_md("plan", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("Plan Writer"));
        assert!(content.contains("docs/specs/2026-04-01-auth-design.md"));
    }

    #[test]
    fn test_generate_plan_stage_without_spec_path() {
        let config = json!({});
        let md = generate_claude_md("plan", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("spec path not provided"));
    }

    #[test]
    fn test_generate_implement_stage() {
        let config = json!({"plan_path": "docs/plans/2026-04-01-auth.md"});
        let md = generate_claude_md("implement", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("Implementer"));
        assert!(content.contains("docs/plans/2026-04-01-auth.md"));
    }

    #[test]
    fn test_generate_review_stage() {
        let config = json!({"pr_url": "https://github.com/org/repo/pull/42"});
        let md = generate_claude_md("review", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("Reviewer"));
        assert!(content.contains("pull/42"));
    }

    #[test]
    fn test_generate_implement_stage_without_plan_path() {
        let config = json!({});
        let md = generate_claude_md("implement", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("plan path not provided"));
    }

    #[test]
    fn test_generate_review_stage_without_pr_url() {
        let config = json!({});
        let md = generate_claude_md("review", &config);
        assert!(md.is_some());
        let content = md.unwrap();
        assert!(content.contains("PR URL not provided"));
    }

    #[test]
    fn test_all_stages_include_do_not_merge() {
        let config = json!({"spec_path": "x", "plan_path": "x", "pr_url": "x"});
        for stage in ["spec", "plan", "implement", "review", "evolve"] {
            let content = generate_claude_md(stage, &config).unwrap();
            assert!(
                content.contains("Do NOT merge the PR"),
                "stage '{stage}' missing 'Do NOT merge' instruction"
            );
        }
    }

    #[test]
    fn test_review_stage_does_not_create_new_branch() {
        let config = json!({"pr_url": "https://github.com/org/repo/pull/42"});
        let content = generate_claude_md("review", &config).unwrap();
        assert!(
            !content.contains("Create a new branch"),
            "review stage should NOT tell drone to create a new branch"
        );
        assert!(
            content.contains("Do NOT create a new branch"),
            "review stage should explicitly say not to create a new branch"
        );
    }

    #[test]
    fn test_unknown_stage_returns_none() {
        let config = json!({});
        assert!(generate_claude_md("unknown", &config).is_none());
        assert!(generate_claude_md("", &config).is_none());
    }
}
