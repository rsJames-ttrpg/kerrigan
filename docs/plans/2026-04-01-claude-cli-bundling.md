# Claude CLI Bundling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bundle the Claude CLI binary into the drone at build time so drones are fully self-contained — no runtime dependency on `claude` being installed.

**Architecture:** Buck2 `http_file` fetches the pinned Claude CLI binary. The drone's `mapped_srcs` makes it available to `include_bytes!`. At runtime the drone extracts it to the temp home and executes it from there.

**Tech Stack:** Buck2 (`http_file`, `mapped_srcs`), Rust (`include_bytes!`, `tokio::fs`)

---

## File Structure

| File | Responsibility |
|---|---|
| `tools/BUCK` | Add `http_file` target for Claude CLI (pinned version + SHA256) |
| `tools/update-claude-cli.sh` | Utility script to fetch latest version/SHA and update `tools/BUCK` |
| `src/drones/claude/base/BUCK` | Add Claude CLI to `mapped_srcs` so `include_bytes!` can embed it |
| `src/drones/claude/base/src/environment.rs` | Embed CLI bytes, extract to temp home during `create_home()` |
| `src/drones/claude/base/src/drone.rs` | Use extracted CLI path instead of `Command::new("claude")` |

---

### Task 1: Add Claude CLI http_file target to tools/BUCK

**Files:**
- Modify: `tools/BUCK`

- [ ] **Step 1: Add http_file target**

Add the following to the end of `tools/BUCK`:

```python
# -- Claude CLI (bundled into drone binaries) ----------------------------------
# Usage: referenced via mapped_srcs in drone BUCK targets
# Update: `./tools/update-claude-cli.sh`

CLAUDE_CLI_VERSION = "2.1.89"
CLAUDE_CLI_SHA256 = "903cb3c96b314d86856632c8702f5cdf971b804d0b19ef87446573bcd1d7df1c"
CLAUDE_CLI_BUCKET = "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"

http_file(
    name = "claude-cli-linux-x64",
    urls = ["{}/{}/linux-x64/claude".format(CLAUDE_CLI_BUCKET, CLAUDE_CLI_VERSION)],
    sha256 = CLAUDE_CLI_SHA256,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 2: Verify Buck2 can fetch it**

Run:
```bash
buck2 build root//tools:claude-cli-linux-x64
```

Expected: BUILD SUCCEEDED. This downloads the ~228MB Claude CLI binary and caches it in buck-out.

- [ ] **Step 3: Verify the downloaded file is the actual binary**

```bash
file $(buck2 build --show-full-output root//tools:claude-cli-linux-x64 | awk '{print $2}')
```

Expected: `ELF 64-bit LSB executable` or `ELF 64-bit LSB pie executable` (it's a native Linux binary).

- [ ] **Step 4: Commit**

```bash
git add tools/BUCK
git commit -m "feat(tools): add hermetic Claude CLI fetch target (pinned 2.1.89)"
```

---

### Task 2: Update script for Claude CLI version

**Files:**
- Create: `tools/update-claude-cli.sh`

- [ ] **Step 1: Write the script**

Create `tools/update-claude-cli.sh`:

```bash
#!/bin/bash
set -euo pipefail

BUCK_FILE="$(cd "$(dirname "$0")" && pwd)/BUCK"
BUCKET="https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"

# Get current pinned version from BUCK file
OLD_VERSION=$(grep 'CLAUDE_CLI_VERSION = ' "$BUCK_FILE" | sed 's/.*"\(.*\)".*/\1/')

# Fetch latest version
NEW_VERSION=$(curl -sfL "$BUCKET/latest")
if [ -z "$NEW_VERSION" ]; then
    echo "ERROR: failed to fetch latest version" >&2
    exit 1
fi

# Fetch manifest and extract linux-x64 checksum
NEW_SHA256=$(curl -sfL "$BUCKET/$NEW_VERSION/manifest.json" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['platforms']['linux-x64']['checksum'])")
if [ -z "$NEW_SHA256" ]; then
    echo "ERROR: failed to fetch checksum from manifest" >&2
    exit 1
fi

if [ "$OLD_VERSION" = "$NEW_VERSION" ]; then
    echo "Claude CLI already at $OLD_VERSION"
    exit 0
fi

# Update BUCK file
sed -i "s/CLAUDE_CLI_VERSION = \".*\"/CLAUDE_CLI_VERSION = \"$NEW_VERSION\"/" "$BUCK_FILE"
sed -i "s/CLAUDE_CLI_SHA256 = \".*\"/CLAUDE_CLI_SHA256 = \"$NEW_SHA256\"/" "$BUCK_FILE"

echo "Claude CLI: $OLD_VERSION → $NEW_VERSION"
echo "SHA256: $NEW_SHA256"
echo "Updated $BUCK_FILE"
```

- [ ] **Step 2: Make it executable**

```bash
chmod +x tools/update-claude-cli.sh
```

- [ ] **Step 3: Test the script (dry run)**

```bash
./tools/update-claude-cli.sh
```

Expected: `Claude CLI already at 2.1.89` (since we just pinned to the latest).

- [ ] **Step 4: Commit**

```bash
git add tools/update-claude-cli.sh
git commit -m "feat(tools): add Claude CLI version update script"
```

---

### Task 3: Wire Claude CLI into drone build

**Files:**
- Modify: `src/drones/claude/base/BUCK`

- [ ] **Step 1: Add Claude CLI to mapped_srcs**

In `src/drones/claude/base/BUCK`, change `CLAUDE_DRONE_MAPPED_SRCS` from:

```python
CLAUDE_DRONE_MAPPED_SRCS = {
    "src/config/settings.json": "src/config/settings.json",
    "src/config/CLAUDE.md": "src/config/CLAUDE.md",
}
```

to:

```python
CLAUDE_DRONE_MAPPED_SRCS = {
    "src/config/settings.json": "src/config/settings.json",
    "src/config/CLAUDE.md": "src/config/CLAUDE.md",
    "//tools:claude-cli-linux-x64": "src/config/claude-cli",
}
```

The `//tools:claude-cli-linux-x64` target output is mapped into the source sandbox at `src/config/claude-cli`, making it accessible to `include_bytes!("config/claude-cli")`.

- [ ] **Step 2: Verify the drone still builds**

```bash
buck2 build root//src/drones/claude/base:claude-drone
```

Expected: BUILD SUCCEEDED. The binary will now be ~250MB+ (was ~20MB before) because it embeds the Claude CLI.

Note: This step will fail until Task 4 adds the `include_bytes!` reference in the Rust code. If building here, expect a warning about unused mapped_srcs or a clean build. The real verification is after Task 4.

- [ ] **Step 3: Commit**

```bash
git add src/drones/claude/base/BUCK
git commit -m "feat(drone): add Claude CLI binary to drone mapped_srcs"
```

---

### Task 4: Embed and extract Claude CLI at runtime

**Files:**
- Modify: `src/drones/claude/base/src/environment.rs`
- Modify: `src/drones/claude/base/src/drone.rs`

- [ ] **Step 1: Add the CLI constant and extraction to environment.rs**

In `src/drones/claude/base/src/environment.rs`, add the new constant after the existing ones:

```rust
const CLAUDE_CLI: &[u8] = include_bytes!("config/claude-cli");
```

Add this import at the top of the file (after the existing `use` statements):

```rust
use std::os::unix::fs::PermissionsExt;
```

In the `create_home` function, after writing `settings.json` and before the credentials symlink section, add extraction of the CLI binary:

```rust
    // Write embedded Claude CLI binary
    let claude_bin_dir = claude_dir.join("bin");
    fs::create_dir_all(&claude_bin_dir)
        .await
        .context("failed to create .claude/bin dir")?;
    let claude_bin = claude_bin_dir.join("claude");
    fs::write(&claude_bin, CLAUDE_CLI)
        .await
        .context("failed to write claude CLI binary")?;
    fs::set_permissions(&claude_bin, std::fs::Permissions::from_mode(0o755))
        .await
        .context("failed to set claude CLI permissions")?;
```

- [ ] **Step 2: Update drone.rs to use the extracted CLI path**

In `src/drones/claude/base/src/drone.rs`, in the `execute` method, change:

```rust
        let mut child = Command::new("claude")
```

to:

```rust
        let claude_bin = env.home.join(".claude/bin/claude");
        let mut child = Command::new(&claude_bin)
```

- [ ] **Step 3: Update the test in environment.rs**

In the `test_create_home_creates_dirs` test, add assertions for the CLI binary after the existing CLAUDE.md assertions:

```rust
        // Embedded Claude CLI written and executable
        let claude_bin = env.home.join(".claude/bin/claude");
        assert!(claude_bin.exists(), "claude CLI binary should exist");
        let metadata = std::fs::metadata(&claude_bin).expect("claude CLI metadata");
        assert!(
            metadata.permissions().mode() & 0o111 != 0,
            "claude CLI should be executable"
        );
```

Add this import at the top of the test module (inside `#[cfg(test)] mod tests`):

```rust
    use std::os::unix::fs::PermissionsExt;
```

- [ ] **Step 4: Run tests**

```bash
cd src/drones/claude/base && cargo test
```

Expected: all tests pass. The `test_create_home_creates_dirs` test verifies the CLI binary is extracted and executable.

Note: `cargo test` will work because `cargo` reads `mapped_srcs` differently — but the `include_bytes!` path `config/claude-cli` needs to exist. For Cargo builds, you may need to place a dummy file. If cargo test fails because the file doesn't exist, create a small placeholder:

```bash
touch src/drones/claude/base/src/config/claude-cli
```

This placeholder is only for Cargo's benefit — Buck2 provides the real binary via `mapped_srcs`. Add it to `.gitignore` if it's a dummy.

Alternatively, skip Cargo tests and use Buck2:

```bash
buck2 test root//src/drones/claude/base:claude-drone-test
```

- [ ] **Step 5: Verify the full build produces a working drone**

```bash
buck2 build root//src/drones/claude/base:claude-drone
ls -lh $(buck2 build --show-full-output root//src/drones/claude/base:claude-drone | awk '{print $2}')
```

Expected: The binary is ~250MB (was ~20MB), confirming the CLI is embedded.

- [ ] **Step 6: Commit**

```bash
git add src/drones/claude/base/src/environment.rs src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): embed Claude CLI binary, extract at runtime"
```

---

### Task 5: End-to-end test in container

No automated test — manual verification that the bundled drone works in the dev container.

- [ ] **Step 1: Rebuild the container**

```bash
./deploy/dev/build.sh
```

Expected: builds successfully. The staged `claude-drone` binary will be ~250MB.

- [ ] **Step 2: Run the container**

```bash
docker run -it --rm -p 3100:3100 -v /tmp/kerrigan-e2e:/data kerrigan
```

Expected: Overseer and Queen start normally.

- [ ] **Step 3: Submit a test job (from another terminal)**

```bash
HATCHERY_ID=$(curl -s http://localhost:3100/api/hatcheries | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")

DEF_ID=$(curl -s -X POST http://localhost:3100/api/jobs/definitions \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "test-bundled-cli",
    "description": "Test that bundled Claude CLI works",
    "config": {
      "drone_type": "claude-drone",
      "repo_url": "https://github.com/rsJames-ttrpg/kerrigan.git",
      "task": "Say hello"
    }
  }' | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

RUN_ID=$(curl -s -X POST http://localhost:3100/api/jobs/runs \
  -H 'Content-Type: application/json' \
  -d "{\"definition_id\": \"$DEF_ID\", \"triggered_by\": \"manual\"}" \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")

curl -s -X PUT "http://localhost:3100/api/hatcheries/$HATCHERY_ID/jobs/$RUN_ID"
```

- [ ] **Step 4: Watch the logs**

In the container terminal, watch for:
- `spawning drone` — Queen picks up the job
- `drone starting` — drone SDK harness starts
- Claude CLI auth URL — instead of "No such file or directory", you should see the CLI start and print an auth link

The auth link surfacing in Queen's logs validates the full chain: Buck2 fetch → embed → extract → execute.
