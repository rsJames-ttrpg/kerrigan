# Drone Config and Behaviour Design

## Overview

This spec defines how Claude Code drones are configured for autonomous development work. It covers the Claude CLI bundling, invocation pattern, permission model, per-stage configs, plugin selection, MCP servers, and instructions.

## Claude CLI Bundling

Drones are self-contained portable units. The Claude CLI binary is bundled into the drone binary at build time.

**Build-time flow:**
1. Parse `https://claude.ai/install.sh` to find the binary download URL for the target platform
2. Buck2 fetches the Claude CLI binary as a hermetic tool (pinned version + SHA256)
3. The drone binary embeds it via `include_bytes!` or Buck2 resources
4. At runtime, the drone extracts the CLI to its temp home and runs it from there

No network dependency at runtime. No assumption that `claude` is on PATH.

## Invocation Pattern

Every drone stage uses the same CLI pattern:

```
{home}/claude --print \
  --output-format stream-json \
  --dangerously-skip-permissions \
  --settings {home}/.claude/settings.json \
  --append-system-prompt-file {home}/drone-instructions.md \
  --mcp-config {home}/mcp.json \
  --plugin-dir {home}/plugins/ \
  "{task prompt from job spec}"
```

The drone binary sets up `{home}`, extracts all assets (including the Claude CLI itself), clones the repo, then spawns this command. Project-level CLAUDE.md is discovered naturally from the repo — the drone does NOT use `--bare`.

## Permission Model

`--dangerously-skip-permissions` for all drones. Safety comes from:
- Isolated temp home directory per session
- Pre-commit hooks in the repo enforce code quality (fmt, clippy, tests)
- Queen enforces timeouts and kills runaway drones
- Initial testing in a Docker container for validation

## Drone Subtypes

Four stage-specific drones, each a separate binary with different embedded configs:

### `claude/spec-writer`
- **Task:** Takes a problem description, produces a design spec
- **Plugins:** superpowers (brainstorming, writing-plans)
- **Instructions:** Read the problem, explore the codebase, write a spec, commit, create PR
- **Output:** PR with spec document

### `claude/implementer`
- **Task:** Takes a plan, produces code + tests
- **Plugins:** superpowers (TDD, executing-plans, subagent-driven-development)
- **Instructions:** Follow the plan exactly, use TDD, commit after each task, create PR
- **Output:** PR with implementation

### `claude/reviewer`
- **Task:** Takes a PR, reviews it
- **Plugins:** pr-review-toolkit, code-review
- **Instructions:** Review the PR against the spec, check for bugs/style/coverage, post review comments
- **Output:** PR review (GitHub comments)

### `claude/base`
- **Task:** General purpose fallback
- **Plugins:** All
- **Instructions:** Generic development assistant
- **Output:** Varies

## Shared Config

### settings.json (common)

```json
{
  "permissions": {
    "allow": ["Read", "Write", "Edit", "Bash", "Glob", "Grep"]
  },
  "model": "sonnet"
}
```

### mcp.json (common)

```json
{
  "mcpServers": {
    "overseer": {
      "type": "http",
      "url": "${OVERSEER_URL}/mcp"
    }
  }
}
```

Overseer URL injected via environment variable at runtime. Creep added when available as a second MCP server.

### Pre-commit enforcement

Drones inherit the repo's existing pre-commit hooks (`prek.toml`):
- cargo fmt
- clippy
- cargo test
- reindeer sync check

These run on every commit the drone makes. The drone cannot skip them (no `--no-verify`).

## Per-Stage Instructions

### spec-writer/drone-instructions.md

```markdown
You are a spec-writer drone. Your job is to produce a design specification.

## Workflow
1. Read the problem description provided as your task
2. Explore the codebase to understand existing patterns and architecture
3. Read relevant existing specs in docs/specs/ for style reference
4. Write the spec to docs/specs/YYYY-MM-DD-<topic>-design.md
5. Create a branch: feat/spec-<topic>
6. Commit the spec
7. Push and create a PR with the spec for review

## Constraints
- Do NOT write any code
- Do NOT modify existing files except docs/
- Focus on clarity and completeness
- Include architecture, components, data flow, error handling
- Reference existing patterns in the codebase
```

### implementer/drone-instructions.md

```markdown
You are an implementer drone. Your job is to implement a plan.

## Workflow
1. Read the implementation plan provided as your task
2. Create a branch: feat/<feature-name>
3. Follow the plan task by task, in order
4. For each task:
   a. Write the failing test
   b. Run tests to verify it fails
   c. Write minimal implementation to pass
   d. Run tests to verify it passes
   e. Commit with a descriptive message
5. After all tasks, push and create a PR

## Constraints
- Follow TDD strictly — test before implementation
- Commit after each task, not in bulk
- Do NOT deviate from the plan
- If the plan is wrong or unclear, stop and report the issue
- Do NOT add features not in the plan
```

### reviewer/drone-instructions.md

```markdown
You are a reviewer drone. Your job is to review a pull request.

## Workflow
1. Read the PR diff
2. Read the spec/plan that the PR implements
3. Check:
   - Does the code match the spec?
   - Are there bugs or logic errors?
   - Is error handling adequate?
   - Are tests covering the important paths?
   - Does it follow project patterns (check CLAUDE.md)?
4. Post a review on the PR with findings

## Constraints
- Do NOT modify code
- Be specific — reference file:line
- Categorise issues as Critical, Important, or Suggestion
- Approve if no Critical issues remain
```

## Repo Structure

```
src/drones/
  claude/
    base/           # General purpose (existing skeleton)
    spec-writer/    # Spec writing drone
    implementer/    # Implementation drone
    reviewer/       # PR review drone
    shared/         # Shared assets (settings.json, mcp.json)
```

Each subtype directory contains:
```
src/
  main.rs           # harness::run(StageDrone)
  drone.rs          # DroneRunner impl
config/
  settings.json     # Shared or overridden
  drone-instructions.md  # Stage-specific
  mcp.json          # Shared
plugins/            # Stage-specific plugin selection
```

The `shared/` directory holds common config that all subtypes reference. Build-time composition: each drone binary embeds shared + stage-specific assets.

## Docker Testing Environment

For initial validation, a Dockerfile that:
1. Starts from a clean Linux image
2. Contains no pre-installed Claude CLI (verifies bundling works)
3. Has git, gh CLI, and basic dev tools
4. Mounts the target repo
5. Runs the drone binary with a test job spec

This validates the drone is truly self-contained before testing on real repos.

## What This Does NOT Cover

- Auth flow (drone uses `--dangerously-skip-permissions`, auth handled by pre-existing credentials or host symlinks)
- Creep MCP integration (added when Creep is ready)
- Evolution Chamber feedback (future — needs real drone output first)
- Job chaining between stages (separate spec)
