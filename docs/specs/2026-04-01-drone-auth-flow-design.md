# Drone Auth Flow Design

## Overview

Enable Claude CLI authentication through the drone→queen protocol so auth URLs surface in Queen's logs and the user can click them. Also document credential mounting as a bypass option.

## Current State

The protocol already has `AuthRequest` (drone→queen) and `AuthResponse` (queen→drone). Queen's supervisor already handles `AuthRequest` by surfacing it through the notifier. The `LogNotifier` already logs auth URLs at WARN level.

Two gaps remain:
1. **Queen drops stdin** after writing the JobSpec to the drone — it can never send `AuthResponse` back
2. **Drone doesn't stream stderr** from the Claude CLI — it collects all output after the process exits, so auth URLs never reach the channel

## Queen: Keep Stdin Open

In `supervisor.rs`, the supervisor writes the `JobSpec` to the drone's stdin then drops it. Instead, retain the stdin handle and send it along with the stdout reader task. When an `AuthRequest` arrives from the drone, write `AuthResponse { approved: true }` to stdin.

For now, auto-approve all auth requests — the user visiting the URL in their browser IS the approval action. The `AuthResponse` just unblocks the drone so the CLI can proceed once the user has authenticated.

The stdin handle needs to be accessible from the message-reading loop. The simplest approach: move stdin into the same blocking task that reads stdout, converting it from a write-once-and-close to a bidirectional channel. When the reader receives an `AuthRequest`, it writes the `AuthResponse` to stdin immediately.

## Drone: Stream Stderr and Detect Auth URL

In `drone.rs`, change `execute()` to:

1. Spawn the Claude CLI with stderr as `Stdio::piped()` (currently it's piped but read only after exit)
2. Spawn a task that reads stderr line-by-line in real-time
3. When a line contains a URL matching `https://claude.ai/` or `https://console.anthropic.com/`, send an `AuthRequest` via the channel
4. Block until `AuthResponse` comes back (the channel's `request_auth()` already does this)
5. Continue streaming stderr until the CLI exits

The URL detection is a simple string match — no regex needed. Claude CLI prints the auth URL on a line by itself or with a short prefix like "Visit: ".

The drone's `execute()` currently takes `&mut QueenChannel`. The stderr streaming task needs to send auth requests through the channel. Since the channel uses stdin/stdout (synchronous), the simplest approach is to check each stderr line in the main execute flow, pausing the output collection to do the auth handshake.

## Credential Mount (Bypass)

Zero code changes needed. The drone's `create_home()` already symlinks credentials from the real home directory. When running in Docker, mount the host credentials file:

```bash
docker run -it --rm -p 3100:3100 \
  -v kerrigan-data:/data \
  -v ~/.claude/.credentials.json:/root/.claude/.credentials.json:ro \
  kerrigan
```

If credentials exist, the Claude CLI skips the auth flow entirely and the drone never sends an `AuthRequest`.

## Files Changed

| File | Change |
|---|---|
| `src/queen/src/actors/supervisor.rs` | Keep stdin open after JobSpec write, send AuthResponse on AuthRequest |
| `src/drones/claude/base/src/drone.rs` | Stream stderr in real-time, detect auth URLs, call channel.request_auth() |

## What This Does NOT Cover

- Interactive auth approval (Queen auto-approves, user clicks the URL themselves)
- Credential persistence across container restarts (use credential mount for that)
- OAuth token refresh (Claude CLI handles this internally)
