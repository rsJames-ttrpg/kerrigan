use std::ffi::OsStr;
use std::time::Duration;

use clap_complete::engine::{CompletionCandidate, ValueCompleter};

fn overseer_url() -> String {
    std::env::var("KERRIGAN_URL").unwrap_or_else(|_| "http://localhost:3100".into())
}

/// Blocking HTTP GET with short timeout. Returns None on any failure.
fn blocking_get(path: &str) -> Option<String> {
    let url = format!("{}{}", overseer_url(), path);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().ok()
}

#[derive(Clone)]
pub struct RunIdCompleter;

impl ValueCompleter for RunIdCompleter {
    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        let prefix = current.to_string_lossy();

        let Some(runs_json) = blocking_get("/api/jobs/runs") else {
            return vec![];
        };
        let Ok(runs) = serde_json::from_str::<Vec<serde_json::Value>>(&runs_json) else {
            return vec![];
        };

        // Optionally fetch definitions for names
        let def_map: std::collections::HashMap<String, String> =
            blocking_get("/api/jobs/definitions")
                .and_then(|j| serde_json::from_str::<Vec<serde_json::Value>>(&j).ok())
                .map(|defs| {
                    defs.into_iter()
                        .filter_map(|d| {
                            let id = d.get("id")?.as_str()?.to_string();
                            let name = d.get("name")?.as_str()?.to_string();
                            Some((id, name))
                        })
                        .collect()
                })
                .unwrap_or_default();

        runs.into_iter()
            .filter_map(|r| {
                let id = r.get("id")?.as_str()?;
                if !id.starts_with(prefix.as_ref()) {
                    return None;
                }
                let status = r.get("status").and_then(|s| s.as_str()).unwrap_or("?");
                let def_id = r
                    .get("definition_id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let def_name = def_map.get(def_id).map(|s| s.as_str()).unwrap_or("?");
                let task = r
                    .pointer("/config_overrides/task")
                    .and_then(|t| t.as_str())
                    .map(|t| {
                        let line = t.lines().next().unwrap_or(t);
                        if line.chars().count() > 35 {
                            let end = line
                                .char_indices()
                                .nth(32)
                                .map(|(i, _)| i)
                                .unwrap_or(line.len());
                            format!("{}...", &line[..end])
                        } else {
                            line.to_string()
                        }
                    })
                    .unwrap_or_default();
                let help = format!("{status} ({def_name}) {task}");
                Some(CompletionCandidate::new(id).help(Some(help.into())))
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct ArtifactIdCompleter;

impl ValueCompleter for ArtifactIdCompleter {
    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        let prefix = current.to_string_lossy();

        let Some(json) = blocking_get("/api/artifacts") else {
            return vec![];
        };
        let Ok(artifacts) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else {
            return vec![];
        };

        artifacts
            .into_iter()
            .filter_map(|a| {
                let id = a.get("id")?.as_str()?;
                if !id.starts_with(prefix.as_ref()) {
                    return None;
                }
                let name = a.get("name").and_then(|s| s.as_str()).unwrap_or("?");
                let atype = a
                    .get("artifact_type")
                    .and_then(|s| s.as_str())
                    .unwrap_or("?");
                let help = format!("{name} ({atype})");
                Some(CompletionCandidate::new(id).help(Some(help.into())))
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct DefinitionCompleter;

impl ValueCompleter for DefinitionCompleter {
    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        let prefix = current.to_string_lossy();

        let Some(json) = blocking_get("/api/jobs/definitions") else {
            return vec![];
        };
        let Ok(defs) = serde_json::from_str::<Vec<serde_json::Value>>(&json) else {
            return vec![];
        };

        defs.into_iter()
            .filter_map(|d| {
                let name = d.get("name")?.as_str()?.to_string();
                if !name.starts_with(prefix.as_ref()) {
                    return None;
                }
                let desc = d
                    .get("description")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(CompletionCandidate::new(name).help(Some(desc.into())))
            })
            .collect()
    }
}
