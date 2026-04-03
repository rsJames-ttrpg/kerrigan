# Creep CLI + Discovery Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Creep useful in the dev loop by adding a thin CLI, a Claude Code skill for file discovery, and drone lifecycle hooks.

**Architecture:** `creep-cli` is a standalone Rust binary wrapping Creep's gRPC API via tonic-generated client. The `creep-discovery` skill is a Claude Code plugin that teaches Claude to use the CLI. The drone registers/unregisters workspaces and bundles the skill into its isolated home.

**Tech Stack:** Rust (edition 2024), tonic (gRPC client), clap (CLI), Buck2 (build), Claude Code plugin format (package.json + SKILL.md)

---

### Task 1: Scaffold `creep-cli` Crate

**Files:**
- Create: `src/creep-cli/Cargo.toml`
- Create: `src/creep-cli/src/main.rs`
- Create: `src/creep-cli/build.rs`
- Create: `src/creep-cli/proto/creep.proto` (copy from `src/creep/proto/creep.proto`)
- Create: `src/creep-cli/proto_gen/creep.v1.rs` (copy from `src/creep/proto_gen/creep.v1.rs`)
- Create: `src/creep-cli/BUCK`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create `src/creep-cli/Cargo.toml`**

```toml
[package]
name = "creep-cli"
version = "0.1.0"
edition = "2024"

[dependencies]
clap = { version = "4", features = ["derive"] }
tonic = "0.13"
prost = "0.13"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"

[build-dependencies]
tonic-build = "0.13"
```

- [ ] **Step 2: Copy proto files**

```bash
mkdir -p src/creep-cli/proto src/creep-cli/proto_gen
cp src/creep/proto/creep.proto src/creep-cli/proto/creep.proto
cp src/creep/proto_gen/creep.v1.rs src/creep-cli/proto_gen/creep.v1.rs
```

- [ ] **Step 3: Create `src/creep-cli/build.rs`**

Same pattern as creep server:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/creep.proto")?;
    Ok(())
}
```

- [ ] **Step 4: Create `src/creep-cli/src/main.rs` with CLI skeleton**

```rust
mod proto {
    tonic::include_proto!("creep.v1");
}

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use proto::file_index_client::FileIndexClient;

#[derive(Parser)]
#[command(name = "creep-cli", about = "CLI client for Creep file index")]
struct Cli {
    /// Creep server address
    #[arg(long, default_value = "http://localhost:9090", global = true)]
    addr: String,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for files by glob pattern
    Search {
        /// Glob pattern to match (e.g. "*.rs", "src/**/*.toml")
        pattern: String,
        /// Filter by workspace path
        #[arg(long)]
        workspace: Option<String>,
        /// Filter by file type (e.g. "rust", "python")
        #[arg(long, name = "type")]
        file_type: Option<String>,
    },
    /// Get metadata for a specific file
    Metadata {
        /// Absolute path to the file
        path: String,
    },
    /// Register a workspace for indexing
    Register {
        /// Absolute path to the workspace directory
        path: String,
    },
    /// Unregister a workspace
    Unregister {
        /// Absolute path to the workspace directory
        path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = FileIndexClient::connect(cli.addr.clone())
        .await
        .with_context(|| format!("failed to connect to Creep at {}", cli.addr))?;

    match cli.command {
        Commands::Search {
            pattern,
            workspace,
            file_type,
        } => cmd_search(&mut client, &pattern, workspace, file_type, cli.json).await,
        Commands::Metadata { path } => cmd_metadata(&mut client, &path, cli.json).await,
        Commands::Register { path } => cmd_register(&mut client, &path, cli.json).await,
        Commands::Unregister { path } => cmd_unregister(&mut client, &path, cli.json).await,
    }
}

async fn cmd_search(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    pattern: &str,
    workspace: Option<String>,
    file_type: Option<String>,
    json: bool,
) -> Result<()> {
    let response = client
        .search_files(proto::SearchFilesRequest {
            pattern: pattern.to_string(),
            workspace,
            file_type,
        })
        .await
        .context("search_files RPC failed")?;

    let files = response.into_inner().files;
    if json {
        print_json(&files)?;
    } else {
        for f in &files {
            println!(
                "{:<60} {:>8}  {}  {}  {}",
                f.path,
                format_size(f.size),
                format_time(f.modified_at),
                f.file_type,
                truncate_hash(&f.content_hash),
            );
        }
        if files.is_empty() {
            eprintln!("no files matched pattern '{pattern}'");
        }
    }
    Ok(())
}

async fn cmd_metadata(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .get_file_metadata(proto::GetFileMetadataRequest {
            path: path.to_string(),
        })
        .await
        .context("get_file_metadata RPC failed")?;

    match response.into_inner().file {
        Some(f) => {
            if json {
                print_json(&f)?;
            } else {
                println!("path:    {}", f.path);
                println!("size:    {}", format_size(f.size));
                println!("modified: {}", format_time(f.modified_at));
                println!("type:    {}", f.file_type);
                println!("hash:    {}", f.content_hash);
            }
        }
        None => {
            eprintln!("file not found in index: {path}");
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn cmd_register(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .register_workspace(proto::RegisterWorkspaceRequest {
            path: path.to_string(),
        })
        .await
        .context("register_workspace RPC failed")?;

    let count = response.into_inner().files_indexed;
    if json {
        println!(r#"{{"files_indexed":{count},"path":"{path}"}}"#);
    } else {
        println!("Indexed {count} files in {path}");
    }
    Ok(())
}

async fn cmd_unregister(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    client
        .unregister_workspace(proto::UnregisterWorkspaceRequest {
            path: path.to_string(),
        })
        .await
        .context("unregister_workspace RPC failed")?;

    if json {
        println!(r#"{{"unregistered":true,"path":"{path}"}}"#);
    } else {
        println!("Unregistered workspace {path}");
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_time(epoch_secs: i64) -> String {
    // Simple ISO-ish format without pulling in chrono
    // Unix timestamp to readable — just print the raw value for now,
    // chrono is not a dependency
    epoch_secs.to_string()
}

fn truncate_hash(hash: &str) -> &str {
    if hash.len() > 12 {
        &hash[..12]
    } else {
        hash
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
```

Note: `FileMetadata` from prost doesn't implement `Serialize` by default. We need to either add `serde` derives to the proto types or create wrapper structs. The simplest approach: add `type_attribute` to `tonic_build` config. Update `build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .compile_protos(&["proto/creep.proto"], &["proto/"])?;
    Ok(())
}
```

And regenerate `proto_gen/creep.v1.rs` with this config for Buck2.

- [ ] **Step 5: Add to workspace `Cargo.toml`**

In root `Cargo.toml`, add `"src/creep-cli"` to workspace members:

```toml
members = ["src/overseer", "src/queen", "src/drone-sdk", "src/drones/claude/base", "src/creep", "src/creep-cli", "src/nydus", "src/kerrigan", "src/evolution"]
```

- [ ] **Step 6: Create `src/creep-cli/BUCK`**

```python
CREEP_CLI_SRCS = glob(["src/**/*.rs", "proto/**/*.proto", "proto_gen/**/*.rs", "build.rs"])

CREEP_CLI_DEPS = [
    "//third-party:anyhow",
    "//third-party:clap",
    "//third-party:prost",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:tonic",
]

rust_binary(
    name = "creep-cli",
    srcs = CREEP_CLI_SRCS,
    crate_root = "src/main.rs",
    deps = CREEP_CLI_DEPS,
    env = {
        "CARGO_MANIFEST_DIR": ".",
        "OUT_DIR": "proto_gen",
    },
    visibility = ["PUBLIC"],
)

rust_test(
    name = "creep-cli-test",
    srcs = CREEP_CLI_SRCS,
    crate_root = "src/main.rs",
    deps = CREEP_CLI_DEPS,
    env = {
        "CARGO_MANIFEST_DIR": ".",
        "OUT_DIR": "proto_gen",
    },
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 7: Run `./tools/buckify.sh` to regenerate third-party BUCK**

```bash
./tools/buckify.sh
```

This picks up any new deps from the workspace. `clap` is already in third-party (used by kerrigan), `tonic`/`prost` are already there (used by creep). No new fixups expected.

- [ ] **Step 8: Verify cargo check passes**

```bash
cd src/creep-cli && cargo check
```

Expected: success, no errors.

- [ ] **Step 9: Verify buck2 build passes**

```bash
buck2 build root//src/creep-cli:creep-cli
```

Expected: BUILD SUCCEEDED

- [ ] **Step 10: Commit**

```bash
git add src/creep-cli/ Cargo.toml
git commit -m "feat(creep-cli): scaffold crate with gRPC client and four subcommands"
```

---

### Task 2: Test `creep-cli` Against Live Server

**Files:**
- Modify: `src/creep-cli/src/main.rs` (fix any issues found during manual test)

- [ ] **Step 1: Start creep server in background**

```bash
cd /home/jackm/repos/kerrigan && cargo run -p creep &
```

- [ ] **Step 2: Test register command**

```bash
cargo run -p creep-cli -- register /home/jackm/repos/kerrigan
```

Expected: `Indexed N files in /home/jackm/repos/kerrigan` where N > 0.

- [ ] **Step 3: Test search command (human output)**

```bash
cargo run -p creep-cli -- search "*.rs"
```

Expected: table of `.rs` files with path, size, timestamp, type, hash.

- [ ] **Step 4: Test search command (JSON output)**

```bash
cargo run -p creep-cli -- search "*.rs" --json
```

Expected: JSON array of file metadata objects.

- [ ] **Step 5: Test search with filters**

```bash
cargo run -p creep-cli -- search "*" --type rust --workspace /home/jackm/repos/kerrigan
```

Expected: only Rust files from the workspace.

- [ ] **Step 6: Test metadata command**

```bash
cargo run -p creep-cli -- metadata /home/jackm/repos/kerrigan/src/overseer/src/main.rs
```

Expected: path, size, modified, type (rust), hash fields.

- [ ] **Step 7: Test metadata for nonexistent file**

```bash
cargo run -p creep-cli -- metadata /does/not/exist
```

Expected: `file not found in index: /does/not/exist`, exit code 1.

- [ ] **Step 8: Test unregister command**

```bash
cargo run -p creep-cli -- unregister /home/jackm/repos/kerrigan
```

Expected: `Unregistered workspace /home/jackm/repos/kerrigan`

- [ ] **Step 9: Test connection failure**

```bash
cargo run -p creep-cli -- --addr http://localhost:9999 search "*.rs"
```

Expected: non-zero exit, error message about connection failure.

- [ ] **Step 10: Fix any issues, commit if changes were made**

```bash
git add src/creep-cli/
git commit -m "fix(creep-cli): fixes from manual testing"
```

---

### Task 3: Create `creep-discovery` Skill Plugin

**Files:**
- Create: `src/drones/claude/plugins/creep-discovery/package.json`
- Create: `src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md`

- [ ] **Step 1: Create plugin directory structure**

```bash
mkdir -p src/drones/claude/plugins/creep-discovery/skills/creep-discovery
```

- [ ] **Step 2: Create `package.json`**

Create `src/drones/claude/plugins/creep-discovery/package.json`:

```json
{
  "name": "creep-discovery",
  "version": "0.1.0"
}
```

- [ ] **Step 3: Create `SKILL.md`**

Create `src/drones/claude/plugins/creep-discovery/skills/creep-discovery/SKILL.md`:

```markdown
---
name: creep-discovery
description: "Use when exploring or navigating a codebase — find files by pattern, check file metadata, understand workspace structure. Faster than glob/grep for indexed workspaces."
---

# Creep File Discovery

Use `creep-cli` to search the pre-indexed file tree. The workspace was registered automatically on drone startup — files are already indexed with content hashes and type detection.

## When to Use

- Finding files by glob pattern across the workspace
- Checking whether a file exists and its type before reading it
- Getting a quick overview of what files exist in a directory pattern
- Comparing content hashes to detect changes

## When NOT to Use

- Searching file *contents* (use Grep for that)
- Reading file contents (use Read for that)
- If `creep-cli` fails with a connection error, fall back to Glob/Grep — Creep may not be running

## Commands

All commands support `--json` for machine-parseable output. Always use `--json` when you need to parse the result.

### Search files by pattern

```bash
creep-cli search "*.rs"                              # all Rust files
creep-cli search "src/**/*.rs" --json                # Rust files under src/, JSON output
creep-cli search "*_test.rs" --type rust             # test files, filtered by type
creep-cli search "*.toml" --workspace /path/to/repo  # filter by workspace
```

### Get metadata for a specific file

```bash
creep-cli metadata /absolute/path/to/file.rs
```

Output: path, size, modified timestamp, file type, BLAKE3 content hash.

### Register / unregister workspaces

Workspaces are registered automatically by the drone. You should not need these unless debugging.

```bash
creep-cli register /path/to/workspace
creep-cli unregister /path/to/workspace
```

## Tips

- Patterns are glob patterns, not regex. Use `*` for single-level, `**` for recursive.
- File types detected: rust, python, typescript, javascript, go, c, cpp, java, toml, yaml, json, markdown, and more.
- Content hashes are BLAKE3 — fast and collision-resistant. Use them to check if a file has changed between operations.
```

- [ ] **Step 4: Commit**

```bash
git add src/drones/claude/plugins/
git commit -m "feat(skill): add creep-discovery Claude Code plugin"
```

---

### Task 4: Buck2 Targets for Skill Plugin

**Files:**
- Create: `src/drones/claude/plugins/BUCK`
- Create: `src/drones/claude/plugins/install.sh`

- [ ] **Step 1: Create `src/drones/claude/plugins/BUCK`**

```python
filegroup(
    name = "creep-discovery",
    srcs = glob(["creep-discovery/**"]),
    visibility = ["PUBLIC"],
)

sh_binary(
    name = "install",
    main = "install.sh",
    resources = [":creep-discovery"],
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 2: Create `src/drones/claude/plugins/install.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail

DEST="${HOME}/.claude/plugins/kerrigan-creep-discovery"
mkdir -p "$DEST/skills/creep-discovery"

SCRIPT_DIR="$(dirname "$0")"

# Find the creep-discovery filegroup output
SRC="$SCRIPT_DIR/creep-discovery"
if [ ! -d "$SRC" ]; then
    # Buck2 resource layout may nest differently
    SRC="$SCRIPT_DIR/src/drones/claude/plugins/creep-discovery"
fi
if [ ! -d "$SRC" ]; then
    echo "error: creep-discovery plugin files not found" >&2
    exit 1
fi

cp "$SRC/package.json" "$DEST/package.json"
cp "$SRC/skills/creep-discovery/SKILL.md" "$DEST/skills/creep-discovery/SKILL.md"

echo "Installed creep-discovery plugin to $DEST"
```

- [ ] **Step 3: Make install.sh executable**

```bash
chmod +x src/drones/claude/plugins/install.sh
```

- [ ] **Step 4: Verify buck2 targets**

```bash
buck2 build root//src/drones/claude/plugins:creep-discovery
buck2 run root//src/drones/claude/plugins:install
```

Expected: first builds the filegroup, second copies files to `~/.claude/plugins/kerrigan-creep-discovery/`.

- [ ] **Step 5: Verify plugin is discovered by Claude Code**

After install, check that the skill appears in Claude Code's skill list. Run `claude` and check if `creep-discovery` shows up in available skills.

- [ ] **Step 6: Commit**

```bash
git add src/drones/claude/plugins/
git commit -m "feat(skill): Buck2 targets and install script for creep-discovery plugin"
```

---

### Task 5: Drone Hooks — Workspace Registration and Plugin Bundling

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`
- Modify: `src/drones/claude/base/src/environment.rs`

- [ ] **Step 1: Add `install_plugins` function to `environment.rs`**

Add at the end of `src/drones/claude/base/src/environment.rs` (before the `#[cfg(test)]` module):

```rust
/// Copy the creep-discovery plugin into the drone's Claude plugins directory.
/// Source: /opt/kerrigan/plugins/creep-discovery/ (container filesystem).
/// Destination: {home}/.claude/plugins/creep-discovery/
pub async fn install_plugins(home: &Path) -> Result<()> {
    let src = Path::new("/opt/kerrigan/plugins/creep-discovery");
    if !src.exists() {
        tracing::debug!("creep-discovery plugin not found at {}, skipping", src.display());
        return Ok(());
    }

    let dest = home.join(".claude/plugins/creep-discovery/skills/creep-discovery");
    fs::create_dir_all(&dest)
        .await
        .context("failed to create plugins directory")?;

    // Copy package.json
    fs::copy(
        src.join("package.json"),
        home.join(".claude/plugins/creep-discovery/package.json"),
    )
    .await
    .context("failed to copy package.json")?;

    // Copy SKILL.md
    fs::copy(
        src.join("skills/creep-discovery/SKILL.md"),
        dest.join("SKILL.md"),
    )
    .await
    .context("failed to copy SKILL.md")?;

    tracing::info!("installed creep-discovery plugin");
    Ok(())
}
```

- [ ] **Step 2: Add workspace registration to `setup()` in `drone.rs`**

In `src/drones/claude/base/src/drone.rs`, add after the `environment::write_env_vars` block (around line 65) and before the final `Ok(env)`:

```rust
        // Install Claude Code plugins into the drone home
        environment::install_plugins(&env.home).await?;

        // Best-effort: register workspace with Creep for fast file discovery
        match tokio::process::Command::new("creep-cli")
            .args(["register", &env.workspace.to_string_lossy()])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                tracing::info!(output = %stdout.trim(), "registered workspace with Creep");
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!(stderr = %stderr.trim(), "creep-cli register failed");
            }
            Err(e) => {
                tracing::warn!(error = %e, "creep-cli not available, skipping workspace registration");
            }
        }
```

- [ ] **Step 3: Add workspace unregistration to `teardown()` in `drone.rs`**

In `src/drones/claude/base/src/drone.rs`, replace the existing `teardown` method:

```rust
    async fn teardown(&self, env: &DroneEnvironment) {
        // Best-effort: unregister workspace from Creep
        match tokio::process::Command::new("creep-cli")
            .args(["unregister", &env.workspace.to_string_lossy()])
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                tracing::info!("unregistered workspace from Creep");
            }
            Ok(_) | Err(_) => {
                tracing::debug!("creep-cli unregister skipped (not available or failed)");
            }
        }
        environment::cleanup(&env.home).await;
    }
```

- [ ] **Step 4: Verify cargo check passes**

```bash
cd src/drones/claude/base && cargo check
```

Expected: success. Note: `cargo test` will fail because of `include_bytes!("config/claude-cli")` — that's expected and pre-existing.

- [ ] **Step 5: Verify buck2 build passes**

```bash
buck2 build root//src/drones/claude/base:claude-drone
```

Expected: BUILD SUCCEEDED

- [ ] **Step 6: Commit**

```bash
git add src/drones/claude/base/
git commit -m "feat(drone): register workspace with Creep and install discovery plugin"
```

---

### Task 6: Container Build Integration

**Files:**
- Modify: `deploy/dev/build.sh`
- Modify: `Dockerfile`

- [ ] **Step 1: Update `deploy/dev/build.sh`**

Add `creep-cli` to the build targets and staging. Replace the `buck2 build` line:

```bash
echo "=== building kerrigan binaries with buck2 ==="
buck2 build \
  root//src/overseer:overseer \
  root//src/queen:queen \
  root//src/creep:creep \
  root//src/creep-cli:creep-cli \
  root//src/drones/claude/base:claude-drone
```

Add to the staging section, update `mkdir` and add entries to the `for` loop:

```bash
mkdir -p "$STAGE/bin" "$STAGE/drones" "$STAGE/plugins"
```

Add `creep-cli` to the `for target_bin` loop:

```bash
  "root//src/creep-cli:creep-cli bin/creep-cli" \
```

Add plugin staging after the binary loop:

```bash
echo "=== staging plugins ==="
cp -r src/drones/claude/plugins/creep-discovery "$STAGE/plugins/creep-discovery"
echo "  creep-discovery -> $STAGE/plugins/creep-discovery"
```

- [ ] **Step 2: Update `Dockerfile`**

Add after the `COPY deploy/dev/.stage/drones/claude-drone` line:

```dockerfile
COPY deploy/dev/.stage/bin/creep-cli   /opt/kerrigan/bin/creep-cli
COPY deploy/dev/.stage/plugins/        /opt/kerrigan/plugins/
```

- [ ] **Step 3: Ensure `/opt/kerrigan/bin` is on PATH**

Check `deploy/dev/entrypoint.sh`. Currently it calls binaries by absolute path. Add PATH export near the top (after `set -e`):

```bash
export PATH="/opt/kerrigan/bin:$PATH"
```

This ensures `creep-cli` is available to drones that call it by name.

- [ ] **Step 4: Commit**

```bash
git add deploy/dev/build.sh Dockerfile deploy/dev/entrypoint.sh
git commit -m "feat(deploy): add creep-cli and discovery plugin to container build"
```

---

### Task 7: Update CLAUDE.md and Roadmap

**Files:**
- Modify: `CLAUDE.md`
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Update CLAUDE.md**

Add `creep-cli` to the Creep section in `CLAUDE.md`:

```markdown
### Creep CLI (`src/creep-cli/`)
Thin CLI client for Creep's gRPC API. Used by skills and drone hooks for file discovery.

- **Build:** `buck2 build root//src/creep-cli:creep-cli`
- **Install:** `buck2 run root//src/drones/claude/plugins:install` (installs creep-discovery skill to ~/.claude/plugins/)
- **Usage:** `creep-cli search "*.rs" --json`, `creep-cli metadata <path>`, `creep-cli register <path>`, `creep-cli unregister <path>`
```

- [ ] **Step 2: Update roadmap item #8**

In `docs/ROADMAP.md`, update the Creep integration entry:

```markdown
**8. Creep integration with drones** `[done — CLI + skill + drone hooks]`
- `creep-cli` crate: thin gRPC client wrapping Creep's four RPCs (search, metadata, register, unregister)
- `creep-discovery` Claude Code skill plugin: teaches drones to use creep-cli for file discovery
- Drone hooks: auto-register workspace on setup, unregister on teardown
- Plugin bundled into drone home, CLI shipped in container
- Depends on: Creep v1 merged (#7)
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: update CLAUDE.md and roadmap with creep-cli and discovery skill"
```
