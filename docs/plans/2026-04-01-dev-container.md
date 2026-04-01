# Dev Container Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Single all-in-one Docker container running Overseer + Queen + claude-drone for testing the autonomous development loop.

**Architecture:** Pre-built Buck2 binaries copied into an Ubuntu 24.04 runtime image. A shell entrypoint starts Overseer in the background, waits for it, then runs Queen in the foreground. The drone binary lives at a known path that Queen's `drone_dir` config points to.

**Tech Stack:** Docker, Buck2 (host builds), Ubuntu 24.04, gh CLI, buck2 binary

---

## File Structure

| File | Responsibility |
|---|---|
| `Dockerfile` | Multi-stage: install runtime deps + buck2, COPY pre-built binaries |
| `deploy/dev/overseer.toml` | Overseer config for container (sqlite in /data) |
| `deploy/dev/hatchery.toml` | Queen config for container (localhost overseer, drone_dir=/opt/kerrigan/drones) |
| `deploy/dev/entrypoint.sh` | Starts Overseer bg, waits for ready, runs Queen fg |
| `deploy/dev/build.sh` | Convenience script: buck2 build all targets, docker build |

---

### Task 1: Container Config Files

**Files:**
- Create: `deploy/dev/overseer.toml`
- Create: `deploy/dev/hatchery.toml`

- [ ] **Step 1: Create deploy/dev directory**

```bash
mkdir -p deploy/dev
```

- [ ] **Step 2: Write overseer.toml**

Create `deploy/dev/overseer.toml`:

```toml
[server]
http_port = 3100
mcp_transport = "http"

[storage]
database_url = "sqlite:///data/overseer.db"
artifact_url = "file:///data/artifacts"

[embedding]
default = "stub"

[embedding.providers.stub]
source = "stub"
dimensions = 384
```

Key difference from repo root `overseer.toml`: database and artifact paths point to `/data/` (volume-mounted in container).

- [ ] **Step 3: Write hatchery.toml**

Create `deploy/dev/hatchery.toml`:

```toml
[queen]
name = "dev-hatchery"
overseer_url = "http://localhost:3100"
drone_dir = "/opt/kerrigan/drones"
max_concurrency = 1
drone_timeout = "2h"

[creep]
enabled = false
```

Queen expects the drone binary at `/opt/kerrigan/drones/claude-drone` (matching `drone_dir` + the `drone_type` from job config, which defaults to `"claude-drone"` in `src/queen/src/actors/poller.rs:59`).

Creep disabled — simplifies the first run. Queen's creep_manager actor will log that it's disabled and skip startup.

- [ ] **Step 4: Commit**

```bash
git add deploy/dev/overseer.toml deploy/dev/hatchery.toml
git commit -m "feat(deploy): add dev container config files for overseer and queen"
```

---

### Task 2: Entrypoint Script

**Files:**
- Create: `deploy/dev/entrypoint.sh`

- [ ] **Step 1: Write entrypoint.sh**

Create `deploy/dev/entrypoint.sh`:

```bash
#!/bin/bash
set -e

echo "=== starting overseer ==="
/opt/kerrigan/bin/overseer /opt/kerrigan/config/overseer.toml &
OVERSEER_PID=$!

# Wait for Overseer's TCP port to accept connections.
# There is no /health endpoint — we check that the port is listening.
echo "waiting for overseer on port 3100..."
for i in $(seq 1 30); do
  if curl -sf http://localhost:3100/api/jobs/definitions > /dev/null 2>&1; then
    echo "overseer ready (pid $OVERSEER_PID)"
    break
  fi
  if ! kill -0 "$OVERSEER_PID" 2>/dev/null; then
    echo "ERROR: overseer exited unexpectedly"
    exit 1
  fi
  sleep 1
done

# If overseer never became ready, bail
if ! curl -sf http://localhost:3100/api/jobs/definitions > /dev/null 2>&1; then
  echo "ERROR: overseer did not become ready after 30s"
  kill "$OVERSEER_PID" 2>/dev/null || true
  exit 1
fi

echo "=== starting queen ==="
exec /opt/kerrigan/bin/queen --config /opt/kerrigan/config/hatchery.toml
```

Notes:
- Overseer runs in the background. We check by hitting a known API endpoint (`GET /api/jobs/definitions` returns an empty list on a fresh DB — good enough for a readiness check).
- If Overseer crashes during startup (bad config, migration failure), we detect it via `kill -0` and exit early.
- Queen runs via `exec` so it replaces the shell — it receives signals directly (Ctrl+C works for graceful shutdown).
- Queen's logs stream to stdout/stderr. When a drone spawns Claude CLI and it prints auth URLs to stderr, they surface through Queen's supervisor, which logs drone output via `tracing`.

- [ ] **Step 2: Make it executable**

```bash
chmod +x deploy/dev/entrypoint.sh
```

- [ ] **Step 3: Commit**

```bash
git add deploy/dev/entrypoint.sh
git commit -m "feat(deploy): add dev container entrypoint script"
```

---

### Task 3: Build Helper Script

**Files:**
- Create: `deploy/dev/build.sh`

- [ ] **Step 1: Write build.sh**

Create `deploy/dev/build.sh`:

```bash
#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "=== building kerrigan binaries with buck2 ==="
buck2 build \
  root//src/overseer:overseer \
  root//src/queen:queen \
  root//src/creep:creep \
  root//src/drones/claude/base:claude-drone

# Buck2 output paths are content-addressed. Use buck2 build --show-full-output
# to get the actual paths, then copy to a staging dir for Docker.
STAGE="$REPO_ROOT/deploy/dev/.stage"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/drones"

echo "=== staging binaries ==="
for target_bin in \
  "root//src/overseer:overseer bin/overseer" \
  "root//src/queen:queen bin/queen" \
  "root//src/creep:creep bin/creep" \
  "root//src/drones/claude/base:claude-drone drones/claude-drone"; do

  target="${target_bin% *}"
  dest="${target_bin#* }"
  src=$(buck2 build --show-full-output "$target" 2>/dev/null | awk '{print $2}')
  cp "$src" "$STAGE/$dest"
  echo "  $target -> $STAGE/$dest"
done

echo "=== building docker image ==="
docker build -t kerrigan -f "$REPO_ROOT/Dockerfile" "$REPO_ROOT"

echo "=== cleaning staging dir ==="
rm -rf "$STAGE"

echo "=== done ==="
echo "Run with: docker run -it --rm -p 3100:3100 -v kerrigan-data:/data kerrigan"
```

Notes:
- Buck2 output paths are content-addressed (e.g. `buck-out/v2/gen/root/<hash>/src/overseer/__overseer__/overseer`). Using `--show-full-output` avoids hardcoding the hash.
- Binaries are staged into `deploy/dev/.stage/` so the Dockerfile can COPY from a predictable path.
- The staging dir is cleaned up after docker build.

- [ ] **Step 2: Make it executable**

```bash
chmod +x deploy/dev/build.sh
```

- [ ] **Step 3: Add .stage to .gitignore**

Append to `.gitignore` (or create `deploy/dev/.gitignore`):

Create `deploy/dev/.gitignore`:

```
.stage/
```

- [ ] **Step 4: Commit**

```bash
git add deploy/dev/build.sh deploy/dev/.gitignore
git commit -m "feat(deploy): add build helper script for dev container"
```

---

### Task 4: Dockerfile

**Files:**
- Create: `Dockerfile`

- [ ] **Step 1: Write Dockerfile**

Create `Dockerfile` at the repo root:

```dockerfile
FROM ubuntu:24.04

# Runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    zstd \
    && rm -rf /var/lib/apt/lists/*

# GitHub CLI (gh)
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# Buck2 (pinned to 2026-01-19 release matching .buckconfig)
# gh requires auth for release downloads, so use the direct URL pattern.
# The release asset URL is deterministic: github.com/facebook/buck2/releases/download/<tag>/<asset>
RUN curl -fsSL "https://github.com/facebook/buck2/releases/download/2026-01-19/buck2-x86_64-unknown-linux-gnu.zst" \
      | zstd -d > /usr/local/bin/buck2 \
    && chmod +x /usr/local/bin/buck2

# Create directory structure
RUN mkdir -p /opt/kerrigan/bin /opt/kerrigan/drones /opt/kerrigan/config /data/artifacts

# Copy pre-built binaries from staging dir (populated by deploy/dev/build.sh)
COPY deploy/dev/.stage/bin/overseer   /opt/kerrigan/bin/overseer
COPY deploy/dev/.stage/bin/queen      /opt/kerrigan/bin/queen
COPY deploy/dev/.stage/bin/creep      /opt/kerrigan/bin/creep
COPY deploy/dev/.stage/drones/claude-drone /opt/kerrigan/drones/claude-drone

# Copy container-specific configs
COPY deploy/dev/overseer.toml   /opt/kerrigan/config/overseer.toml
COPY deploy/dev/hatchery.toml   /opt/kerrigan/config/hatchery.toml

# Copy entrypoint
COPY deploy/dev/entrypoint.sh   /opt/kerrigan/entrypoint.sh

# Expose Overseer HTTP port
EXPOSE 3100

# Data volume
VOLUME /data

ENTRYPOINT ["/opt/kerrigan/entrypoint.sh"]
```

Notes:
- `gh` is installed via the official apt repo so it's a proper package (auto-updates, etc). It's needed at runtime for drone PR creation and auth.
- Buck2 is downloaded directly via curl rather than `gh release download` since `gh` inside Docker won't be authenticated during image build.
- The `.stage/` directory is populated by `deploy/dev/build.sh` before `docker build` runs.
- `/data` is a volume for the sqlite DB and artifacts — persists across container restarts.
- No Claude CLI installed — it's bundled inside the drone binary per the drone config spec.

- [ ] **Step 2: Commit**

```bash
git add Dockerfile
git commit -m "feat(deploy): add Dockerfile for dev container"
```

---

### Task 5: Smoke Test

No automated tests — this is a manual verification that the container builds and the services start.

- [ ] **Step 1: Build everything**

```bash
./deploy/dev/build.sh
```

This runs `buck2 build` for all four targets, stages binaries, and runs `docker build`.

Expected: Image builds successfully. No errors.

- [ ] **Step 2: Run the container**

```bash
docker run -it --rm -p 3100:3100 -v kerrigan-data:/data kerrigan
```

Expected output (approximately):
```
=== starting overseer ===
waiting for overseer on port 3100...
overseer ready (pid 7)
=== starting queen ===
INFO queen starting name="dev-hatchery"
```

Queen should start polling Overseer. You'll see periodic log lines from the heartbeat and poller actors.

- [ ] **Step 3: Verify Overseer is reachable from host**

In a separate terminal:

```bash
curl -s http://localhost:3100/api/jobs/definitions | head
```

Expected: `[]` (empty JSON array — no job definitions yet).

- [ ] **Step 4: Verify Queen registered with Overseer**

```bash
curl -s http://localhost:3100/api/hatcheries | python3 -m json.tool
```

Expected: JSON array containing one hatchery with `name: "dev-hatchery"`.

- [ ] **Step 5: Stop and clean up**

Ctrl+C in the container terminal. Queen handles SIGTERM gracefully (deregisters from Overseer, cancels actors).

```bash
docker volume rm kerrigan-data  # if you want a fresh DB next time
```
