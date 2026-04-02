use std::collections::HashSet;

use async_trait::async_trait;

use super::{Notifier, QueenEvent};

#[derive(Debug)]
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
            (
                "message",
                format!("Drone failed for job {job_run_id}: {error}"),
            ),
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
            (
                "message",
                format!("Drone spawned for job {job_run_id} (type: {drone_type})"),
            ),
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
            (
                "message",
                format!("Auth requested for job {job_run_id}: {message} — {url}"),
            ),
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
    // Parse the template as JSON, walk string values and substitute placeholders,
    // then re-serialize. This ensures placeholder values are properly JSON-escaped.
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(template) else {
        // If the template isn't valid JSON, fall back to string replacement
        let mut result = template.to_string();
        for (key, val) in placeholders {
            result = result.replace(&format!("{{{{{key}}}}}"), val);
        }
        return result;
    };
    substitute_placeholders(&mut value, placeholders);
    serde_json::to_string(&value).unwrap_or_else(|_| template.to_string())
}

fn substitute_placeholders(value: &mut serde_json::Value, placeholders: &[(&str, String)]) {
    match value {
        serde_json::Value::String(s) => {
            for (key, val) in placeholders {
                *s = s.replace(&format!("{{{{{key}}}}}"), val);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                substitute_placeholders(item, placeholders);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                substitute_placeholders(v, placeholders);
            }
        }
        _ => {}
    }
}

use crate::config::NotificationConfig;

pub const VALID_EVENTS: &[&str] = &[
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
        Self::from_config_with_env(config, |k| std::env::var(k))
    }

    pub fn from_config_with_env(
        config: &NotificationConfig,
        env_lookup: impl Fn(&str) -> Result<String, std::env::VarError>,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(config.tls_skip_verify)
            .build()?;
        let url = config
            .url
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| anyhow::anyhow!("notifications.url is required for webhook backend"))?
            .to_string();

        let token = match config.token.as_deref() {
            Some(t) if t.starts_with("env:") => {
                let var = &t[4..];
                let val = env_lookup(var).map_err(|_| {
                    anyhow::anyhow!(
                        "notifications.token references env var '{var}' which is not set"
                    )
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
            client,
            url,
            token,
            events,
            body_template,
        })
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    async fn notify(&self, event: QueenEvent) {
        let et = event_type(&event);
        if !self.events.contains(et) {
            return;
        }

        let placeholders = build_placeholders(&event);
        let body = render_template(&self.body_template, &placeholders);

        let mut req = self
            .client
            .post(&self.url)
            .header("content-type", "application/json")
            .body(body);

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
    fn test_render_template_json_escapes_special_chars() {
        let template = r#"{"text":"{{message}}"}"#;
        let placeholders = vec![(
            "message",
            r#"Drone failed for job abc: expected "}""#.to_string(),
        )];
        let result = render_template(template, &placeholders);
        // The result should be valid JSON with the quotes escaped
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["text"], r#"Drone failed for job abc: expected "}""#);
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
        assert_eq!(map["message"], "Drone failed for job run-1: compile error");
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

    #[test]
    fn test_build_placeholders_completed() {
        let event = QueenEvent::DroneCompleted {
            job_run_id: "run-4".into(),
            exit_code: 0,
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "drone_completed");
        assert_eq!(map["job_run_id"], "run-4");
        assert_eq!(
            map["message"],
            "Drone completed for job run-4 (exit code 0)"
        );
    }

    #[test]
    fn test_build_placeholders_spawned() {
        let event = QueenEvent::DroneSpawned {
            job_run_id: "run-5".into(),
            drone_type: "claude".into(),
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "drone_spawned");
        assert_eq!(map["job_run_id"], "run-5");
        assert_eq!(map["message"], "Drone spawned for job run-5 (type: claude)");
    }

    #[test]
    fn test_build_placeholders_hatchery_registered() {
        let event = QueenEvent::HatcheryRegistered {
            name: "prod-hatchery".into(),
            id: "h-123".into(),
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "hatchery_registered");
        assert_eq!(map["job_run_id"], "");
        assert_eq!(map["message"], "Hatchery registered: prod-hatchery (h-123)");
    }

    #[test]
    fn test_build_placeholders_auth_requested() {
        let event = QueenEvent::AuthRequested {
            job_run_id: "run-6".into(),
            url: "https://auth.example.com".into(),
            message: "Please authenticate".into(),
        };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "auth_requested");
        assert_eq!(map["job_run_id"], "run-6");
        assert_eq!(
            map["message"],
            "Auth requested for job run-6: Please authenticate — https://auth.example.com"
        );
    }

    #[test]
    fn test_build_placeholders_creep_started() {
        let p = build_placeholders(&QueenEvent::CreepStarted);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "creep_started");
        assert_eq!(map["message"], "Creep sidecar started");
    }

    #[test]
    fn test_build_placeholders_creep_died() {
        let event = QueenEvent::CreepDied { restart_in_secs: 5 };
        let p = build_placeholders(&event);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "creep_died");
        assert_eq!(map["message"], "Creep sidecar died, restarting in 5s");
    }

    #[test]
    fn test_build_placeholders_shutting_down() {
        let p = build_placeholders(&QueenEvent::ShuttingDown);
        let map: std::collections::HashMap<&str, &str> =
            p.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(map["event_type"], "shutting_down");
        assert_eq!(map["message"], "Queen shutting down");
    }

    #[test]
    fn test_from_config_valid() {
        let config = NotificationConfig {
            backend: "webhook".into(),
            url: Some("http://localhost:8080/v2/send".into()),
            token: Some("plain-token".into()),
            events: Some(vec!["drone_failed".into(), "drone_stalled".into()]),
            body: Some(serde_json::json!({"message": "{{message}}"})),
            tls_skip_verify: false,
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
            tls_skip_verify: false,
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
            tls_skip_verify: false,
        };
        let err = WebhookNotifier::from_config(&config).unwrap_err();
        assert!(err.to_string().contains("unknown event type"));
    }

    #[test]
    fn test_from_config_env_token() {
        let config = NotificationConfig {
            backend: "webhook".into(),
            url: Some("http://localhost".into()),
            token: Some("env:TEST_WEBHOOK_TOKEN".into()),
            events: None,
            body: None,
            tls_skip_verify: false,
        };
        let env = |key: &str| -> Result<String, std::env::VarError> {
            if key == "TEST_WEBHOOK_TOKEN" {
                Ok("secret-from-env".into())
            } else {
                Err(std::env::VarError::NotPresent)
            }
        };
        let notifier = WebhookNotifier::from_config_with_env(&config, env).unwrap();
        assert_eq!(notifier.token.as_deref(), Some("secret-from-env"));
    }

    #[test]
    fn test_from_config_env_token_missing() {
        let config = NotificationConfig {
            backend: "webhook".into(),
            url: Some("http://localhost".into()),
            token: Some("env:NONEXISTENT_VAR_12345".into()),
            events: None,
            body: None,
            tls_skip_verify: false,
        };
        let env =
            |_: &str| -> Result<String, std::env::VarError> { Err(std::env::VarError::NotPresent) };
        let err = WebhookNotifier::from_config_with_env(&config, env).unwrap_err();
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
            tls_skip_verify: false,
        };
        let notifier = WebhookNotifier::from_config(&config).unwrap();
        assert_eq!(notifier.events.len(), VALID_EVENTS.len());
    }
}
