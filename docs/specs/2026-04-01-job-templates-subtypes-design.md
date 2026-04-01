# Job Templates and Drone Subtypes

**Date:** 2026-04-01
**Roadmap item:** #6 — Job templates for dev stages

## Context

The dev loop has four stages: spec → plan → implement → review. Each stage needs different Claude Code instructions telling it which skill to invoke. Currently there's a single `default` job definition and one generic CLAUDE.md. This spec adds stage-specific job definitions and dynamic CLAUDE.md generation.

## Approach

Single `claude-drone` binary. During setup, the drone reads `config.stage` from the job config and generates stage-specific CLAUDE.md content. No new binaries, no protocol changes.

## Stages

### spec (spec-from-problem)

Invokes `superpowers:brainstorming` skill. Takes a problem description as the task string. Outputs a design spec committed to `docs/specs/`, pushed as a PR. Also stores the spec as an Overseer artifact via MCP.

**Config inputs:** `task` (problem description), `repo_url`, `secrets`

### plan (plan-from-spec)

Invokes `superpowers:writing-plans` skill. Reads the spec file from the repo. Outputs an implementation plan committed to `docs/plans/`, pushed as a PR. Also stores the plan as an Overseer artifact via MCP.

**Config inputs:** `task` (instruction), `repo_url`, `spec_path`, `secrets`

### implement (implement-from-plan)

Invokes `superpowers:subagent-driven-development` skill. Reads the plan file from the repo. Implements the code, runs tests, creates a PR. This is the heaviest stage — full permissions, long timeout.

**Config inputs:** `task` (instruction), `repo_url`, `plan_path`, `secrets`

### review (review-pr)

Invokes `pr-review-toolkit:review-pr` skill. Clones the repo, checks out the PR branch for full code introspection (running tests, tracing code paths, checking types). Outputs review feedback stored as an Overseer artifact via MCP and posted as PR comments.

**Config inputs:** `task` (instruction), `repo_url`, `pr_url`, `secrets`

## Stage Dispatch

In `environment.rs`, a new function:

```
generate_claude_md(stage: &str, config: &serde_json::Value) -> String
```

Produces stage-specific CLAUDE.md content. The config is available so the function can embed relevant fields (spec_path, plan_path, pr_url) directly in the instructions.

In `drone.rs` `setup()`, after creating the home directory:
1. Read `config.stage` (defaults to no stage — uses embedded default CLAUDE.md)
2. If a known stage, call `generate_claude_md()` and overwrite `{home}/CLAUDE.md`

## CLAUDE.md Content Per Stage

Each stage's CLAUDE.md includes:
- Base rules (focus on task, commit frequently, don't modify files outside scope)
- Git workflow (branch, commit, push, create PR)
- The specific skill to invoke and how to use it
- What to do with the output (commit + store as artifact)
- Stage-specific context extracted from the job config

## Job Definitions

Overseer seeds these definitions on startup (in addition to `default`):

| Name | `stage` | Description |
|---|---|---|
| `spec-from-problem` | `spec` | Generate a design spec from a problem description |
| `plan-from-spec` | `plan` | Write an implementation plan from a spec |
| `implement-from-plan` | `implement` | Implement code from a plan |
| `review-pr` | `review` | Review a pull request |

Each definition's config contains `stage` and `drone_type: "claude-drone"`. Other fields (`repo_url`, `task`, `secrets`, `spec_path`, etc.) come from config overrides at submit time.

## Handoff Between Stages

Output from each stage is both:
1. **Committed to the repo** — the next stage clones and reads it
2. **Stored as an Overseer artifact** — for traceability and the Evolution Chamber

Human gates (approval between spec→plan and plan→implement) are a chaining concern (#7), not a drone concern. Drones complete normally. The orchestration layer decides when to advance.

## Usage

```bash
# Spec a problem
kerrigan submit "fix the auth timeout bug" --definition spec-from-problem \
  --set repo_url=https://github.com/org/repo.git \
  --set secrets.github_pat=ghp_...

# Plan from a spec (after spec is approved)
kerrigan submit "write implementation plan" --definition plan-from-spec \
  --set repo_url=https://github.com/org/repo.git \
  --set spec_path=docs/specs/2026-04-01-auth-timeout-design.md \
  --set secrets.github_pat=ghp_...

# Implement from a plan (after plan is approved)
kerrigan submit "implement the plan" --definition implement-from-plan \
  --set repo_url=https://github.com/org/repo.git \
  --set plan_path=docs/plans/2026-04-01-auth-timeout.md \
  --set secrets.github_pat=ghp_...

# Review a PR
kerrigan submit "review this PR" --definition review-pr \
  --set repo_url=https://github.com/org/repo.git \
  --set pr_url=https://github.com/org/repo/pull/42 \
  --set secrets.github_pat=ghp_...
```

## Out of Scope

- Job chaining / automatic stage progression (#7)
- Human approval gates (part of #7)
- Model selection per stage (handled by skills internally)
- Drone subtypes as separate binaries
- Evolution Chamber analysis (#9)
