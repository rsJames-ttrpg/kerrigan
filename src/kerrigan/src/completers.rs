use std::ffi::OsStr;

use clap_complete::engine::{CompletionCandidate, ValueCompleter};
use nydus::NydusClient;

fn overseer_url() -> String {
    std::env::var("KERRIGAN_URL").unwrap_or_else(|_| "http://localhost:3100".into())
}

/// Run an async closure on a throwaway single-threaded tokio runtime.
/// Returns empty vec on any failure (runtime build, timeout, network).
fn blocking_complete<F, Fut>(f: F) -> Vec<CompletionCandidate>
where
    F: FnOnce(NydusClient) -> Fut,
    Fut: std::future::Future<Output = Vec<CompletionCandidate>>,
{
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return vec![];
    };
    let client = NydusClient::new(overseer_url());
    rt.block_on(f(client))
}

#[derive(Clone)]
pub struct RunIdCompleter;

impl ValueCompleter for RunIdCompleter {
    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        let prefix = current.to_string_lossy();
        blocking_complete(|client| async move {
            let Ok(runs) = client.list_runs(None).await else {
                return vec![];
            };
            let Ok(defs) = client.list_definitions().await else {
                return runs
                    .into_iter()
                    .filter(|r| r.id.starts_with(prefix.as_ref()))
                    .map(|r| CompletionCandidate::new(&r.id).help(Some(r.status.into())))
                    .collect();
            };
            let def_map: std::collections::HashMap<String, String> =
                defs.into_iter().map(|d| (d.id, d.name)).collect();

            runs.into_iter()
                .filter(|r| r.id.starts_with(prefix.as_ref()))
                .map(|r| {
                    let def_name = def_map
                        .get(&r.definition_id)
                        .map(|s| s.as_str())
                        .unwrap_or("?");
                    let task_excerpt = r
                        .config_overrides
                        .as_ref()
                        .and_then(|c| c.get("task"))
                        .and_then(|t| t.as_str())
                        .map(|t| {
                            let line = t.lines().next().unwrap_or(t);
                            if line.chars().count() > 40 {
                                let end = line
                                    .char_indices()
                                    .nth(37)
                                    .map(|(i, _)| i)
                                    .unwrap_or(line.len());
                                format!("{}...", &line[..end])
                            } else {
                                line.to_string()
                            }
                        })
                        .unwrap_or_default();
                    let help = format!("{} {} {}", r.status, def_name, task_excerpt);
                    CompletionCandidate::new(&r.id).help(Some(help.into()))
                })
                .collect()
        })
    }
}

#[derive(Clone)]
pub struct DefinitionCompleter;

impl ValueCompleter for DefinitionCompleter {
    fn complete(&self, current: &OsStr) -> Vec<CompletionCandidate> {
        let prefix = current.to_string_lossy();
        blocking_complete(|client| async move {
            let Ok(defs) = client.list_definitions().await else {
                return vec![];
            };
            defs.into_iter()
                .filter(|d| d.name.starts_with(prefix.as_ref()))
                .map(|d| CompletionCandidate::new(&d.name).help(Some(d.description.into())))
                .collect()
        })
    }
}
