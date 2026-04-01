# Webhook Notifier for Queen

## Problem

Queen has a pluggable `Notifier` trait with events wired throughout its actor system, but the only implementation is `LogNotifier` (tracing logs). There's no way to get alerted when drones fail, stall, or time out without watching logs.

## Goal

Add a generic `WebhookNotifier` that POSTs JSON to any HTTP endpoint when filtered events occur. The first target is Signal via [signal-cli-rest-api](https://github.com/bbernhard/signal-cli-rest-api) behind [secured-signal-api](https://github.com/codeshelldev/secured-signal-api), but the design supports any webhook-compatible service (Slack, Discord, etc.) through config alone.

## Design

### WebhookNotifier

A new `notifier/webhook.rs` implementing the existing `Notifier` trait.

**Struct fields:**
- `client: reqwest::Client` — reused across notifications
- `url: String` — POST target
- `token: Option<String>` — resolved bearer token value
- `events: HashSet<String>` — event type filter (snake_case names)
- `body_template: String` — JSON string with `{{placeholder}}` markers

**`notify()` behavior:**
1. Derive `event_type` (snake_case) from the `QueenEvent` variant
2. If `event_type` is not in `self.events`, return early
3. Build placeholder map from the event data
4. Render `body_template` by replacing `{{key}}` with values
5. POST rendered JSON to `self.url` with optional `Authorization: Bearer` header
6. Log errors via tracing, never panic or block the supervisor

**Constructor:** `WebhookNotifier::from_config(config: &NotificationConfig) -> anyhow::Result<Self>`
- Resolves `env:VAR_NAME` token syntax at startup (fail fast if env var missing)
- Validates `url` is present
- Validates `events` against known event names
- Serializes `body` table to a JSON string template

### Template Placeholders

| Placeholder | Source | Example |
|---|---|---|
| `{{event_type}}` | Enum variant, snake_case | `drone_failed` |
| `{{job_run_id}}` | From event data (`""` if N/A) | `abc-123` |
| `{{message}}` | Human-readable summary | `Drone failed for job abc-123: compile error` |
| `{{error}}` | Error string (failed only, else `""`) | `compile error` |
| `{{last_activity_secs}}` | Stalled only, else `""` | `450` |

Rendering is simple string replacement on the serialized JSON body template. Placeholders not relevant to the current event resolve to empty string.

**`{{message}}` formats:**
- Failed: `"Drone failed for job {id}: {error}"`
- Stalled: `"Drone stalled for job {id} (no activity for {secs}s)"`
- Timed out: `"Drone timed out for job {id}"`

### Configuration

Expand `NotificationConfig` in `config.rs` with optional fields:

```toml
[notifications]
backend = "webhook"
url = "http://localhost:8080/v2/send"
token = "env:SIGNAL_API_TOKEN"          # optional; "env:" prefix resolves from env var
events = ["failed", "stalled", "timed_out"]

[notifications.body]
message = "{{message}}"
number = "+1234567890"
recipients = ["+0987654321"]
```

- `backend = "webhook"` selects `WebhookNotifier`; `"log"` (default) keeps `LogNotifier`
- `body` is an arbitrary TOML table serialized to JSON at startup; `{{placeholders}}` rendered per-event
- `events` filters which `QueenEvent` variants trigger a POST; snake_case names
- `token` with `env:` prefix reads value from environment variable; plain strings used as-is
- When `backend = "log"`, extra fields are ignored — backward compatible

**Valid event names:** `hatchery_registered`, `drone_spawned`, `drone_completed`, `drone_failed`, `drone_stalled`, `drone_timed_out`, `auth_requested`, `creep_started`, `creep_died`, `shutting_down`

**Validation at startup:**
- `backend = "webhook"` requires `url` to be non-empty
- `env:VAR_NAME` token resolved immediately — fails fast if missing
- `events` validated against known event names — unknown names are an error

### Wiring

In `main.rs`, replace the hardcoded `LogNotifier` with a match:

```rust
let notifier: Arc<dyn Notifier> = match config.notifications.backend.as_str() {
    "webhook" => Arc::new(WebhookNotifier::from_config(&config.notifications)?),
    _ => Arc::new(LogNotifier),
};
```

### Changes Summary

| File | Change |
|---|---|
| `src/queen/src/notifier/webhook.rs` | New — `WebhookNotifier` struct and impl |
| `src/queen/src/notifier/mod.rs` | Add `pub mod webhook;` |
| `src/queen/src/config.rs` | Expand `NotificationConfig` with optional url/token/events/body fields |
| `src/queen/src/main.rs` | Match on `backend` to construct the right notifier |
| `src/queen/Cargo.toml` | Add `reqwest` as a direct dependency |

No changes to `Notifier` trait, `QueenEvent` enum, `LogNotifier`, supervisor, or any actor code.

### Example: Signal Configuration

```toml
[notifications]
backend = "webhook"
url = "http://localhost:8080/v2/send"
token = "env:SIGNAL_API_TOKEN"
events = ["failed", "stalled", "timed_out"]

[notifications.body]
message = "{{message}}"
number = "+1234567890"
recipients = ["+0987654321"]
```

### Example: Slack Configuration (future, no code changes needed)

```toml
[notifications]
backend = "webhook"
url = "https://hooks.slack.com/services/T.../B.../xxx"
events = ["failed", "timed_out"]

[notifications.body]
text = "{{message}}"
```
