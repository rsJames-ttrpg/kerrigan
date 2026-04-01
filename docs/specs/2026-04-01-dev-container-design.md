# Dev Container Design

## Overview

Single all-in-one container for testing the Overseer → Queen → Drone loop. Not a production deployment — a dev/testing environment for iterating on the autonomous development pipeline.

The container bundles all Kerrigan services (Overseer, Queen, Creep, claude-drone) with the toolchain a drone needs to work on the Kerrigan repo itself (Buck2, git, gh). The Claude CLI is bundled inside the drone binary — not installed separately in the container.

## Build Strategy

Pre-built binaries, not built inside the container. The host builds with Buck2 (authoritative build system), then the Dockerfile COPYs the binaries into a runtime image.

```
Host: buck2 build root//src/overseer:overseer root//src/queen:queen \
      root//src/creep:creep root//src/drones/claude/base:claude-drone
      ↓
Dockerfile: COPY binaries into /opt/kerrigan/bin/
```

## Base Image

Ubuntu 24.04. Matches the BuildBuddy RBE worker image (`gcr.io/flame-public/rbe-ubuntu24-04:latest`) glibc baseline, so binaries built with the hermetic toolchain are compatible.

## Installed Tools

| Tool | Source | Purpose |
|---|---|---|
| git | apt | Drone clones repos |
| gh | GitHub apt repo | Drone creates PRs, auth link flow |
| zstd | apt | Buck2 binary decompression |
| curl | apt | Health checks, downloads |
| buck2 | GH release `2026-01-19` | Drone builds kerrigan (dogfooding) |

The Claude CLI is **not** installed in the container. Per the drone config spec, it is bundled into the drone binary at build time (fetched by Buck2, embedded via `include_bytes!`). At runtime the drone extracts it to its temp home and runs it from there.

### Buck2 Installation

```bash
gh release download 2026-01-19 --repo facebook/buck2 \
  --pattern 'buck2-x86_64-unknown-linux-gnu.zst' -O - \
  | zstd -d > /usr/local/bin/buck2
chmod +x /usr/local/bin/buck2
```

## Container Layout

```
/opt/kerrigan/
  bin/
    overseer          # Pre-built binary
    queen             # Pre-built binary
    creep             # Pre-built binary
  drones/
    claude-drone      # Pre-built drone binary
  config/
    overseer.toml     # Overseer config
    hatchery.toml     # Queen config
/data/
  overseer.db         # SQLite database (volume-mounted)
  artifacts/          # Artifact storage
/usr/local/bin/
  buck2               # Buck2 binary
```

## Config Files

### overseer.toml

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

### hatchery.toml

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

Creep disabled initially to simplify the first run. Can be enabled once the core loop works.

## Entrypoint

Shell script that starts services in order:

```bash
#!/bin/bash
set -e

# Start Overseer in background
/opt/kerrigan/bin/overseer /opt/kerrigan/config/overseer.toml &
OVERSEER_PID=$!

# Wait for Overseer to be healthy
for i in $(seq 1 30); do
  if curl -sf http://localhost:3100/api/jobs/definitions > /dev/null 2>&1; then
    echo "overseer ready"
    break
  fi
  sleep 1
done

# Start Queen in foreground (logs visible, auth links clickable)
exec /opt/kerrigan/bin/queen --config /opt/kerrigan/config/hatchery.toml
```

Queen runs in the foreground so its stdout/stderr stream to the terminal. When a drone spawns Claude CLI and it prints an auth URL, that surfaces through Queen's supervisor logs.

## Auth Flow

No pre-baked credentials. The intended test flow:

1. Container starts, Overseer + Queen come up
2. A job is submitted to Overseer (manually via curl or MCP)
3. Queen polls, picks up the job, spawns claude-drone
4. Drone runs `claude --dangerously-skip-permissions ...`
5. Claude CLI prints an auth link to stderr
6. Auth link surfaces in Queen's log output (visible in terminal)
7. User clicks the link, authenticates in browser
8. Claude CLI proceeds with the task

This validates the full auth flow works through the service stack.

## Usage

```bash
# 1. Build binaries on host
buck2 build root//src/overseer:overseer root//src/queen:queen \
  root//src/creep:creep root//src/drones/claude/base:claude-drone

# 2. Build container image
docker build -t kerrigan -f Dockerfile .

# 3. Run (interactive, so auth links are clickable)
docker run -it --rm -v kerrigan-data:/data kerrigan

# 4. In another terminal, submit a test job
curl -X POST http://localhost:3100/api/jobs/definitions \
  -H 'Content-Type: application/json' \
  -d '{"name": "test", "config": {"drone_type": "claude-drone", "repo_url": "https://github.com/...", "task": "echo hello"}}'
```

Port 3100 needs to be published (`-p 3100:3100`) if submitting jobs from the host.

## What This Does NOT Cover

- Production deployment (k8s, systemd, multi-node)
- Cross-compilation for aarch64/RPi (container is x86_64 for now)
- Creep integration (disabled, enable later)
- Persistent Claude credentials across container restarts
- CI/CD integration
