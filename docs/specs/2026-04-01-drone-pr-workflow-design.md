# Drone PR Workflow

**Date:** 2026-04-01
**Roadmap item:** #5 — Drone PR workflow

## Context

Drones run Claude Code to complete tasks, but the output currently stops at "work done in a temp workspace." For the dev loop to function, completed work needs to land as a PR that an operator can review and merge. The drone also needs GitHub credentials to push, BuildBuddy keys for remote cache, and the session transcript needs to be stored efficiently.

## Changes

### 1. PR Workflow via CLAUDE.md + Drone Safety Net

Claude Code handles the git workflow natively — it already knows how to branch, commit, push, and create PRs. The drone provides instructions and a verification backstop.

**Drone's embedded `config/CLAUDE.md`:**
- Instructs Claude Code to create a branch, commit changes with descriptive messages, push, and create a PR with a summary
- Does not specify branch naming — Claude Code picks an appropriate name

**Drone post-execute verification (in `execute()`, after Claude Code exits):**
1. Check `git status` in workspace — if uncommitted changes exist, commit and push them
2. Check `gh pr view` — if no PR exists for the current branch, run `gh pr create` with a generic title derived from the task description
3. Collect `GitRefs` (existing `collect_git_refs()`)
4. If `git_refs.pr_url` is still `None`, the drone reports the result with exit code indicating failure

**Queen-side enforcement:**
When processing `DroneOutput`, if `git_refs.pr_url` is `None` and the exit code was 0, Queen overrides the status to `failed` with error "no PR created." A successful drone run requires a PR.

### 2. Secrets via Job Config

Job definitions carry secrets that drones need for authenticated operations. These flow through `JobSpec.config` which Queen already forwards to the drone.

**Config schema:**
```json
{
  "repo_url": "...",
  "task": "...",
  "drone_type": "claude-drone",
  "secrets": {
    "github_pat": "ghp_...",
    "buildbuddy_api_key": "..."
  }
}
```

**Drone setup reads `config.secrets` and:**
1. Writes `~/.config/gh/hosts.yml` with the GitHub PAT for `gh` CLI auth
2. Configures a git credential helper that returns the PAT for HTTPS push operations
3. Sets `BUCK2_RE_HTTP_HEADERS=x-buildbuddy-api-key:<key>` in the environment passed to Claude Code

The `kerrigan` CLI can pass secrets at submit time via `--set secrets.github_pat=ghp_...` which flows through `config_overrides`.

### 3. Session Artifact Compression

The full session transcript is the drone's primary artifact. Queen already stores it via Overseer's artifact API. The change: compress before storing.

**Queen supervisor change:**
- After receiving `DroneOutput`, gzip the conversation JSON bytes
- Store via `nydus.store_artifact()` with:
  - Name: `{run_id}-conversation.jsonl.gz`
  - Content type: `application/gzip`
  - Run ID: the job run ID

This replaces the current uncompressed `{run_id}-conversation.json` storage. One dependency addition: `flate2` for gzip compression.

### 4. Overseer MCP in Drone Config

The drone's embedded `config/settings.json` configures Overseer as an MCP server available to Claude Code during execution.

**Transport:** HTTP (not stdio — stdio is used by the harness protocol between drone and Queen).

**URL:** Read from `JobSpec.config` (same Overseer URL Queen uses).

**Available tools during execution:**
- `store_artifact` — Claude Code can store intermediate artifacts
- `log_decision` — Claude Code can log design decisions with reasoning
- `store_memory` / `search_memory` — Claude Code can use Overseer's semantic memory

This is configured in `settings.json` as an MCP server entry. The drone setup writes the actual URL into the config after reading it from `JobSpec.config`.

## Out of Scope

- Job templates / pre-defined definitions (roadmap #6)
- Job chaining / stage progression (roadmap #7)
- Branch naming conventions
- PR review automation
- Drone subtypes (spec-writer, implementer, reviewer)
