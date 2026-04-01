# Drone Auth Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable Claude CLI auth URLs to surface through Queen's logs so users can click them, and allow credential mounting as a bypass.

**Architecture:** The drone streams Claude CLI stderr line-by-line, detects auth URLs, and sends `AuthRequest` via the JSON-line channel to Queen. Queen's supervisor keeps the drone's stdin open (instead of dropping it after JobSpec) so it can send `AuthResponse` back. The `AuthRequest`/`AuthResponse` protocol and Queen's notifier handling already exist — only the stdin lifecycle and drone stderr streaming need implementation.

**Tech Stack:** Rust (tokio async I/O, `BufReader`, `AsyncBufReadExt`), drone-sdk protocol

---

## File Structure

| File | Responsibility |
|---|---|
| `src/queen/src/actors/supervisor.rs` | Retain stdin handle after JobSpec write; send AuthResponse on AuthRequest |
| `src/drones/claude/base/src/drone.rs` | Stream stderr in real-time; detect auth URLs; call channel.request_auth() |

---

### Task 1: Queen retains drone stdin and responds to AuthRequest

**Files:**
- Modify: `src/queen/src/actors/supervisor.rs`

The supervisor currently writes the JobSpec to stdin then drops it (line 183: `drop(stdin)`). We need to:
1. Keep stdin alive by storing it in the `DroneHandle`
2. When an `AuthRequest` arrives in `drain_protocol_messages`, write an `AuthResponse` to stdin

- [ ] **Step 1: Add stdin_tx to DroneHandle**

In `src/queen/src/actors/supervisor.rs`, change the `DroneHandle` struct to include a channel for sending messages to the drone's stdin:

```rust
#[allow(dead_code)]
struct DroneHandle {
    job_run_id: String,
    drone_type: String,
    process: Child,
    started_at: Instant,
    timeout: Duration,
    last_activity: Instant,
    protocol_rx: mpsc::Receiver<DroneMessage>,
    stdin_tx: Option<mpsc::Sender<QueenMessage>>,
}
```

- [ ] **Step 2: Modify spawn_drone to keep stdin open**

In the `spawn_drone` function, replace the current stdin blocking task (lines 160-184) with one that writes the JobSpec and then continues reading from a channel:

```rust
    // Bidirectional stdin: write JobSpec, then keep open for AuthResponse etc.
    let stdin = process.stdin.take().expect("stdin was piped");
    let job_spec = JobSpec {
        job_run_id: request.job_run_id.clone(),
        repo_url: request.repo_url.clone(),
        branch: request.branch.clone(),
        task: request.task.clone(),
        config: request.job_config.clone(),
    };
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<QueenMessage>(16);
    let job_run_id_for_stdin = request.job_run_id.clone();
    tokio::task::spawn_blocking(move || {
        let fd: std::os::fd::OwnedFd = stdin.into_owned_fd().expect("take stdin fd");
        let mut stdin = std::process::ChildStdin::from(fd);
        // Write the initial JobSpec
        let msg = QueenMessage::Job(job_spec);
        match serde_json::to_writer(&mut stdin, &msg) {
            Ok(()) => {
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
            }
            Err(e) => {
                tracing::error!(job_run_id = %job_run_id_for_stdin, error = %e, "failed to write job spec to drone stdin");
                return;
            }
        }
        // Keep stdin open, forwarding messages from the channel
        while let Some(msg) = stdin_rx.blocking_recv() {
            match serde_json::to_writer(&mut stdin, &msg) {
                Ok(()) => {
                    let _ = stdin.write_all(b"\n");
                    let _ = stdin.flush();
                }
                Err(e) => {
                    tracing::error!(job_run_id = %job_run_id_for_stdin, error = %e, "failed to write message to drone stdin");
                    break;
                }
            }
        }
        // Channel closed or error — stdin drops here, closing the pipe
    });
```

- [ ] **Step 3: Store stdin_tx in DroneHandle**

Update the `DroneHandle` construction (around line 224):

```rust
    let handle = DroneHandle {
        job_run_id: request.job_run_id.clone(),
        drone_type: request.drone_type.clone(),
        process,
        started_at: now,
        timeout: default_timeout,
        last_activity: now,
        protocol_rx,
        stdin_tx: Some(stdin_tx),
    };
```

- [ ] **Step 4: Send AuthResponse in drain_protocol_messages**

In the `drain_protocol_messages` function, in the `DroneMessage::AuthRequest` branch (around line 265), after notifying, send an `AuthResponse` back:

```rust
                        DroneMessage::AuthRequest(auth) => {
                            tracing::info!(
                                job_run_id = %id,
                                url = %auth.url,
                                message = %auth.message,
                                "drone auth request"
                            );
                            notifier
                                .notify(QueenEvent::AuthRequested {
                                    job_run_id: id.clone(),
                                    url: auth.url,
                                    message: auth.message,
                                })
                                .await;
                            // Auto-approve: user clicking the URL is the approval
                            if let Some(tx) = &handle.stdin_tx {
                                let response = QueenMessage::AuthResponse(
                                    drone_sdk::protocol::AuthResponse { approved: true },
                                );
                                if tx.send(response).await.is_err() {
                                    tracing::warn!(job_run_id = %id, "failed to send auth response (stdin closed)");
                                }
                            }
                        }
```

Note: `drain_protocol_messages` currently uses `try_recv` (non-async). The `tx.send()` is async. Since the function is already `async`, and we're using `mpsc::Sender` (not `blocking_send`), this works. However, `tx.send()` is on a bounded channel (capacity 16) so it should never actually block.

- [ ] **Step 5: Build and verify**

```bash
buck2 build root//src/queen:queen
```

Expected: BUILD SUCCEEDED.

- [ ] **Step 6: Run Queen tests**

```bash
cd src/queen && cargo test
```

Expected: all tests pass. The existing tests don't exercise the supervisor's stdin handling directly (it requires a real child process), so they should be unaffected.

- [ ] **Step 7: Commit**

```bash
git add src/queen/src/actors/supervisor.rs
git commit -m "feat(queen): keep drone stdin open for AuthResponse"
```

---

### Task 2: Drone streams stderr and detects auth URLs

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`

The drone currently calls `child.wait_with_output()` which collects all stdout+stderr after the process exits. We need to stream stderr line-by-line while the process runs, detect auth URLs, and call `channel.request_auth()`.

The approach: spawn a background task to read stderr lines and send detected auth URLs through a tokio mpsc channel. The main flow uses `tokio::select!` to handle auth URLs (calling the synchronous `channel.request_auth()`) while waiting for the CLI process to exit. Stdout is also read in a background task.

- [ ] **Step 1: Replace execute() method**

Replace the entire `execute` method in `src/drones/claude/base/src/drone.rs` with:

```rust
    async fn execute(
        &self,
        env: &DroneEnvironment,
        channel: &mut QueenChannel,
    ) -> Result<DroneOutput> {
        let task = fs::read_to_string(env.home.join(".task"))
            .await
            .context("failed to read .task file")?;

        let settings_path = env.home.join(".claude/settings.json");
        let claude_md_path = env.home.join("CLAUDE.md");

        let claude_bin = env.home.join(".claude/bin/claude");
        let mut child = Command::new(&claude_bin)
            .arg("--print")
            .arg("--output-format")
            .arg("json")
            .arg("--dangerously-skip-permissions")
            .arg("--settings")
            .arg(&settings_path)
            .arg("--append-system-prompt-file")
            .arg(&claude_md_path)
            .env("HOME", &env.home)
            .current_dir(&env.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to spawn claude CLI")?;

        // Write task to stdin, then close it
        {
            use tokio::io::AsyncWriteExt;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("failed to acquire stdin pipe"))?;
            stdin
                .write_all(task.as_bytes())
                .await
                .context("failed to write task to claude stdin")?;
        }

        // Stream stderr in a background task to detect auth URLs
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to acquire stderr pipe"))?;
        let (auth_tx, mut auth_rx) = tokio::sync::mpsc::channel::<String>(4);
        let stderr_handle = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(stderr_line = %line, "claude CLI stderr");
                // Detect auth URLs
                if line.contains("claude.ai/") || line.contains("console.anthropic.com/") {
                    // Extract URL from the line
                    if let Some(url) = extract_url(&line) {
                        let _ = auth_tx.send(url).await;
                    }
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        // Read stdout + wait for exit, but also check for auth URLs from stderr
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to acquire stdout pipe"))?;

        let timeout_duration = Duration::from_secs(7200);

        // Spawn stdout reader
        let stdout_handle = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            tokio::io::BufReader::new(stdout)
                .read_to_end(&mut buf)
                .await
                .map(|_| buf)
        });

        // Poll for auth URLs while waiting for process to finish, with timeout
        let result = timeout(timeout_duration, async {
            loop {
                tokio::select! {
                    Some(url) = auth_rx.recv() => {
                        tracing::info!(url = %url, "claude CLI requesting auth");
                        match channel.request_auth(&url, "Claude CLI requires authentication") {
                            Ok(true) => tracing::info!("auth approved, continuing"),
                            Ok(false) => tracing::warn!("auth denied"),
                            Err(e) => tracing::warn!(error = %e, "auth request failed"),
                        }
                    }
                    status = child.wait() => {
                        let status = status.context("failed to wait for claude process")?;
                        let exit_code = status.code().unwrap_or(-1);

                        let stdout_bytes = stdout_handle.await
                            .context("stdout reader panicked")?
                            .context("failed to read stdout")?;
                        let stderr_text = stderr_handle.await.unwrap_or_default();

                        if !stderr_text.is_empty() {
                            tracing::debug!(stderr = %stderr_text, "claude CLI stderr");
                        }
                        if exit_code != 0 {
                            tracing::warn!(exit_code, stderr = %stderr_text, "claude CLI exited with non-zero status");
                        }

                        let stdout = String::from_utf8_lossy(&stdout_bytes);
                        let conversation = match serde_json::from_str::<serde_json::Value>(&stdout) {
                            Ok(v) => v,
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    stdout_len = stdout.len(),
                                    "failed to parse claude stdout as JSON; returning raw output"
                                );
                                serde_json::json!({
                                    "raw_output": stdout.to_string(),
                                    "parse_error": e.to_string()
                                })
                            }
                        };

                        let git_refs = collect_git_refs(&env.workspace).await;

                        return Ok(DroneOutput {
                            exit_code,
                            conversation,
                            artifacts: vec![],
                            git_refs,
                        });
                    }
                }
            }
        }).await;

        match result {
            Ok(output) => output,
            Err(_) => {
                tracing::error!("claude CLI timed out after {:?}", timeout_duration);
                anyhow::bail!("claude CLI timed out after {:?}", timeout_duration);
            }
        }
    }
```

- [ ] **Step 3: Add extract_url helper function**

Add this function after `collect_git_refs` in `drone.rs`:

```rust
/// Extract a URL from a line of text.
fn extract_url(line: &str) -> Option<String> {
    // Find the start of https://
    let start = line.find("https://")?;
    // URL ends at whitespace or end of string
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
```

- [ ] **Step 4: Remove `_channel` underscore prefix**

The `execute` method signature already has `channel: &mut QueenChannel` (without underscore) in the new code above. Verify the trait definition matches:

In `src/drone-sdk/src/runner.rs`, the trait should already have:
```rust
async fn execute(&self, env: &DroneEnvironment, channel: &mut QueenChannel) -> Result<DroneOutput>;
```

No change needed — just make sure the parameter name is `channel` not `_channel` in the implementation.

- [ ] **Step 5: Build and test**

```bash
buck2 build root//src/drones/claude/base:claude-drone
buck2 test root//src/drones/claude/base:claude-drone-test
```

Expected: BUILD SUCCEEDED, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): stream stderr to detect auth URLs, send AuthRequest to queen"
```

---

### Task 3: E2E test in container

- [ ] **Step 1: Rebuild container**

```bash
./deploy/dev/build.sh
```

- [ ] **Step 2: Run without credentials (test auth flow)**

```bash
docker run -it --rm -p 3100:3100 -v /tmp/kerrigan-auth-test:/data kerrigan
```

Submit a job (from another terminal):
```bash
HATCHERY_ID=$(curl -s http://localhost:3100/api/hatcheries | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")
DEF_ID=$(curl -s -X POST http://localhost:3100/api/jobs/definitions -H 'Content-Type: application/json' -d '{"name":"auth-test","description":"test auth","config":{"drone_type":"claude-drone","repo_url":"https://github.com/rsJames-ttrpg/kerrigan.git","task":"Say hello"}}' | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
RUN_ID=$(curl -s -X POST http://localhost:3100/api/jobs/runs -H 'Content-Type: application/json' -d "{\"definition_id\":\"$DEF_ID\",\"triggered_by\":\"manual\"}" | python3 -c "import sys,json; print(json.load(sys.stdin)['id'])")
curl -s -X PUT "http://localhost:3100/api/hatcheries/$HATCHERY_ID/jobs/$RUN_ID"
```

Expected in container logs:
- `drone auth request` with the URL
- `drone requires auth - visit URL to approve` at WARN level
- The URL should be clickable in the terminal

- [ ] **Step 3: Test credential mount (bypass auth)**

```bash
docker run -it --rm -p 3100:3100 \
  -v /tmp/kerrigan-creds-test:/data \
  -v ~/.claude/.credentials.json:/root/.claude/.credentials.json:ro \
  kerrigan
```

Submit a job as above. Expected: no auth request — the CLI should proceed directly using the mounted credentials.
