# Webhook Notifier Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a generic webhook notifier to Queen that POSTs JSON to any HTTP endpoint on filtered events, initially targeting Signal via signal-cli-rest-api.

**Architecture:** New `WebhookNotifier` implements the existing `Notifier` trait. Config expands `[notifications]` with url, token, events filter, and an arbitrary body template with `{{placeholder}}` rendering. Wiring in `main.rs` matches on `backend` to construct the right notifier.

**Tech Stack:** Rust, reqwest (HTTP client), serde_json (template rendering), existing `Notifier` trait + `QueenEvent` enum.

**Spec:** `docs/specs/2026-04-01-webhook-notifier-design.md`

---

### Task 1: Expand NotificationConfig

**Files:**
- Modify: `src/queen/src/config.rs:120-133` (NotificationConfig struct + Default impl)
- Modify: `src/queen/src/config.rs:187-363` (test module — add new tests)

- [ ] **Step 1: Write failing test for webhook config parsing**

Add to the test module in `src/queen/src/config.rs`:

```rust
#[test]
fn test_parse_webhook_notifications() {
    let f = write_toml(
        r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
url = "http://localhost:8080/v2/send"
token = "my-secret-token"
events = ["drone_failed", "drone_stalled", "drone_timed_out"]

[notifications.body]
message = "{{message}}"
number = "+1234567890"
recipients = ["+0987654321"]
"#,
    );
    let config = Config::load(f.path()).unwrap();
    assert_eq!(config.notifications.backend, "webhook");
    assert_eq!(
        config.notifications.url.as_deref(),
        Some("http://localhost:8080/v2/send")
    );
    assert_eq!(
        config.notifications.token.as_deref(),
        Some("my-secret-token")
    );
    assert_eq!(
        config.notifications.events,
        Some(vec![
            "drone_failed".to_string(),
            "drone_stalled".to_string(),
            "drone_timed_out".to_string()
        ])
    );
    assert!(config.notifications.body.is_some());
    let body = config.notifications.body.unwrap();
    assert_eq!(body["message"], "{{message}}");
    assert_eq!(body["number"], "+1234567890");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/queen && cargo test test_parse_webhook_notifications -- --nocapture`
Expected: FAIL — `NotificationConfig` doesn't have `url`, `token`, `events`, `body` fields.

- [ ] **Step 3: Expand NotificationConfig with new fields**

In `src/queen/src/config.rs`, replace the `NotificationConfig` struct and its `Default` impl (lines 120-133):

```rust
#[derive(Debug, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_notification_backend")]
    pub backend: String,
    pub url: Option<String>,
    pub token: Option<String>,
    pub events: Option<Vec<String>>,
    pub body: Option<serde_json::Value>,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            backend: default_notification_backend(),
            url: None,
            token: None,
            events: None,
            body: None,
        }
    }
}
```

Remove the `#[allow(dead_code)]` from both `NotificationConfig` and the `notifications` field on `Config` — these fields are now used.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src/queen && cargo test test_parse_webhook_notifications -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write test for backward compatibility (log backend ignores extra fields)**

Add to test module:

```rust
#[test]
fn test_log_backend_ignores_webhook_fields() {
    let f = write_toml(
        r#"
[queen]
name = "test"

[notifications]
backend = "log"
"#,
    );
    let config = Config::load(f.path()).unwrap();
    assert_eq!(config.notifications.backend, "log");
    assert!(config.notifications.url.is_none());
    assert!(config.notifications.token.is_none());
    assert!(config.notifications.events.is_none());
    assert!(config.notifications.body.is_none());
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cd src/queen && cargo test test_log_backend_ignores_webhook_fields -- --nocapture`
Expected: PASS (defaults already handle this)

- [ ] **Step 7: Run all existing tests to verify nothing broke**

Run: `cd src/queen && cargo test`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/queen/src/config.rs
git commit -m "feat(queen): expand NotificationConfig with webhook fields"
```

---

### Task 2: Add reqwest dependency

**Files:**
- Modify: `src/queen/Cargo.toml` (add reqwest)
- Modify: `src/queen/BUCK` (add reqwest to deps)

- [ ] **Step 1: Add reqwest to Cargo.toml**

Add to `[dependencies]` in `src/queen/Cargo.toml`:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 2: Add reqwest to BUCK deps**

In `src/queen/BUCK`, add `"//third-party:reqwest"` to the `QUEEN_DEPS` list:

```python
QUEEN_DEPS = [
    "//third-party:anyhow",
    "//src/drone-sdk:drone-sdk",
    "//src/nydus:nydus",
    "//third-party:async-trait",
    "//third-party:chrono",
    "//third-party:clap",
    "//third-party:reqwest",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tokio",
    "//third-party:tokio-util",
    "//third-party:toml",
    "//third-party:tracing",
    "//third-party:tracing-subscriber",
    "//third-party:flate2",
]
```

- [ ] **Step 3: Run buckify to regenerate third-party BUCK**

Run: `./tools/buckify.sh`

This regenerates `third-party/BUCK` if reqwest isn't already present. It likely is (nydus uses it), but run it to be safe.

- [ ] **Step 4: Verify cargo check passes**

Run: `cd src/queen && cargo check`
Expected: Success.

- [ ] **Step 5: Commit**

```bash
git add src/queen/Cargo.toml src/queen/BUCK
git commit -m "feat(queen): add reqwest dependency for webhook notifier"
```

---

### Task 3: Implement WebhookNotifier

**Files:**
- Create: `src/queen/src/notifier/webhook.rs`
- Modify: `src/queen/src/notifier/mod.rs` (add `pub mod webhook;`)

- [ ] **Step 1: Write failing test for event type extraction**

Create `src/queen/src/notifier/webhook.rs` with the test:

```rust
use std::collections::HashSet;

use async_trait::async_trait;
use serde_json::Value;

use super::{Notifier, QueenEvent};

pub struct WebhookNotifier {
    client: reqwest::Client,
    url: String,
    token: Option<String>,
    events: HashSet<String>,
    body_template: String,
}

fn event_type(event: &QueenEvent) -> &'static str {
    match event {
        QueenEvent::HatcheryRegistered { .. } => "hatchery_registered",
        QueenEvent::DroneSpawned { .. } => "drone_spawned",
        QueenEvent::DroneCompleted { .. } => "drone_completed",
        QueenEvent::DroneFailed { .. } => "drone_failed",
        QueenEvent::DroneStalled { .. } => "drone_stalled",
        QueenEvent::DroneTimedOut { .. } => "drone_timed_out",
        QueenEvent::AuthRequested { .. } => "auth_requested",
        QueenEvent::CreepStarted => "creep_started",
        QueenEvent::CreepDied { .. } => "creep_died",
        QueenEvent::ShuttingDown => "shutting_down",
    }
}

fn build_placeholders(event: &QueenEvent) -> Vec<(&'static str, String)> {
    let et = event_type(event);
    match event {
        QueenEvent::DroneFailed { job_run_id, error } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", error.clone()),
            ("last_activity_secs", String::new()),
            ("message", format!("Drone failed for job {job_run_id}: {error}")),
        ],
        QueenEvent::DroneStalled {
            job_run_id,
            last_activity_secs,
        } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", String::new()),
            ("last_activity_secs", last_activity_secs.to_string()),
            (
                "message",
                format!(
                    "Drone stalled for job {job_run_id} (no activity for {last_activity_secs}s)"
                ),
            ),
        ],
        QueenEvent::DroneTimedOut { job_run_id } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", format!("Drone timed out for job {job_run_id}")),
        ],
        QueenEvent::DroneCompleted {
            job_run_id,
            exit_code,
        } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            (
                "message",
                format!("Drone completed for job {job_run_id} (exit code {exit_code})"),
            ),
        ],
        QueenEvent::DroneSpawned {
            job_run_id,
            drone_type,
        } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", format!("Drone spawned for job {job_run_id} (type: {drone_type})")),
        ],
        QueenEvent::HatcheryRegistered { name, id } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", String::new()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", format!("Hatchery registered: {name} ({id})")),
        ],
        QueenEvent::AuthRequested {
            job_run_id,
            url,
            message,
        } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", job_run_id.clone()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", format!("Auth requested for job {job_run_id}: {message} — {url}")),
        ],
        QueenEvent::CreepStarted => vec![
            ("event_type", et.to_string()),
            ("job_run_id", String::new()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", "Creep sidecar started".to_string()),
        ],
        QueenEvent::CreepDied { restart_in_secs } => vec![
            ("event_type", et.to_string()),
            ("job_run_id", String::new()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            (
                "message",
                format!("Creep sidecar died, restarting in {restart_in_secs}s"),
            ),
        ],
        QueenEvent::ShuttingDown => vec![
            ("event_type", et.to_string()),
            ("job_run_id", String::new()),
            ("error", String::new()),
            ("last_activity_secs", String::new()),
            ("message", "Queen shutting down".to_string()),
        ],
    }
}

fn render_template(template: &str, placeholders: &[(&str, String)]) -> String {
    let mut result = template.to_string();
    for (key, value) in placeholders {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_names() {
        assert_eq!(
            event_type(&QueenEvent::DroneFailed {
                job_run_id: "x".into(),
                error: "e".into()
            }),
            "drone_failed"
        );
        assert_eq!(
            event_type(&QueenEvent::DroneStalled {
                job_run_id: "x".into(),
                last_activity_secs: 100
            }),
            "drone_stalled"
        );
        assert_eq!(
            event_type(&QueenEvent::DroneTimedOut {
                job_run_id: "x".into()
            }),
            "drone_timed_out"
        );
    }

    #[test]
    fn test_render_template_replaces_placeholders() {
        let template = r#"{"text":"{{message}}","type":"{{event_type}}"}"#;
        let placeholders = vec![
            ("message", "Drone failed for job abc: oops".to_string()),
            ("event_type", "drone_failed".to_string()),
        ];
        let result = render_template(template, &placeholders);
        assert_eq!(
            result,
            r#"{"text":"Drone failed for job abc: oops","type":"drone_failed"}"#
        );
    }

    #[test]
    fn test_render_template_empty_placeholder() {
        let template = r#"{"error":"{{error}}"}"#;
        let placeholders = vec![("error", String::new())];
        let result = render_template(template, &placeholders);
        assert_eq!(result, r#"{"error":""}"#);
    }

    #[test]
    fn test_build_placeholders_failed() {
        let event = QueenEvent::DroneFailed {
            job_run_id: "run-1".into(),
            error: "compile error".into(),
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "drone_failed");
        assert_eq!(map["job_run_id"], "run-1");
        assert_eq!(map["error"], "compile error");
        assert_eq!(
            map["message"],
            "Drone failed for job run-1: compile error"
        );
    }

    #[test]
    fn test_build_placeholders_stalled() {
        let event = QueenEvent::DroneStalled {
            job_run_id: "run-2".into(),
            last_activity_secs: 450,
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "drone_stalled");
        assert_eq!(map["last_activity_secs"], "450");
        assert_eq!(
            map["message"],
            "Drone stalled for job run-2 (no activity for 450s)"
        );
    }

    #[test]
    fn test_build_placeholders_timed_out() {
        let event = QueenEvent::DroneTimedOut {
            job_run_id: "run-3".into(),
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "drone_timed_out");
        assert_eq!(map["message"], "Drone timed out for job run-3");
    }
}
```

Also add `pub mod webhook;` to `src/queen/src/notifier/mod.rs`.

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd src/queen && cargo test notifier::webhook -- --nocapture`
Expected: All 5 tests pass (these test pure functions, no HTTP needed).

- [ ] **Step 3: Implement from_config constructor**

Add to `src/queen/src/notifier/webhook.rs`, after `render_template` and before `#[cfg(test)]`:

```rust
use crate::config::NotificationConfig;

const VALID_EVENTS: &[&str] = &[
    "hatchery_registered",
    "drone_spawned",
    "drone_completed",
    "drone_failed",
    "drone_stalled",
    "drone_timed_out",
    "auth_requested",
    "creep_started",
    "creep_died",
    "shutting_down",
];

impl WebhookNotifier {
    pub fn from_config(config: &NotificationConfig) -> anyhow::Result<Self> {
        let url = config
            .url
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow::anyhow!("notifications.url is required for webhook backend"))?
            .to_string();

        let token = match config.token.as_deref() {
            Some(t) if t.starts_with("env:") => {
                let var = &t[4..];
                let val = std::env::var(var).map_err(|_| {
                    anyhow::anyhow!("notifications.token references env var '{var}' which is not set")
                })?;
                Some(val)
            }
            Some(t) => Some(t.to_string()),
            None => None,
        };

        let events: HashSet<String> = match &config.events {
            Some(ev) => {
                for e in ev {
                    if !VALID_EVENTS.contains(&e.as_str()) {
                        anyhow::bail!(
                            "unknown event type '{e}' in notifications.events (valid: {})",
                            VALID_EVENTS.join(", ")
                        );
                    }
                }
                ev.iter().cloned().collect()
            }
            None => VALID_EVENTS.iter().map(|s| (*s).to_string()).collect(),
        };

        let body_template = match &config.body {
            Some(body) => serde_json::to_string(body)?,
            None => "{}".to_string(),
        };

        Ok(Self {
            client: reqwest::Client::new(),
            url,
            token,
            events,
            body_template,
        })
    }
}
```

- [ ] **Step 4: Write tests for from_config**

Add to the `tests` module:

```rust
use crate::config::NotificationConfig;

#[test]
fn test_from_config_valid() {
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: Some("http://localhost:8080/v2/send".into()),
        token: Some("plain-token".into()),
        events: Some(vec!["drone_failed".into(), "drone_stalled".into()]),
        body: Some(serde_json::json!({"message": "{{message}}"})),
    };
    let notifier = WebhookNotifier::from_config(&config).unwrap();
    assert_eq!(notifier.url, "http://localhost:8080/v2/send");
    assert_eq!(notifier.token.as_deref(), Some("plain-token"));
    assert_eq!(notifier.events.len(), 2);
    assert!(notifier.events.contains("drone_failed"));
    assert!(notifier.events.contains("drone_stalled"));
}

#[test]
fn test_from_config_missing_url() {
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: None,
        token: None,
        events: None,
        body: None,
    };
    let err = WebhookNotifier::from_config(&config).unwrap_err();
    assert!(err.to_string().contains("url is required"));
}

#[test]
fn test_from_config_invalid_event() {
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: Some("http://localhost".into()),
        token: None,
        events: Some(vec!["bogus_event".into()]),
        body: None,
    };
    let err = WebhookNotifier::from_config(&config).unwrap_err();
    assert!(err.to_string().contains("unknown event type"));
}

#[test]
fn test_from_config_env_token() {
    std::env::set_var("TEST_WEBHOOK_TOKEN", "secret-from-env");
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: Some("http://localhost".into()),
        token: Some("env:TEST_WEBHOOK_TOKEN".into()),
        events: None,
        body: None,
    };
    let notifier = WebhookNotifier::from_config(&config).unwrap();
    assert_eq!(notifier.token.as_deref(), Some("secret-from-env"));
    std::env::remove_var("TEST_WEBHOOK_TOKEN");
}

#[test]
fn test_from_config_env_token_missing() {
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: Some("http://localhost".into()),
        token: Some("env:NONEXISTENT_VAR_12345".into()),
        events: None,
        body: None,
    };
    let err = WebhookNotifier::from_config(&config).unwrap_err();
    assert!(err.to_string().contains("NONEXISTENT_VAR_12345"));
}

#[test]
fn test_from_config_no_events_defaults_to_all() {
    let config = NotificationConfig {
        backend: "webhook".into(),
        url: Some("http://localhost".into()),
        token: None,
        events: None,
        body: None,
    };
    let notifier = WebhookNotifier::from_config(&config).unwrap();
    assert_eq!(notifier.events.len(), VALID_EVENTS.len());
}
```

- [ ] **Step 5: Run all webhook tests**

Run: `cd src/queen && cargo test notifier::webhook -- --nocapture`
Expected: All 11 tests pass.

- [ ] **Step 6: Implement the Notifier trait**

Add to `src/queen/src/notifier/webhook.rs`, after the `from_config` impl block:

```rust
#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, event: QueenEvent) {
        let et = event_type(&event);
        if !self.events.contains(et) {
            return;
        }

        let placeholders = build_placeholders(&event);
        let body = render_template(&self.body_template, &placeholders);

        let mut req = self.client.post(&self.url).header("content-type", "application/json").body(body);

        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        match req.timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(resp) if !resp.status().is_success() => {
                tracing::warn!(
                    status = %resp.status(),
                    event_type = et,
                    "webhook notification failed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    event_type = et,
                    "webhook notification request failed"
                );
            }
            Ok(_) => {
                tracing::debug!(event_type = et, "webhook notification sent");
            }
        }
    }
}
```

- [ ] **Step 7: Run all queen tests**

Run: `cd src/queen && cargo test`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/queen/src/notifier/webhook.rs src/queen/src/notifier/mod.rs
git commit -m "feat(queen): add WebhookNotifier with template rendering and event filtering"
```

---

### Task 4: Wire up notifier selection in main.rs

**Files:**
- Modify: `src/queen/src/main.rs:12,34` (import + construction)

- [ ] **Step 1: Update imports in main.rs**

In `src/queen/src/main.rs`, add the webhook import alongside the existing LogNotifier import. Replace line 12:

```rust
use notifier::log::LogNotifier;
```

with:

```rust
use notifier::log::LogNotifier;
use notifier::webhook::WebhookNotifier;
```

- [ ] **Step 2: Replace hardcoded LogNotifier with config-driven match**

In `src/queen/src/main.rs`, replace line 34:

```rust
let notifier: Arc<dyn notifier::Notifier> = Arc::new(LogNotifier);
```

with:

```rust
let notifier: Arc<dyn notifier::Notifier> = match config.notifications.backend.as_str() {
    "webhook" => Arc::new(WebhookNotifier::from_config(&config.notifications)?),
    _ => Arc::new(LogNotifier),
};
```

- [ ] **Step 3: Verify cargo check passes**

Run: `cd src/queen && cargo check`
Expected: Success, no warnings.

- [ ] **Step 4: Run all queen tests**

Run: `cd src/queen && cargo test`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/queen/src/main.rs
git commit -m "feat(queen): wire webhook notifier selection from config"
```

---

### Task 5: Config validation

**Files:**
- Modify: `src/queen/src/config.rs:155-169` (validate method)
- Modify: `src/queen/src/config.rs` (test module)

- [ ] **Step 1: Write failing test for webhook validation**

Add to the test module in `src/queen/src/config.rs`:

```rust
#[test]
fn test_validate_webhook_missing_url() {
    let f = write_toml(
        r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
"#,
    );
    let err = Config::load(f.path()).unwrap_err();
    assert!(err.to_string().contains("notifications.url is required"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src/queen && cargo test test_validate_webhook_missing_url -- --nocapture`
Expected: FAIL — validation doesn't check webhook config yet.

- [ ] **Step 3: Add webhook validation to Config::validate**

In `src/queen/src/config.rs`, add to the end of `validate()` before `Ok(())`:

```rust
if self.notifications.backend == "webhook" {
    if self.notifications.url.as_ref().is_none_or(|u| u.is_empty()) {
        anyhow::bail!("notifications.url is required for webhook backend");
    }
    if let Some(events) = &self.notifications.events {
        let valid = [
            "hatchery_registered",
            "drone_spawned",
            "drone_completed",
            "drone_failed",
            "drone_stalled",
            "drone_timed_out",
            "auth_requested",
            "creep_started",
            "creep_died",
            "shutting_down",
        ];
        for e in events {
            if !valid.contains(&e.as_str()) {
                anyhow::bail!("unknown notification event: '{e}'");
            }
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src/queen && cargo test test_validate_webhook_missing_url -- --nocapture`
Expected: PASS

- [ ] **Step 5: Write test for invalid event name in config**

Add to test module:

```rust
#[test]
fn test_validate_webhook_invalid_event() {
    let f = write_toml(
        r#"
[queen]
name = "test"

[notifications]
backend = "webhook"
url = "http://localhost:8080"
events = ["drone_failed", "not_a_real_event"]
"#,
    );
    let err = Config::load(f.path()).unwrap_err();
    assert!(err.to_string().contains("unknown notification event"));
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cd src/queen && cargo test test_validate_webhook_invalid_event -- --nocapture`
Expected: PASS

- [ ] **Step 7: Run all queen tests**

Run: `cd src/queen && cargo test`
Expected: All tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/queen/src/config.rs
git commit -m "feat(queen): validate webhook notification config at startup"
```

---

### Task 6: Update hatchery.toml example and verify end-to-end

**Files:**
- Modify: `hatchery.toml` (add commented webhook example)

- [ ] **Step 1: Add commented webhook config to hatchery.toml**

Append to `hatchery.toml`:

```toml

# Uncomment to enable webhook notifications (e.g. Signal via signal-cli-rest-api):
# [notifications]
# backend = "webhook"
# url = "http://localhost:8080/v2/send"
# token = "env:SIGNAL_API_TOKEN"
# events = ["drone_failed", "drone_stalled", "drone_timed_out"]
#
# [notifications.body]
# message = "{{message}}"
# number = "+1234567890"
# recipients = ["+0987654321"]
```

- [ ] **Step 2: Verify cargo check for queen**

Run: `cd src/queen && cargo check`
Expected: Success.

- [ ] **Step 3: Run all queen tests**

Run: `cd src/queen && cargo test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add hatchery.toml
git commit -m "docs: add webhook notification example to hatchery.toml"
```
