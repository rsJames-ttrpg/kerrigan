# Creep Watcher Debounce Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop Creep from spamming "watcher event dropped: no available capacity" during bulk filesystem operations by debouncing events before they hit the channel.

**Architecture:** Replace `notify::recommended_watcher` with `notify-debouncer-mini` which coalesces rapid events over a 500ms window. Increase channel buffer from 256 to 1024. Rate-limit the overflow warning to at most once per 10 seconds.

**Tech Stack:** Rust, notify-debouncer-mini, tokio mpsc, tracing

**Spec:** `docs/specs/2026-04-03-creep-watcher-debounce-design.md`

---

### Task 1: Add `notify-debouncer-mini` dependency

**Files:**
- Modify: `src/creep/Cargo.toml`
- Modify: `src/creep/BUCK`
- Regenerate: `third-party/BUCK`

- [ ] **Step 1: Add the crate to Cargo.toml**

```bash
cd src/creep && cargo add notify-debouncer-mini@0.5
```

This should add `notify-debouncer-mini = "0.5"` to `[dependencies]` in `src/creep/Cargo.toml`.

- [ ] **Step 2: Regenerate third-party BUCK file**

```bash
cd /home/jackm/repos/kerrigan && ./tools/buckify.sh
```

Expected: completes without error. If `notify-debouncer-mini` has a build script that fails, add a fixup at `third-party/fixups/notify-debouncer-mini/fixups.toml` with `[buildscript] run = true`. This is unlikely — it's pure Rust.

- [ ] **Step 3: Add dep to creep BUCK file**

In `src/creep/BUCK`, add `"//third-party:notify-debouncer-mini"` to `CREEP_DEPS`:

```python
CREEP_DEPS = [
    "//third-party:anyhow",
    "//third-party:blake3",
    "//third-party:glob-match",
    "//third-party:ignore",
    "//third-party:notify",
    "//third-party:notify-debouncer-mini",
    "//third-party:prost",
    "//third-party:serde",
    "//third-party:tokio",
    "//third-party:toml",
    "//third-party:tonic",
    "//third-party:tonic-health",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
]
```

- [ ] **Step 4: Verify it compiles**

```bash
cd src/creep && cargo check
```

Expected: compiles with no errors.

- [ ] **Step 5: Commit**

```bash
git add src/creep/Cargo.toml src/creep/BUCK Cargo.lock third-party/BUCK
git commit -m "deps(creep): add notify-debouncer-mini for watcher debouncing"
```

---

### Task 2: Replace watcher with debouncer in `watcher.rs`

**Files:**
- Modify: `src/creep/src/watcher.rs`

- [ ] **Step 1: Update imports and watcher type**

Replace the existing imports and struct definition at the top of `src/creep/src/watcher.rs`:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::RecursiveMode;
use notify_debouncer_mini::{DebouncedEventKind, Debouncer, new_debouncer};
use tokio::sync::{Mutex, mpsc};

use crate::index::FileIndex;

/// Events produced by the file watcher.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

/// Watches one or more workspace directories for file changes, filtering out
/// gitignored paths using the `ignore` crate.
pub struct FileWatcher {
    /// One debouncer per workspace so we can stop watching individually.
    watchers: HashMap<PathBuf, Debouncer<notify::RecommendedWatcher>>,
    /// Gitignore matcher per workspace root, built when a workspace is registered.
    matchers: HashMap<PathBuf, ignore::gitignore::Gitignore>,
    /// Sender half of the event channel — cloned into each debouncer closure.
    tx: mpsc::Sender<WatchEvent>,
}
```

- [ ] **Step 2: Increase channel capacity in `new()`**

Replace the channel creation line in `FileWatcher::new()`:

```rust
    pub fn new(_index: FileIndex) -> (Arc<Mutex<Self>>, mpsc::Receiver<WatchEvent>) {
        let (tx, rx) = mpsc::channel::<WatchEvent>(1024);
        let watcher = Arc::new(Mutex::new(Self {
            watchers: HashMap::new(),
            matchers: HashMap::new(),
            tx,
        }));
        (watcher, rx)
    }
```

- [ ] **Step 3: Replace `recommended_watcher` with `new_debouncer` in `watch()`**

Replace the watcher creation block (lines 72–98) in `FileWatcher::watch()` with:

```rust
        // Create a debounced watcher. The closure runs on the debouncer's
        // background thread so we use `try_send` (non-blocking).
        let tx = self.tx.clone();
        let mut debouncer =
            new_debouncer(Duration::from_millis(500), move |res: notify_debouncer_mini::DebounceEventResult| {
                let events = match res {
                    Ok(events) => events,
                    Err(errs) => {
                        for e in errs {
                            tracing::error!("creep: notify watcher error: {e}");
                        }
                        return;
                    }
                };
                for event in events {
                    let watch_event = if event.path.exists() {
                        WatchEvent::Modified(event.path)
                    } else {
                        WatchEvent::Removed(event.path)
                    };
                    if let Err(e) = tx.try_send(watch_event) {
                        // Rate-limit overflow warnings to avoid log spam.
                        static LAST_WARN: std::sync::Mutex<Option<Instant>> =
                            std::sync::Mutex::new(None);
                        let mut last = LAST_WARN.lock().unwrap();
                        if last.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
                            tracing::warn!("creep: watcher event dropped: {e}");
                            *last = Some(Instant::now());
                        }
                    }
                }
            })?;

        debouncer.watcher().watch(workspace, RecursiveMode::Recursive)?;
        self.watchers.insert(workspace_buf, debouncer);
```

- [ ] **Step 4: Remove unused import**

Remove `use notify::{EventKind, RecursiveMode, Watcher as _};` — `EventKind` and `Watcher` are no longer used directly. `RecursiveMode` is already imported via the new imports.

- [ ] **Step 5: Run tests**

```bash
cd src/creep && cargo test
```

Expected: all existing tests pass. Tests inject events via `tx` directly and don't touch the watcher creation path.

- [ ] **Step 6: Verify buck2 build**

```bash
buck2 build root//src/creep:creep
```

Expected: builds clean.

- [ ] **Step 7: Commit**

```bash
git add src/creep/src/watcher.rs
git commit -m "fix(creep): debounce file watcher events to prevent channel overflow

Replace notify::recommended_watcher with notify-debouncer-mini (500ms
window). Increase channel buffer from 256 to 1024. Rate-limit the
overflow warning to at most once per 10 seconds."
```
