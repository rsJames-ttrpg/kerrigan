---
title: Kerrigan CLI
slug: kerrigan-cli
description: Operator console — submit jobs, monitor pipelines, approve gates, manage credentials
lastmod: 2026-04-05
tags: [kerrigan, cli, operator, pipeline]
sources:
  - path: src/kerrigan/src/main.rs
    hash: ""
  - path: src/kerrigan/src/display.rs
    hash: ""
sections: [commands, pipeline-display, completions]
---

# Kerrigan CLI

## Commands

Global flag: `--url` (env: `KERRIGAN_URL`, default: `http://localhost:3100`).

**Job Submission & Control:**

| Command | Description |
|---------|-------------|
| `submit <problem>` | Submit task. `--definition`, `--branch`, `--set KEY=VALUE` (repeatable) |
| `status [run_id]` | Show run details or all runs with pipeline chain visualization |
| `approve <run_id>` | Advance job past pipeline gate |
| `reject <run_id>` | Fail job with `--message` |
| `watch <run_id>` | Poll until terminal state. `--interval` (default 3s) |
| `cancel <run_id>` | Cancel a running job |

**Output & Auth:**

| Command | Description |
|---------|-------------|
| `log <run_id>` | Display artifacts and tasks for a run |
| `auth <run_id> <code>` | Submit OAuth code for human approval gate |

**Infrastructure:**

| Command | Description |
|---------|-------------|
| `hatcheries [--status]` | List worker pools with capacity info |
| `artifacts list [--run] [--type] [--since]` | List artifacts with filters |
| `artifacts get <id>` | Fetch artifact; auto-decompresses gzip |
| `creds add --pattern <P> [--type] [--secret]` | Register credential (reads from stdin or `KERRIGAN_CRED_SECRET`) |
| `creds list` | Show all credentials (secrets redacted) |
| `creds rm <id>` | Delete credential by ID prefix |

**Analysis:**

| Command | Description |
|---------|-------------|
| `evolve [--since] [--min-sessions] [--submit] [--json]` | Run evolution analysis; optionally submit job |
| `completions <shell>` | Generate shell completions (bash, zsh, fish) |

All ID arguments support **prefix matching** — short 8-char IDs work. Errors on ambiguous prefix.

## Pipeline Display

`status` groups runs by `parent_id` relationships and renders pipeline chains:

```
Pipeline: "implement the auth system"
  spec-from-problem -> plan-from-spec -> implement-from-plan
    abc12345   spec-from-problem       completed
  > def67890   plan-from-spec          running
  ? ghi01234   implement-from-plan     pending
```

Markers: `>` running, `?` pending, blank otherwise. Status colors: completed=green, running=yellow, pending=cyan, failed=red, cancelled=dimmed. Color only on TTY.

## Completions

Three custom `ValueCompleter` implementations provide tab completion:

- **RunIdCompleter** — fetches `/api/jobs/runs`, displays status + definition name + task (truncated 35 chars)
- **ArtifactIdCompleter** — fetches `/api/artifacts`, displays name + type
- **DefinitionCompleter** — fetches `/api/jobs/definitions`, displays name + description
