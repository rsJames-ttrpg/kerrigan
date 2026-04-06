# Drone Bash Reliability

Reduce the drone session bash failure rate from 19.1% by fixing the six root cause
categories identified across 17 sessions (362 bash calls, 69 failures).

## Background

Analysis of all drone session artifacts from 2026-04-05 through 2026-04-06 reveals
that bash tool call failures cluster into six categories:

| Category              | Count | Share | Root cause                                    |
|-----------------------|-------|-------|-----------------------------------------------|
| merge-conflict (hooks)| 13    | 19%   | Pre-commit hooks modify staged files, drone doesn't re-stage |
| git-identity-missing  | 8     | 12%   | No `user.name`/`user.email` configured        |
| git-push-before-pr    | 8     | 12%   | `gh pr create` without pushing first           |
| other/nonzero-exit    | 19    | 28%   | Mixed: compile errors, test failures (expected iterative dev) |
| wrong-cli-args        | 7     | 10%   | Hallucinated CLI flags, wrong cargo/gh syntax  |
| command-not-found     | 3     | 4%    | Toolchain wrappers not on PATH                 |

65% of failures are preventable through infrastructure and instruction fixes.
The remaining 35% (compile errors, test failures) are normal iterative development.

## Approach

Fix each error category where it originates. Infrastructure problems get
infrastructure fixes (drone binary setup). Instruction problems get instruction
fixes (CLAUDE.md templates). Repo-specific conventions are declared in `drone.toml`
so drones adapt to the repo they're working in rather than hardcoding assumptions.

## Change 1: `drone.toml` support for Claude drone

The native drone already reads `drone.toml` from the workspace root. Extend this
pattern to the Claude drone.

The Claude drone reads `drone.toml` from the workspace root after `git clone`.
If absent, all fields take defaults. The file is optional.

### Schema

```toml
[git]
# Shared git settings (used by all drone types)
default_branch = "main"        # default: "main"
branch_prefix = "feat/"        # default: "feat/"
auto_commit = true             # default: true
pr_on_complete = true          # default: true
protected_paths = []           # default: []

[git.identity.claude]
# Git identity for Claude drones
user_name = "claude-drone"     # default: "claude-drone"
user_email = "claude-drone@noreply"  # default: "claude-drone@noreply"

[git.identity.native]
# Git identity for native drones
user_name = "native-drone"     # default: "native-drone"
user_email = "native-drone@noreply"  # default: "native-drone@noreply"

[setup]
# Shell commands to run after clone, before the drone session starts.
# Each command runs in the workspace root with the drone's environment.
# Non-zero exit from any command is logged as a warning but does not
# abort the session (the drone should still attempt its task).
commands = [
    "./tools/setup-hooks.sh",
]

[prompts]
# Repo-specific instructions appended to the drone's CLAUDE.md.
# Use this for build system conventions, test commands, or any
# repo-specific rules that drones should follow.
extra_rules = """
## Build & Test

- Use `buck2 build root//src/<crate>:<crate>` to build, NOT `cargo build`
- Use `buck2 test root//src/<crate>:<crate>-test` to test, NOT `cargo test`
- `cargo check` and `cargo clippy` are OK for quick feedback
- Clippy CI-equivalent: `buck2 build 'root//src/<crate>:<crate>[clippy.txt]'`
- Run `buck2 targets root//...` to discover available targets
"""
```

### Drone binary behaviour

1. After `git clone`, check for `drone.toml` in workspace root.
2. Parse with serde/toml. Missing file means all defaults.
3. Each drone binary knows its own type string (`"claude"` or `"native"`).
4. Look up `git.identity.<type>` for identity fields. Fall back to hardcoded
   defaults if the section is missing.
5. Run `[setup].commands` sequentially. Log stdout/stderr. Warn on non-zero
   exit but continue.
6. Append `[prompts].extra_rules` to the generated CLAUDE.md.

### Shared config struct

Define a `DroneToml` struct in `drone-sdk` for the shared `drone.toml` schema
(`[git]`, `[git.identity.*]`, `[setup]`, `[prompts]`). Both drone binaries
use this struct to parse the shared sections.

The native drone keeps its own `DroneConfig` for native-specific fields
(`provider`, `runtime`, `cache`, `tools`, `mcp`, `orchestrator`, `health_checks`).
It can compose `DroneToml` alongside `DroneConfig` or flatten them — that's an
implementation detail left to the plan. The native drone's existing `GitSection`
fields (`default_branch`, `branch_prefix`, etc.) move into the shared struct
since they apply to both drone types.

## Change 2: Git identity from `drone.toml`

Each drone binary reads `git.identity.<type>` from `drone.toml` and writes
the identity to `.gitconfig` during setup, alongside the existing credential
helper configuration.

```gitconfig
[user]
    name = claude-drone
    email = claude-drone@noreply
[credential]
    helper = store
```

If `drone.toml` is absent or has no identity section for the drone's type,
the drone uses its hardcoded default:

- Claude drone: `claude-drone` / `claude-drone@noreply`
- Native drone: `native-drone` / `native-drone@noreply`

### Where it happens

Claude drone: `environment.rs` `configure_github_auth()` — append `[user]`
section to the `.gitconfig` it already writes.

Native drone: equivalent setup phase, reading from shared `DroneToml`.

**Eliminates:** git-identity-missing (12%, 8 occurrences)

## Change 3: Post-clone setup commands

The `[setup].commands` list runs after clone, before the drone session starts.
Commands execute sequentially in the workspace root with the drone's environment
(including `$HOME` set to the temp home).

For the kerrigan repo, this is `./tools/setup-hooks.sh`, which:
- Installs pre-commit hooks via prek (if buck2 is available)
- Sets up hermetic toolchain wrappers in `~/.local/bin/`
- Gracefully exits 0 if buck2 isn't on PATH

### Error handling

Non-zero exit from a setup command logs a warning via the drone channel but
does not abort the session. Rationale: a missing toolchain is bad but the
drone should still attempt its task — some work is better than no work.
Setup command stdout/stderr is captured and included in the session log.

### Timeout

Each setup command gets a 5-minute timeout. If exceeded, the command is killed
and the drone logs a warning and continues.

**Eliminates:** command-not-found (4%, 3 occurrences)

## Change 4: Repo-specific prompts

The `[prompts].extra_rules` string is appended verbatim to the generated
CLAUDE.md after the stage-specific content and before the base rules.

Order in the final CLAUDE.md:
1. Stage-specific instructions (from `stages.rs`)
2. Repo-specific rules (from `drone.toml` `[prompts].extra_rules`)
3. Base rules (from `config/CLAUDE.md`)

This gives repo-specific rules higher priority than base rules (Claude
weighs earlier content more) while stage instructions remain highest priority.

For kerrigan, the extra rules teach drones to use `buck2 build`/`buck2 test`
instead of `cargo build`/`cargo test`, and provide the target naming convention.

**Reduces:** wrong-cli-args (4%), gh-cli-error (6%)

## Change 5: Base CLAUDE.md improvements

Update `src/drones/claude/base/src/config/CLAUDE.md` with three additions:

### Pre-commit hook handling

```markdown
## Pre-commit Hooks

This repo may use pre-commit hooks that auto-fix files (trailing whitespace,
end-of-file newlines, formatting). When a commit fails because hooks modified files:

1. Run `git add -u` to re-stage the modified files
2. Run `git commit` again with the same message
3. Do NOT use `--no-verify` to skip hooks
```

### Push-before-PR rule

Add to the existing git workflow section:

```markdown
ALWAYS run `git push -u origin HEAD` before `gh pr create`. The PR command
will fail if you haven't pushed.
```

### CLI uncertainty rule

```markdown
## CLI Usage

When unsure about a command's flags or arguments, run `<command> --help` first
rather than guessing. Common mistakes to avoid:
- `gh pr diff` has no `--stat` flag
- `cargo test` accepts only ONE test name filter as a positional argument
```

**Reduces:** merge-conflict/hook failures (19%), git-push-before-pr (12%)

## What this spec does NOT cover

- **Compile errors, test failures, clippy warnings** (28% of errors): These are
  normal iterative development. Drones are expected to encounter and fix these.
- **Permission denied** (1%): Container configuration issue, not a drone problem.
- **Native drone's existing config migration**: The shared `DroneToml` struct
  should be designed so the native drone can adopt it, but the actual migration
  of native drone config is out of scope.

## Expected impact

| Category              | Before | After (expected) |
|-----------------------|--------|------------------|
| git-identity-missing  | 8      | 0 (deterministic fix) |
| command-not-found     | 3      | 0 (deterministic fix) |
| git-push-before-pr    | 8      | ~2 (instruction, not guaranteed) |
| merge-conflict (hooks)| 13     | ~3 (instruction, not guaranteed) |
| wrong-cli-args        | 7      | ~3 (repo prompts help, hallucination remains) |
| normal dev errors     | 19     | 19 (unchanged, expected) |
| **Total**             | **69** | **~27** |
| **Error rate**        | **19.1%** | **~7-8%** |

Conservative estimate: error rate drops from 19.1% to 7-8%. The deterministic
infrastructure fixes (identity, toolchain) go to zero. The instruction fixes
are probabilistic — they significantly reduce but won't eliminate the categories.

## Testing

- **Unit tests**: Parse `drone.toml` with all sections, missing sections,
  missing file. Verify identity lookup by drone type with fallback.
- **Integration test**: Clone a repo with `drone.toml`, verify `.gitconfig`
  has correct identity, verify setup commands ran, verify CLAUDE.md contains
  extra_rules content.
- **Session analysis**: After deployment, re-run the session analysis script
  on new sessions to measure actual error rate reduction.
