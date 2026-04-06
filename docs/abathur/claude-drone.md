---
title: Claude Drone
slug: claude-drone
description: Self-extracting drone binary — embeds Claude Code config, stage-specific prompts, plugin system
lastmod: 2026-04-06
tags: [drone, claude, stages, plugins]
sources:
  - path: src/drones/claude/base/src/drone.rs
    hash: 29173bd360314f1e75882bd9c6456df864faac2513a562e6d55211a3e3046147
  - path: src/drones/claude/base/src/environment.rs
    hash: c9fb307685a72aa30f7f16143bd828761bb04327bfbdaa8ea2005553aa666361
  - path: src/drones/claude/base/src/stages.rs
    hash: ccaf8af9edeaf1cabf86d84b250bbe41bb2ebd674686da5e0a7c55719963da8c
sections: [overview, setup-phase, execute-phase, stages, pr-safety-net, secret-handling, embedded-config, file-layout]
---

# Claude Drone

## Overview

Self-contained binary that embeds Claude Code CLI, config files, and plugins at compile time via `include_bytes!()`. Each invocation creates an isolated `/tmp/drone-{job_run_id}/` home, clones the target repo, spawns Claude Code with stage-specific instructions, and ensures a PR is created.

Implements `DroneRunner` from `drone-sdk`.

## Setup Phase

1. **Create isolated home** — `/tmp/drone-{id}/` with `.claude/` subdirectories. Job run ID validated (alphanumeric + `-_` only) to prevent path traversal.
2. **Configure secrets** — GitHub PAT written to `.git-credentials` and `.config/gh/hosts.yml` BEFORE clone (so HTTPS auth works).
3. **Clone repo** — `git clone --depth 1`, respects branch from job config.
4. **Write task** — Task text saved to `{home}/.task`.
5. **Generate stage CLAUDE.md** — If `config.stage` is set, overwrites base CLAUDE.md with stage-specific instructions. Stage persisted to `{home}/.stage`.
6. **Configure MCP** — Rewrites `OVERSEER_MCP_URL_PLACEHOLDER` in settings.json with actual Overseer URL.
7. **Environment vars** — Extracts `buildbuddy_api_key` → `BUCK2_RE_HTTP_HEADERS`, writes to `.drone-env`.
8. **Install plugins** — Extracts `drone-plugins.tar`, generates `installed_plugins.json` manifest for 6 vendored plugins.
9. **Register with Creep** — Best-effort `creep-cli register {workspace}` for fast file discovery.

## Execute Phase

1. **Auth check** — If `.credentials.json` missing, runs `claude auth login --console`. Detects auth URLs from stderr, relays to Queen via channel, receives code back. 10-minute timeout.
2. **Spawn Claude CLI:**
   ```
   claude --print --output-format json \
     --dangerously-skip-permissions \
     --settings {settings.json} \
     --append-system-prompt-file {CLAUDE.md}
   ```
   Task piped to stdin. Working directory: workspace. PATH prepended with `.local/bin` (hermetic toolchain). Extra env vars loaded from `.drone-env`.
3. **I/O monitoring** — Stderr watched for auth URLs. Stdout collected as JSON conversation.
4. **Timeout** — 2 hours (7200s).
5. **Post-execution** — Parse conversation JSON, collect session JSONL (gzip + base64), run PR safety net.

## Stages

`generate_claude_md(stage, config) -> Option<String>` produces stage-specific system prompts:

| Stage | Skill Used | Output | PR? |
|-------|-----------|--------|-----|
| `spec` | `/superpowers:brainstorming` | `docs/specs/YYYY-MM-DD-*.md` + artifact | Yes |
| `plan` | `/superpowers:writing-plans` | `docs/plans/YYYY-MM-DD-*.md` + artifact | Yes |
| `implement` | `/superpowers:subagent-driven-development` | Code changes | Yes |
| `review` | `/pr-review-toolkit:review-pr` | Review comments + fixes on existing PR | No (existing) |
| `evolve` | (none) | GitHub issues via `gh issue create` | No |

All stages except `evolve` include base rules: focus on assigned task, commit frequently, don't modify system config. All stages except `evolve` instruct "Do NOT merge the PR."

Config values interpolated: `spec_path` (plan stage), `plan_path` (implement stage), `pr_url` (review stage).

## PR Safety Net

Runs after execute for all stages except `evolve`:

1. Check for uncommitted changes → `git add -A && git commit`
2. Verify on non-default branch (not main/master)
3. `git push -u origin {branch}`
4. Check if PR exists via `gh pr view`
5. If no PR: `gh pr create` with task-derived title (60 char max)

Sets `pr_required = false` for evolve stage (only creates issues, not PRs).

## Secret Handling

| Secret | Source | Storage | Lifecycle |
|--------|--------|---------|-----------|
| GitHub PAT | `config.secrets.github_pat` | `.git-credentials` + `.config/gh/hosts.yml` | Setup → teardown (temp dir deleted) |
| Claude creds | `~/.claude/.credentials.json` | Symlinked into drone home | Never copied; symlink destroyed with temp |
| BuildBuddy key | `config.secrets.buildbuddy_api_key` | `.drone-env` as `BUCK2_RE_HTTP_HEADERS` | Setup → teardown |

## Embedded Config

Compiled into the binary via `include_bytes!()`:

- `config/settings.json` — MCP servers (Overseer placeholder), enabled plugins, effort level
- `config/CLAUDE.md` — Base system prompt (git workflow, don't merge PRs)
- `config/claude-cli` — Claude Code CLI binary
- `config/drone-plugins.tar` — 6 vendored plugins (pr-review-toolkit, superpowers, code-simplifier, claude-code-setup, feature-dev, creep-discovery)

## File Layout

```
/tmp/drone-{job_run_id}/
├── .claude/
│   ├── bin/claude              # embedded CLI (0o755)
│   ├── settings.json           # MCP + plugins config
│   ├── .credentials.json       # symlink → real creds
│   └── plugins/
│       ├── cache/{plugins}/    # extracted vendored plugins
│       └── installed_plugins.json
├── .config/gh/hosts.yml        # GitHub CLI auth (if PAT)
├── .gitconfig                  # credential helper
├── .git-credentials            # GitHub PAT (if provided)
├── .task                       # task description
├── .stage                      # current stage name
├── .drone-env                  # extra env vars
├── CLAUDE.md                   # system prompt (base or stage-specific)
└── workspace/{repo}/           # cloned repository
```
