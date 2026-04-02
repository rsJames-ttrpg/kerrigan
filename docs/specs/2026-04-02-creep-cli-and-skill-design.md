# Creep CLI + Discovery Skill Design

## Overview

Creep v1 is built and running (gRPC file-indexing sidecar) but has zero consumers. This spec adds three pieces to make Creep useful in the dev loop:

1. **`creep-cli`** — thin CLI wrapping Creep's gRPC API
2. **`creep-discovery` skill** — Claude Code plugin teaching drones to use the CLI
3. **Drone hooks** — auto-register/unregister workspaces on drone setup/teardown

The CLI is the integration surface. Skills call it via Bash. Drones hook into it for workspace lifecycle. Humans use it for debugging. No MCP discovery problems — Claude uses it because the skill tells it to.

## Component 1: `creep-cli` Crate

### Location

`src/creep-cli/` — new workspace member crate.

### Commands

Four subcommands, 1:1 with Creep's gRPC RPCs:

```
creep-cli search <pattern> [--workspace <path>] [--type <ext>]
creep-cli metadata <path>
creep-cli register <path>
creep-cli unregister <path>
```

### Behavior

- **Connection:** `localhost:9090` by default. `--addr <host:port>` override.
- **Output:** Human-readable table by default. `--json` flag for machine-parseable output (skills use this).
- **Errors:** Non-zero exit code on gRPC failure, connection refused, invalid args. Stderr for errors, stdout for results.

### `search` output (human)

```
src/overseer/src/main.rs          4.2 KB  2026-04-02T10:30:00  rust  abc123...
src/overseer/src/db/sqlite.rs     8.1 KB  2026-04-01T15:20:00  rust  def456...
```

### `search` output (`--json`)

```json
[
  {
    "path": "src/overseer/src/main.rs",
    "size": 4200,
    "modified_at": 1743580200,
    "file_type": "rust",
    "content_hash": "abc123..."
  }
]
```

### `register` output

```
Indexed 342 files in /tmp/drone-abc123/workspace
```

### Dependencies

- `clap` (arg parsing)
- `tonic` (gRPC client, generated from creep's proto)
- `tokio` (async runtime)
- `serde_json` (JSON output)
- `prost` (protobuf types)

### Build

```
# BUCK target
rust_binary(
    name = "creep-cli",
    ...
    deps = ["//third-party:clap", "//third-party:tonic", ...]
)
```

Proto codegen: same pattern as creep server — `build.rs` for Cargo, `proto_gen/` for Buck2.
The CLI crate duplicates `proto_gen/` from the creep crate (one small generated file). Not worth a shared proto crate for four message types.

### Container

Staged by `build.sh`, copied to `/opt/kerrigan/bin/creep-cli` in the Dockerfile. On PATH via the entrypoint.

## Component 2: `creep-discovery` Skill

### Location

`src/drones/claude/plugins/creep-discovery/` — a Claude Code plugin in the repo.

### Structure

```
src/drones/claude/plugins/creep-discovery/
  package.json
  skills/
    creep-discovery/
      SKILL.md
```

### `package.json`

```json
{
  "name": "creep-discovery",
  "version": "0.1.0"
}
```

### `SKILL.md`

Frontmatter:

```yaml
---
name: creep-discovery
description: "Use when exploring or navigating a codebase — find files by pattern, check file metadata, understand workspace structure. Faster than glob/grep for indexed workspaces."
---
```

Body teaches Claude:

- **When to use:** Exploring an indexed workspace. The drone's setup hook registers the workspace automatically, so Creep already knows about the files.
- **Commands:**
  - `creep-cli search "*.rs" --json` — find all Rust files
  - `creep-cli search "test_*" --type rust --json` — find test files
  - `creep-cli metadata src/overseer/src/main.rs` — check a specific file's size, hash, type
- **When to fall back:** If `creep-cli` fails (Creep not running, workspace not registered), fall back to `Glob`/`Grep` tools. Don't block on Creep being unavailable.
- **JSON output:** Always use `--json` flag when parsing output programmatically.

### Buck2 Targets

In `src/drones/claude/plugins/BUCK`:

```python
# Package the skill directory
filegroup(
    name = "creep-discovery",
    srcs = glob(["creep-discovery/**"]),
    visibility = ["PUBLIC"],
)

# Install to ~/.claude/plugins/ for local use
sh_binary(
    name = "install",
    main = "install.sh",
)
```

### Local Use

`buck2 run root//src/drones/claude/plugins:install` installs the skill for the operator's own Claude Code sessions. Same skill, same CLI, works locally as long as Creep is running.

## Component 3: Drone Hooks

### Workspace Registration

In the claude-drone's `DroneRunner::setup()` implementation (after `clone_repo` succeeds):

```rust
// Best-effort: register workspace with Creep for fast file discovery
let status = Command::new("creep-cli")
    .args(["register", workspace.to_str().unwrap()])
    .status()
    .await;
if let Err(e) = status {
    tracing::warn!("failed to register workspace with Creep: {e}");
}
```

### Workspace Unregistration

In `DroneRunner::teardown()` (before `cleanup` removes the temp dir):

```rust
// Best-effort: unregister workspace from Creep
let _ = Command::new("creep-cli")
    .args(["unregister", workspace.to_str().unwrap()])
    .status()
    .await;
```

Both are best-effort — if Creep is down, the drone still works. The skill's fallback instructions handle the case where Creep isn't available.

### Skill Bundling in Drone Environment

The drone's `environment.rs` gets a new function to set up the plugin directory.
Called during `create_home()` (or immediately after it in `setup()`), before the Claude CLI is invoked:

```rust
pub async fn install_plugins(home: &Path) -> Result<()> {
    let plugin_dir = home.join(".claude/plugins/creep-discovery");
    fs::create_dir_all(&plugin_dir).await?;
    // Copy from /opt/kerrigan/plugins/creep-discovery/ in the container
    copy_dir("/opt/kerrigan/plugins/creep-discovery", &plugin_dir).await?;
    Ok(())
}
```

The drone's HOME is set to `/tmp/drone-{id}/`, so `~/.claude/plugins/` resolves to the isolated home. Claude Code discovers plugins from this directory automatically.

## Build & Deploy Changes

### `deploy/dev/build.sh`

Add `creep-cli` to the build and staging:

```bash
buck2 build \
  root//src/overseer:overseer \
  root//src/queen:queen \
  root//src/creep:creep \
  root//src/creep-cli:creep-cli \
  root//src/drones/claude/base:claude-drone
```

Stage `creep-cli` to `.stage/bin/creep-cli` and the plugin files to `.stage/plugins/creep-discovery/`.

### `Dockerfile`

```dockerfile
COPY deploy/dev/.stage/bin/creep-cli    /opt/kerrigan/bin/creep-cli
COPY deploy/dev/.stage/plugins/         /opt/kerrigan/plugins/
```

### `entrypoint.sh`

Ensure `/opt/kerrigan/bin` is on PATH (may already be the case).

## What This Does NOT Cover

- **Creep v2 features** (tree-sitter, symbol index) — separate spec
- **MCP server for Creep** — the CLI + skill approach sidesteps this
- **Creep scaling** — single instance, single host, adequate for current workload
- **Skill for spec/plan stages** — only implement and review stages get the skill, since those navigate code
