# Creep Watcher Debounce

## Problem

Creep's file watcher spams `watcher event dropped: no available capacity` during bulk filesystem operations (git checkout, cargo/buck2 builds). The 256-event channel fills instantly because every individual filesystem event fires a separate `try_send` with no debouncing or batching.

## Solution

Replace `notify::recommended_watcher` with `notify-debouncer-mini`, which coalesces rapid filesystem events over a configurable window before invoking the callback. This reduces event volume at the source.

## Design

### Data flow (before)

```
FS event -> notify callback (1:1) -> try_send -> channel(256) -> process_events
```

### Data flow (after)

```
FS events -> debouncer (500ms window) -> callback (batched) -> try_send per path -> channel(1024) -> process_events
```

### Changes to `src/creep/src/watcher.rs`

**Watcher type:**
- `watchers: HashMap<PathBuf, notify::RecommendedWatcher>` becomes `watchers: HashMap<PathBuf, notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>>`

**`FileWatcher::new()`:**
- Channel capacity increases from 256 to 1024

**`FileWatcher::watch()`:**
- `notify::recommended_watcher(callback)` becomes `notify_debouncer_mini::new_debouncer(Duration::from_millis(500), callback)`
- Callback receives `Result<Vec<DebouncedEvent>, Vec<Error>>` instead of `Result<Event>`
- `debouncer.watcher().watch(path, Recursive)` instead of `watcher.watch(path, Recursive)`

**Event kind mapping:**
- `notify-debouncer-mini` does not distinguish Create/Modify/Remove
- Map debounced events using path existence: `path.exists()` -> `WatchEvent::Modified`, `!path.exists()` -> `WatchEvent::Removed`
- `WatchEvent::Created` variant stays in the enum (used by tests that inject events directly) but is no longer produced by the watcher callback

**Rate-limited warning:**
- Replace `eprintln!("creep: watcher event dropped: {e}")` with `tracing::warn!`
- Add rate limiting: log at most once per 10 seconds using a `std::sync::Mutex<Option<Instant>>` static

### Dependency changes

- Add `notify-debouncer-mini` to `src/creep/Cargo.toml`
- Run `./tools/buckify.sh` to regenerate `third-party/BUCK`
- Add `"//third-party:notify-debouncer-mini"` to `src/creep/BUCK` deps
- Add fixup in `third-party/fixups/` if needed (unlikely — pure Rust crate)

### Test impact

Existing tests inject events directly via the `tx` sender channel, bypassing the notify watcher entirely. They continue to work unchanged. No new tests needed — the debouncer is a well-tested upstream crate.

## Verification

1. `cd src/creep && cargo test` — existing tests pass
2. `buck2 build root//src/creep:creep` — builds clean
3. Run creep, register a workspace, perform `git checkout` or bulk file operation — confirm no event-dropped spam in logs
