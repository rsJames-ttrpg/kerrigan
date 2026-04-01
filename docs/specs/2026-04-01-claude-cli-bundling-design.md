# Claude CLI Bundling Design

## Overview

Bundle the Claude CLI binary into the drone binary at build time so drones are fully self-contained. No runtime dependency on `claude` being on PATH or installed in the container.

## Download Infrastructure

The Claude CLI is distributed via a GCS bucket:

```
Base:     https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases
Version:  $BASE/latest              → "2.1.89"
Manifest: $BASE/$VERSION/manifest.json  → per-platform checksums
Binary:   $BASE/$VERSION/$PLATFORM/claude
```

Platforms: `linux-x64`, `linux-arm64`, `darwin-x64`, `darwin-arm64`, plus musl variants.

## Build-Time: Fetch and Pin

Add an `http_file` target to `tools/BUCK` that fetches the Claude CLI binary for linux-x64, pinned to a specific version with SHA256 verification:

```python
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

To update: change `CLAUDE_CLI_VERSION` and `CLAUDE_CLI_SHA256`. The SHA256 can be retrieved from the manifest at `$BASE/$VERSION/manifest.json` under `platforms["linux-x64"].checksum`.

## Build-Time: Drone Embeds the Binary

The drone's BUCK target uses `mapped_srcs` to make the CLI binary available in the source sandbox, where `include_bytes!` can reference it at compile time:

```python
CLAUDE_DRONE_MAPPED_SRCS = {
    "src/config/settings.json": "src/config/settings.json",
    "src/config/CLAUDE.md": "src/config/CLAUDE.md",
    "src/config/claude-cli": "//tools:claude-cli-linux-x64",
}
```

In the Rust source:

```rust
const CLAUDE_CLI: &[u8] = include_bytes!("config/claude-cli");
```

This embeds the ~228MB binary directly into the drone executable. The resulting drone binary will be ~250MB+.

## Runtime: Extract and Execute

When the drone sets up its isolated temp home (`/tmp/drone-{id}/`), it extracts the embedded CLI:

```
/tmp/drone-{id}/
  .claude/
    bin/
      claude          ← extracted from include_bytes!
    settings.json
    .credentials.json ← symlink to host credentials
  .task
  workspace/          ← git clone target
  CLAUDE.md
```

The extraction happens in `environment.rs::create_home()`. The drone's `execute()` method in `drone.rs` changes from `Command::new("claude")` to `Command::new(env.home.join(".claude/bin/claude"))`.

## Files Changed

| File | Change |
|---|---|
| `tools/BUCK` | Add `http_file` target `claude-cli-linux-x64` |
| `src/drones/claude/base/BUCK` | Add `"src/config/claude-cli": "//tools:claude-cli-linux-x64"` to `mapped_srcs` |
| `src/drones/claude/base/src/environment.rs` | Add `CLAUDE_CLI` constant, extract to `.claude/bin/claude` in `create_home()` |
| `src/drones/claude/base/src/drone.rs` | Use `env.home.join(".claude/bin/claude")` instead of `Command::new("claude")` |

## What This Does NOT Cover

- Multi-platform builds (only linux-x64 for now)
- Claude CLI auto-update (pinned version, updated manually)
- Compressed embedding (not needed at this scale)
