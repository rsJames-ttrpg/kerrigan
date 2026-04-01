# Job Templates and Drone Subtypes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add stage-specific CLAUDE.md generation for the four dev loop stages (spec, plan, implement, review) and seed corresponding job definitions in Overseer.

**Architecture:** Single `claude-drone` binary reads `config.stage` during setup and generates stage-specific CLAUDE.md content via a new `generate_claude_md()` function. Overseer seeds four stage definitions on startup alongside the existing `default`.

**Tech Stack:** Rust 2024, serde_json

**Spec:** `docs/specs/2026-04-01-job-templates-subtypes-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|----------------|
| `src/drones/claude/base/src/stages.rs` | `generate_claude_md(stage, config)` — produces stage-specific CLAUDE.md content |

### Modified files

| File | Change |
|------|--------|
| `src/drones/claude/base/src/drone.rs` | Call `generate_claude_md` in setup based on `config.stage` |
| `src/drones/claude/base/src/main.rs` | Add `mod stages;` |
| `src/overseer/src/main.rs` | Seed 4 additional job definitions on startup |

---

## Task 1: Stage-specific CLAUDE.md generation

**Files:**
- Create: `src/drones/claude/base/src/stages.rs`
- Modify: `src/drones/claude/base/src/main.rs`

- [ ] **Step 1: Create `src/drones/claude/base/src/stages.rs`**

```rust
use serde_json::Value;

/// Base rules included in every stage's CLAUDE.md.
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

/// Generate stage-specific CLAUDE.md content.
/// Returns `None` if the stage is unknown — caller should use the embedded default.
pub fn generate_claude_md(stage: &str, config: &Value) -> Option<String> {
    match stage {
        "spec" => Some(generate_spec(config)),
        "plan" => Some(generate_plan(config)),
        "implement" => Some(generate_implement(config)),
        "review" => Some(generate_review(config)),
        _ => None,
    }
}

fn generate_spec(config: &Value) -> String {
    format!(
        r#"# Claude Drone — Spec Writer

You are a Claude Code drone tasked with writing a design specification.

## Your Task

Use the `/brainstorm` skill (superpowers:brainstorming) to turn the problem description
into a fully formed design spec. Follow the skill's process:

1. Understand the problem
2. Ask clarifying questions (answer them yourself based on the codebase)
3. Propose 2-3 approaches with trade-offs
4. Write the design spec

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

## Your Task

Read the design spec at `{spec_path}` and use the superpowers:writing-plans skill to create
a detailed implementation plan. The plan should be comprehensive enough that another
engineer (or drone) can implement it without additional context.

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

## Your Task

Read the implementation plan at `{plan_path}` and use the superpowers:subagent-driven-development
skill to execute it task by task. Follow TDD: write tests first, then implement.

Implement all tasks in the plan. Run tests after each task. Commit frequently.

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

## Your Task

The PR to review: {pr_url}

Check out the PR branch and use the pr-review-toolkit:review-pr skill to perform a
thorough code review. You have full access to the codebase — run tests, trace code paths,
check types, verify behavior.

Review for:
- Correctness and logic errors
- Security vulnerabilities
- Code quality and maintainability
- Test coverage
- Adherence to project conventions

Post your review feedback as PR comments using `gh`. Also store the review as an
Overseer artifact via MCP if available.

After review, create a PR with your review notes committed to the repo
(e.g., `docs/reviews/YYYY-MM-DD-pr-<number>.md`).

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
        assert!(content.contains("/brainstorm"));
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
    fn test_unknown_stage_returns_none() {
        let config = json!({});
        assert!(generate_claude_md("unknown", &config).is_none());
        assert!(generate_claude_md("", &config).is_none());
    }
}
```

- [ ] **Step 2: Add module declaration**

In `src/drones/claude/base/src/main.rs`, add `mod stages;` after the existing module declarations:

```rust
mod drone;
mod environment;
mod stages;
```

- [ ] **Step 3: Verify tests pass**

Run: `cd src/drones/claude/base && cargo test`

Note: The binary target won't compile (missing `claude-cli`), but tests should work since `stages.rs` doesn't use `include_bytes!`. If cargo test fails due to the binary, try: `cargo test --lib` or `cargo test stages`

- [ ] **Step 4: Commit**

```bash
git add src/drones/claude/base/src/stages.rs src/drones/claude/base/src/main.rs
git commit -m "feat(drone): add stage-specific CLAUDE.md generation"
```

---

## Task 2: Wire stage dispatch into drone setup

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`

- [ ] **Step 1: Add stage dispatch to setup()**

In `src/drones/claude/base/src/drone.rs`, in the `setup` method, after `environment::write_task(...)` (line 36) and before the MCP URL configuration, add the stage dispatch:

```rust
        // Generate stage-specific CLAUDE.md if config.stage is set
        if let Some(stage) = job.config.get("stage").and_then(|v| v.as_str()) {
            if let Some(claude_md) = crate::stages::generate_claude_md(stage, &job.config) {
                tokio::fs::write(env.home.join("CLAUDE.md"), claude_md)
                    .await
                    .context("failed to write stage-specific CLAUDE.md")?;
                tracing::info!(stage = %stage, "generated stage-specific CLAUDE.md");
            }
        }
```

This goes after `write_task` and before `configure_mcp_url`. The full setup order becomes:
1. `create_home` — creates dirs, writes embedded defaults
2. `configure_github_auth` — secrets for clone
3. `clone_repo`
4. `write_task`
5. **Generate stage-specific CLAUDE.md** (overwrites embedded default if stage is known)
6. `configure_mcp_url`
7. `write_env_vars`

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p queen -p overseer -p drone-sdk`
Expected: compiles (claude-drone won't compile with cargo due to missing claude-cli, but the other crates verify our types are consistent)

- [ ] **Step 3: Commit**

```bash
git add src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): wire stage dispatch into setup — generate CLAUDE.md per config.stage"
```

---

## Task 3: Seed stage job definitions in Overseer

**Files:**
- Modify: `src/overseer/src/main.rs`

- [ ] **Step 1: Add stage definitions to the seeding block**

In `src/overseer/src/main.rs`, replace the current seeding block (lines 87-101):

```rust
    // Seed default job definition if it doesn't exist
    let existing = state.jobs.list_job_definitions().await?;
    if !existing.iter().any(|d| d.name == "default") {
        state
            .jobs
            .create_job_definition(
                "default",
                "Default job definition for ad-hoc tasks",
                serde_json::json!({
                    "drone_type": "claude-drone"
                }),
            )
            .await?;
        tracing::info!("seeded default job definition");
    }
```

With:

```rust
    // Seed job definitions if they don't exist
    let existing = state.jobs.list_job_definitions().await?;
    let existing_names: std::collections::HashSet<&str> =
        existing.iter().map(|d| d.name.as_str()).collect();

    let seed_definitions = [
        (
            "default",
            "Default job definition for ad-hoc tasks",
            serde_json::json!({ "drone_type": "claude-drone" }),
        ),
        (
            "spec-from-problem",
            "Generate a design spec from a problem description",
            serde_json::json!({ "drone_type": "claude-drone", "stage": "spec" }),
        ),
        (
            "plan-from-spec",
            "Write an implementation plan from a spec",
            serde_json::json!({ "drone_type": "claude-drone", "stage": "plan" }),
        ),
        (
            "implement-from-plan",
            "Implement code from an implementation plan",
            serde_json::json!({ "drone_type": "claude-drone", "stage": "implement" }),
        ),
        (
            "review-pr",
            "Review a pull request",
            serde_json::json!({ "drone_type": "claude-drone", "stage": "review" }),
        ),
    ];

    for (name, description, config) in seed_definitions {
        if !existing_names.contains(name) {
            state
                .jobs
                .create_job_definition(name, description, config)
                .await?;
            tracing::info!("seeded job definition: {name}");
        }
    }
```

- [ ] **Step 2: Verify compilation and tests**

Run: `cargo check -p overseer && cargo test -p overseer`
Expected: compiles, all 75 tests pass

- [ ] **Step 3: Commit**

```bash
git add src/overseer/src/main.rs
git commit -m "feat(overseer): seed stage job definitions on startup"
```

---

## Task 4: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Run all tests**

Run: `cargo test -p overseer -p queen -p drone-sdk`
Expected: all pass

- [ ] **Step 2: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass

- [ ] **Step 3: Verify CLI shows definitions**

Start Overseer: `buck2 run root//src/overseer:overseer` (in another terminal)

Run: `curl -s http://localhost:3100/api/jobs/definitions | python3 -m json.tool`
Expected: 5 definitions listed: default, spec-from-problem, plan-from-spec, implement-from-plan, review-pr

- [ ] **Step 4: Verify submit with stage definition**

Run: `kerrigan submit "test spec stage" --definition spec-from-problem --set repo_url=https://github.com/rsJames-ttrpg/kerrigan.git --set secrets.github_pat=<PAT>`
Expected: run starts, drone spawns, uses spec-stage CLAUDE.md
