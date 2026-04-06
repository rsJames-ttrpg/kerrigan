# Multi-Drone-Type Concurrency

## Context

The Queen currently manages two drone types (`claude-drone` and `native-drone`) but treats concurrency as a single global limit. The `drone_type` is already read from job config and used to select the binary, but there's no way to set per-type concurrency limits or per-type operational parameters (timeout, stall threshold).

This means running both drone types simultaneously is limited — you can't say "max 2 claude drones (expensive, long-running) and 4 native drones (cheap, fast)" independently. A burst of native-drone jobs could consume all slots and starve claude-drone jobs, or vice versa.

## Design

### Config Shape

Add a `drones` map to `[queen]` in `hatchery.toml`. Keys are drone binary names (matching files in `drone_dir`). All per-type fields are optional; omitted fields fall back to the global `queen.*` value.

```toml
[queen]
name = "dev-hatchery"
max_concurrency = 4          # Global ceiling — total active drones never exceeds this
drone_dir = "./drones"
drone_timeout = "2h"         # Global default timeout
stall_threshold = 300        # Global default stall threshold

[queen.drones.claude-drone]
max_concurrency = 2          # At most 2 claude-drone instances
drone_timeout = "2h"
stall_threshold = 300

[queen.drones.native-drone]
max_concurrency = 4          # At most 4 native-drone instances
drone_timeout = "30m"        # Native drones are faster, shorter timeout
stall_threshold = 120        # Shorter stall window
```

### Concurrency Model

Two-level concurrency control:

1. **Global cap** (`queen.max_concurrency`): Total active drones across all types never exceeds this. Existing behavior, unchanged.
2. **Per-type cap** (`queen.drones.<type>.max_concurrency`): Active drones of a given type never exceed this. New check.

Both limits must be satisfied to spawn. If either is hit, the request is queued.

When draining the queue, each queued request is checked against both its per-type limit and the global limit. This means if a native-drone slot opens but global capacity is full, nothing dequeues. If global capacity opens but the specific type is at its limit, only requests for other types can dequeue.

### Unknown Drone Types

Jobs specifying a `drone_type` not listed in `[queen.drones.*]` use global defaults for all settings. Their concurrency is bounded only by `queen.max_concurrency`.

### Config Struct Changes

**File:** `src/queen/src/config.rs`

Add `DroneTypeConfig`:

```rust
#[derive(Debug, Deserialize, Clone)]
pub struct DroneTypeConfig {
    pub max_concurrency: Option<i32>,
    pub drone_timeout: Option<String>,
    pub stall_threshold: Option<u64>,
}
```

Add to `QueenConfig`:

```rust
#[serde(default)]
pub drones: HashMap<String, DroneTypeConfig>,
```

Add a resolver method on `QueenConfig`:

```rust
impl QueenConfig {
    /// Resolve the effective config for a drone type.
    /// Per-type values override globals; missing per-type values fall back to globals.
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

`EffectiveDroneConfig` holds resolved values (no Options except `max_concurrency` which is `None` for unconfigured types — meaning no per-type limit).

### Supervisor Changes

**File:** `src/queen/src/actors/supervisor.rs`

**Spawn gate** — extend the existing `active.len() < max_concurrency` check:

```rust
fn can_spawn(&self, drone_type: &str) -> bool {
    // Global limit
    if self.active.len() as i32 >= self.max_concurrency {
        return false;
    }
    // Per-type limit (if configured)
    if let Some(type_limit) = self.type_max_concurrency(drone_type) {
        let type_count = self.active.values()
            .filter(|d| d.drone_type == drone_type)
            .count() as i32;
        if type_count >= type_limit {
            return false;
        }
    }
    true
}
```

**Queue draining** — when capacity opens, iterate the queue and dequeue the first request whose type has capacity:

```rust
fn drain_queue(&mut self) {
    let mut i = 0;
    while i < self.queue.len() {
        if self.can_spawn(&self.queue[i].drone_type) {
            let request = self.queue.remove(i).unwrap();
            self.spawn_drone(request);
        } else {
            i += 1;
        }
    }
}
```

This preserves FIFO ordering within each type while allowing other types to proceed when one type is at capacity.

**Timeout/stall resolution** — when constructing `DroneHandle`, use the resolved per-type config:

```rust
let effective = config.queen.effective_drone_config(&request.drone_type);
let timeout = parse_duration(&effective.drone_timeout)?;
let stall_threshold = Duration::from_secs(effective.stall_threshold);
```

### Validation

**File:** `src/queen/src/config.rs` — extend `Config::validate()`:

```rust
for (name, drone_config) in &self.queen.drones {
    if let Some(max) = drone_config.max_concurrency {
        if max <= 0 {
            anyhow::bail!("queen.drones.{name}.max_concurrency must be greater than 0");
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
            anyhow::bail!("queen.drones.{name}.stall_threshold must be greater than 0");
        }
    }
}
```

### Files to Modify

1. `src/queen/src/config.rs` — Add `DroneTypeConfig`, `EffectiveDroneConfig`, resolver method, validation, tests
2. `src/queen/src/actors/supervisor.rs` — Per-type spawn gate, queue draining, timeout/stall resolution
3. `hatchery.toml` — Add `[queen.drones.*]` sections for both drone types

### Verification

1. `cd src/queen && cargo test` — all existing + new config tests pass
2. `cargo clippy` — no warnings
3. Manual test with `hatchery.toml` configured for both drone types:
   - Set `max_concurrency = 3`, `claude-drone.max_concurrency = 1`, `native-drone.max_concurrency = 2`
   - Submit 2 claude-drone jobs — verify only 1 spawns, second queues
   - Submit 3 native-drone jobs — verify only 2 spawn (limited by per-type), third queues even though global has room
   - Complete a claude-drone — verify queued claude-drone spawns
4. Test unknown drone type: submit job with `drone_type = "custom-drone"` — verify it uses global defaults, spawns if global capacity available
