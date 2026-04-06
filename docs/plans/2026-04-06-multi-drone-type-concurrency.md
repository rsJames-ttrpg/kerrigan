# Multi-Drone-Type Concurrency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-drone-type concurrency limits and operational config (timeout, stall threshold) to the Queen, so claude-drone and native-drone can run simultaneously with independent resource constraints under a shared global ceiling.

**Architecture:** Extend `QueenConfig` with a `drones: HashMap<String, DroneTypeConfig>` deserialized from `[queen.drones.<type>]` TOML tables. The Supervisor's spawn gate checks both the global `max_concurrency` and the per-type limit. Timeout and stall threshold are resolved per-type with global fallback. Queue draining iterates by index and skips type-blocked items to prevent head-of-line blocking.

**Tech Stack:** Rust (edition 2024), serde/toml deserialization, tokio actors

**Spec:** `docs/specs/2026-04-06-multi-drone-type-concurrency-design.md`

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `src/queen/src/config.rs` | Modify | Add `DroneTypeConfig`, `EffectiveDroneConfig`, resolver, validation, tests |
| `src/queen/src/actors/supervisor.rs` | Modify | Per-type spawn gate, per-type timeout/stall, queue drain rewrite |
| `src/queen/src/main.rs` | Modify | Pass `drones` config to supervisor |
| `hatchery.toml` | Modify | Add `[queen.drones.*]` sections |

---

### Task 1: Add `DroneTypeConfig` and `EffectiveDroneConfig` structs

**Files:**
- Modify: `src/queen/src/config.rs:80-99` (after `QueenConfig`)

- [ ] **Step 1: Add the `DroneTypeConfig` struct and `drones` field**

After the closing `}` of `QueenConfig` (line 99), add:

```rust
/// Per-drone-type configuration. All fields are optional — omitted values
/// fall back to the corresponding global `queen.*` setting.
#[derive(Debug, Deserialize, Clone)]
pub struct DroneTypeConfig {
    pub max_concurrency: Option<i32>,
    pub drone_timeout: Option<String>,
    pub stall_threshold: Option<u64>,
}
```

Inside `QueenConfig` (after `default_repo_url` on line 98), add:

```rust
    /// Per-drone-type configuration, keyed by drone binary name.
    #[serde(default)]
    pub drones: HashMap<String, DroneTypeConfig>,
```

- [ ] **Step 2: Add `EffectiveDroneConfig` and the resolver method**

After `DroneTypeConfig`, add:

```rust
/// Resolved drone config with global fallbacks applied.
#[derive(Debug, Clone)]
pub struct EffectiveDroneConfig {
    /// `None` means no per-type limit — only the global limit applies.
    pub max_concurrency: Option<i32>,
    pub drone_timeout: String,
    pub stall_threshold: u64,
}

impl QueenConfig {
    /// Resolve effective config for a drone type.
    /// Per-type values override globals; missing per-type values use globals.
    pub fn effective_drone_config(&self, drone_type: &str) -> EffectiveDroneConfig {
        let type_config = self.drones.get(drone_type);
        EffectiveDroneConfig {
            max_concurrency: type_config.and_then(|c| c.max_concurrency),
            drone_timeout: type_config
                .and_then(|c| c.drone_timeout.clone())
                .unwrap_or_else(|| self.drone_timeout.clone()),
            stall_threshold: type_config
                .and_then(|c| c.stall_threshold)
                .unwrap_or(self.stall_threshold),
        }
    }
}
```

- [ ] **Step 3: Run `cargo check` to verify it compiles**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo check`
Expected: compiles successfully (no code references the new types yet, so no breakage)

- [ ] **Step 4: Commit**

```bash
git add src/queen/src/config.rs
git commit -m "feat(queen): add DroneTypeConfig and EffectiveDroneConfig structs"
```

---

### Task 2: Add per-type config validation

**Files:**
- Modify: `src/queen/src/config.rs:233-270` (`Config::validate()`)

- [ ] **Step 1: Write failing tests for per-type validation**

Add these tests at the end of the `#[cfg(test)] mod tests` block (before the final `}`):

```rust
    #[test]
    fn test_validate_drone_type_zero_concurrency() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[queen.drones.bad-drone]
max_concurrency = 0
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.drones.bad-drone.max_concurrency must be greater than 0")
        );
    }

    #[test]
    fn test_validate_drone_type_invalid_timeout() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[queen.drones.bad-drone]
drone_timeout = "not-valid"
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.drones.bad-drone.drone_timeout 'not-valid' is not a valid duration")
        );
    }

    #[test]
    fn test_validate_drone_type_zero_stall() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[queen.drones.bad-drone]
stall_threshold = 0
"#,
        );
        let err = Config::load(f.path()).unwrap_err();
        assert!(
            err.to_string()
                .contains("queen.drones.bad-drone.stall_threshold must be greater than 0")
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo test test_validate_drone_type`
Expected: all 3 tests FAIL (validation not implemented yet)

- [ ] **Step 3: Implement the validation**

In `Config::validate()`, add this block before the final `Ok(())` (before line 270):

```rust
        for (name, drone_config) in &self.queen.drones {
            if let Some(max) = drone_config.max_concurrency {
                if max <= 0 {
                    anyhow::bail!(
                        "queen.drones.{name}.max_concurrency must be greater than 0"
                    );
                }
            }
            if let Some(ref timeout) = drone_config.drone_timeout {
                if crate::parse_duration(timeout).is_err() {
                    anyhow::bail!(
                        "queen.drones.{name}.drone_timeout '{timeout}' is not a valid duration"
                    );
                }
            }
            if let Some(stall) = drone_config.stall_threshold {
                if stall == 0 {
                    anyhow::bail!(
                        "queen.drones.{name}.stall_threshold must be greater than 0"
                    );
                }
            }
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo test test_validate_drone_type`
Expected: all 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/config.rs
git commit -m "feat(queen): validate per-drone-type config fields"
```

---

### Task 3: Add config parsing and resolver tests

**Files:**
- Modify: `src/queen/src/config.rs` (test module)

- [ ] **Step 1: Write tests for parsing and resolution**

Add these tests at the end of the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn test_parse_drone_type_config() {
        let f = write_toml(
            r#"
[queen]
name = "test"

[queen.drones.claude-drone]
max_concurrency = 2
drone_timeout = "2h"
stall_threshold = 300

[queen.drones.native-drone]
max_concurrency = 4
drone_timeout = "30m"
stall_threshold = 120
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert_eq!(config.queen.drones.len(), 2);

        let claude = &config.queen.drones["claude-drone"];
        assert_eq!(claude.max_concurrency, Some(2));
        assert_eq!(claude.drone_timeout.as_deref(), Some("2h"));
        assert_eq!(claude.stall_threshold, Some(300));

        let native = &config.queen.drones["native-drone"];
        assert_eq!(native.max_concurrency, Some(4));
        assert_eq!(native.drone_timeout.as_deref(), Some("30m"));
        assert_eq!(native.stall_threshold, Some(120));
    }

    #[test]
    fn test_effective_drone_config_with_overrides() {
        let f = write_toml(
            r#"
[queen]
name = "test"
drone_timeout = "2h"
stall_threshold = 300

[queen.drones.claude-drone]
max_concurrency = 2
drone_timeout = "4h"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        let effective = config.queen.effective_drone_config("claude-drone");

        assert_eq!(effective.max_concurrency, Some(2));
        assert_eq!(effective.drone_timeout, "4h");
        // stall_threshold not overridden — falls back to global
        assert_eq!(effective.stall_threshold, 300);
    }

    #[test]
    fn test_effective_drone_config_unknown_type() {
        let f = write_toml(
            r#"
[queen]
name = "test"
drone_timeout = "2h"
stall_threshold = 300
"#,
        );
        let config = Config::load(f.path()).unwrap();
        let effective = config.queen.effective_drone_config("unknown-drone");

        assert!(effective.max_concurrency.is_none());
        assert_eq!(effective.drone_timeout, "2h");
        assert_eq!(effective.stall_threshold, 300);
    }

    #[test]
    fn test_drones_field_defaults_to_empty() {
        let f = write_toml(
            r#"
[queen]
name = "test"
"#,
        );
        let config = Config::load(f.path()).unwrap();
        assert!(config.queen.drones.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo test test_parse_drone_type_config test_effective_drone_config test_drones_field`
Expected: all 4 tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/queen/src/config.rs
git commit -m "test(queen): add config parsing and resolver tests for per-drone-type config"
```

---

### Task 4: Add per-type stall threshold to `DroneHandle`

**Files:**
- Modify: `src/queen/src/actors/supervisor.rs:48-60` (`DroneHandle`), `src/queen/src/actors/supervisor.rs:300-313` (handle construction), `src/queen/src/actors/supervisor.rs:507-511` (`check_drones` signature), `src/queen/src/actors/supervisor.rs:675` (stall check)

- [ ] **Step 1: Add `stall_threshold` field to `DroneHandle`**

In the `DroneHandle` struct (line 49-60), add after `stall_notified`:

```rust
    stall_threshold: Duration,
```

- [ ] **Step 2: Update `spawn_drone` to accept and store `stall_threshold`**

Change the `spawn_drone` function signature (line 125-131) to add a `stall_threshold: Duration` parameter:

```rust
async fn spawn_drone(
    client: &NydusClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
    request: SpawnRequest,
    default_timeout: Duration,
    stall_threshold: Duration,
    drone_dir: &Path,
)
```

In the `DroneHandle` construction (line 302-313), add `stall_threshold`:

```rust
    let handle = DroneHandle {
        job_run_id: request.job_run_id.clone(),
        drone_type: request.drone_type.clone(),
        process,
        started_at: now,
        timeout: default_timeout,
        last_activity: now,
        stall_notified: false,
        stall_threshold,
        protocol_rx,
        stderr_rx,
        stdin_tx: Some(stdin_tx),
    };
```

- [ ] **Step 3: Update `check_drones` to use per-handle stall threshold**

Remove the `stall_threshold: Duration` parameter from `check_drones` (line 507-511):

```rust
async fn check_drones(
    client: &NydusClient,
    notifier: &Arc<dyn Notifier>,
    active: &mut HashMap<String, DroneHandle>,
)
```

Change line 675 from:

```rust
        if now.duration_since(handle.last_activity) > stall_threshold && !handle.stall_notified {
```

to:

```rust
        if now.duration_since(handle.last_activity) > handle.stall_threshold && !handle.stall_notified {
```

- [ ] **Step 4: Update all call sites in `run()`**

In the `run()` function:

Line 84 — add `stall_threshold` argument to `spawn_drone`:
```rust
                    spawn_drone(&client, &notifier, &mut active, request, default_timeout, stall_threshold, &drone_dir).await;
```

Line 99 — remove `stall_threshold` from `check_drones`:
```rust
                check_drones(&client, &notifier, &mut active).await;
```

Line 103 — add `stall_threshold` argument to `spawn_drone`:
```rust
                        spawn_drone(&client, &notifier, &mut active, request, default_timeout, stall_threshold, &drone_dir).await;
```

- [ ] **Step 5: Run `cargo check` to verify it compiles**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo check`
Expected: compiles successfully

- [ ] **Step 6: Commit**

```bash
git add src/queen/src/actors/supervisor.rs
git commit -m "refactor(queen): store stall_threshold per DroneHandle"
```

---

### Task 5: Add per-type spawn gate and pass `drones` config to supervisor

**Files:**
- Modify: `src/queen/src/actors/supervisor.rs:1-17` (imports), `src/queen/src/actors/supervisor.rs:62-123` (`run()`)
- Modify: `src/queen/src/main.rs:100-123` (supervisor wiring)

- [ ] **Step 1: Add `can_spawn` and `resolve_timeout_stall` helpers**

Add these free functions in `supervisor.rs` after the `store_session_artifact` function (after line 46) and before `DroneHandle`:

```rust
fn can_spawn(
    active: &HashMap<String, DroneHandle>,
    max_concurrency: i32,
    drones: &HashMap<String, DroneTypeConfig>,
    drone_type: &str,
) -> bool {
    if active.len() as i32 >= max_concurrency {
        return false;
    }
    if let Some(type_config) = drones.get(drone_type) {
        if let Some(type_limit) = type_config.max_concurrency {
            let type_count = active
                .values()
                .filter(|d| d.drone_type == drone_type)
                .count() as i32;
            if type_count >= type_limit {
                return false;
            }
        }
    }
    true
}

fn resolve_timeout_stall(
    drones: &HashMap<String, DroneTypeConfig>,
    drone_type: &str,
    default_timeout: Duration,
    default_stall: Duration,
) -> (Duration, Duration) {
    let type_config = drones.get(drone_type);
    let timeout = type_config
        .and_then(|c| c.drone_timeout.as_ref())
        .and_then(|t| crate::parse_duration(t).ok())
        .unwrap_or(default_timeout);
    let stall = type_config
        .and_then(|c| c.stall_threshold)
        .map(Duration::from_secs)
        .unwrap_or(default_stall);
    (timeout, stall)
}
```

Add the import at the top of the file:

```rust
use crate::config::DroneTypeConfig;
```

- [ ] **Step 2: Add `drones` parameter to `run()` and update spawn gate**

Add `drones: HashMap<String, DroneTypeConfig>` parameter to `run()` (after `stall_threshold`):

```rust
pub async fn run(
    client: NydusClient,
    notifier: Arc<dyn Notifier>,
    max_concurrency: i32,
    default_timeout: Duration,
    stall_threshold: Duration,
    drones: HashMap<String, DroneTypeConfig>,
    drone_dir: PathBuf,
    mut spawn_rx: mpsc::Receiver<SpawnRequest>,
    mut status_rx: mpsc::Receiver<(StatusQuery, oneshot::Sender<StatusResponse>)>,
    token: CancellationToken,
)
```

Update the spawn gate (line 82-88) to use `can_spawn` and per-type resolution:

```rust
            Some(request) = spawn_rx.recv() => {
                if can_spawn(&active, max_concurrency, &drones, &request.drone_type) {
                    let (timeout, stall) = resolve_timeout_stall(&drones, &request.drone_type, default_timeout, stall_threshold);
                    spawn_drone(&client, &notifier, &mut active, request, timeout, stall, &drone_dir).await;
                } else {
                    tracing::info!(job_run_id = %request.job_run_id, drone_type = %request.drone_type, "concurrency limit reached, queueing");
                    queue.push_back(request);
                }
            }
```

- [ ] **Step 3: Rewrite queue draining to skip type-blocked items**

Replace lines 101-107 (the `while` loop after `check_drones`) with:

```rust
                {
                    let mut i = 0;
                    while i < queue.len() {
                        if !can_spawn(&active, max_concurrency, &drones, &queue[i].drone_type) {
                            i += 1;
                            continue;
                        }
                        let request = queue.remove(i).unwrap();
                        let (timeout, stall) = resolve_timeout_stall(&drones, &request.drone_type, default_timeout, stall_threshold);
                        spawn_drone(&client, &notifier, &mut active, request, timeout, stall, &drone_dir).await;
                        if active.len() as i32 >= max_concurrency {
                            break;
                        }
                    }
                }
```

- [ ] **Step 4: Update `main.rs` to pass `drones` to supervisor**

In `main.rs`, after `let drone_dir = PathBuf::from(&config.queen.drone_dir);` (line 108), add:

```rust
    let drones = config.queen.drones.clone();
```

Update the `supervisor::run` call (lines 111-122) to include `drones`:

```rust
        actors::supervisor::run(
            supervisor_client,
            supervisor_notifier,
            max_concurrency,
            default_timeout,
            stall_threshold,
            drones,
            drone_dir,
            spawn_rx,
            status_query_rx,
            supervisor_token,
        )
```

- [ ] **Step 5: Run `cargo check` to verify it compiles**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo check`
Expected: compiles successfully

- [ ] **Step 6: Run all existing tests**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo test`
Expected: all tests PASS (existing behavior unchanged)

- [ ] **Step 7: Commit**

```bash
git add src/queen/src/actors/supervisor.rs src/queen/src/main.rs
git commit -m "feat(queen): per-drone-type concurrency limits and config resolution"
```

---

### Task 6: Update `hatchery.toml` with drone type sections

**Files:**
- Modify: `hatchery.toml`

- [ ] **Step 1: Add `[queen.drones.*]` sections**

After `default_repo_url` (line 7) and before `[creep]` (line 9), add:

```toml

[queen.drones.claude-drone]
max_concurrency = 2
drone_timeout = "2h"
stall_threshold = 300

[queen.drones.native-drone]
max_concurrency = 4
drone_timeout = "30m"
stall_threshold = 120
```

- [ ] **Step 2: Run clippy for final check**

Run: `cd /home/jackm/repos/kerrigan/src/queen && cargo clippy`
Expected: no warnings or errors

- [ ] **Step 3: Commit**

```bash
git add hatchery.toml
git commit -m "feat(queen): configure per-drone-type concurrency in hatchery.toml"
```
