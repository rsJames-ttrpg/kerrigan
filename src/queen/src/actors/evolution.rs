use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use evolution::EvolutionChamber;
use evolution::report::AnalysisScope;
use nydus::NydusClient;
use tokio_util::sync::CancellationToken;

use crate::config::EvolutionConfig;

const ARTIFACT_TYPE: &str = "evolution-report";

pub async fn run(client: NydusClient, config: EvolutionConfig, token: CancellationToken) {
    if !config.enabled {
        tracing::info!("evolution chamber disabled");
        return;
    }

    let chamber = EvolutionChamber::new(client.clone());
    let time_interval = match crate::parse_duration(&config.time_interval) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!(
                interval = %config.time_interval,
                error = %e,
                "invalid evolution time_interval, falling back to 24h"
            );
            Duration::from_secs(86400)
        }
    };

    // Resolve the job definition name to a UUID once at startup
    let definition_id = match resolve_definition_id(&client, &config.drone_definition).await {
        Some(id) => id,
        None => {
            tracing::error!(
                name = %config.drone_definition,
                "evolution job definition not found, actor cannot submit jobs"
            );
            return;
        }
    };

    // Recover last analysis time from the most recent evolution-report artifact
    let mut last_analysis_time: DateTime<Utc> = recover_last_analysis_time(&client).await;
    let mut last_analysis = Instant::now();
    let mut completed_since_last = 0usize;
    let mut previous_completed_total = 0usize;

    let poll_interval = Duration::from_secs(60);

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("evolution actor shutting down");
                return;
            }
            _ = tokio::time::sleep(poll_interval) => {}
        }

        // Count completed runs since last analysis by tracking delta
        let runs = match client.list_runs(Some("completed")).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "failed to poll completed runs for evolution");
                continue;
            }
        };
        let current_total = runs.len();
        if current_total > previous_completed_total {
            completed_since_last += current_total - previous_completed_total;
        }
        previous_completed_total = current_total;

        let time_triggered = last_analysis.elapsed() >= time_interval;
        let count_triggered = completed_since_last >= config.run_interval;

        if !time_triggered && !count_triggered {
            continue;
        }

        tracing::info!(
            time_triggered,
            count_triggered,
            completed = completed_since_last,
            "evolution chamber triggered"
        );

        // Run global analysis
        match chamber
            .analyze(
                AnalysisScope::Global,
                last_analysis_time,
                config.min_sessions,
            )
            .await
        {
            Ok(Some(report)) => {
                tracing::info!(
                    runs = report.runs_analyzed,
                    recommendations = report.recommendations.len(),
                    "evolution analysis complete"
                );
                let report_json = match serde_json::to_string_pretty(&report) {
                    Ok(json) => json,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to serialize evolution report");
                        continue;
                    }
                };

                // Store the report as an artifact for audit trail and restart recovery
                let artifact_name =
                    format!("evolution-report-{}", Utc::now().format("%Y%m%d-%H%M%S"));
                if let Err(e) = client
                    .store_artifact(
                        &artifact_name,
                        "application/json",
                        report_json.as_bytes(),
                        None,
                        Some(ARTIFACT_TYPE),
                    )
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        "failed to store evolution report artifact — restart recovery \
                         will not detect this analysis, which may cause a duplicate run"
                    );
                }

                let task = format!(
                    "Analyze the following Evolution Chamber report and create GitHub issues \
                     for each high/medium severity recommendation.\n\n\
                     Label issues with `evolution-chamber`.\n\n```json\n{}\n```",
                    report_json
                );
                let config_overrides = serde_json::json!({ "task": task, "stage": "evolve" });
                if let Err(e) = client
                    .start_run(
                        &definition_id,
                        "evolution-chamber",
                        None,
                        Some(config_overrides),
                    )
                    .await
                {
                    tracing::warn!(error = %e, "failed to submit evolution report job");
                }
            }
            Ok(None) => {
                tracing::info!("evolution analysis skipped: insufficient data");
            }
            Err(e) => {
                tracing::warn!(error = %e, "evolution analysis failed");
            }
        }

        last_analysis = Instant::now();
        last_analysis_time = Utc::now();
        completed_since_last = 0;
    }
}

/// Resolve a job definition name to its UUID.
async fn resolve_definition_id(client: &NydusClient, name: &str) -> Option<String> {
    let definitions = client.list_definitions().await.ok()?;
    definitions
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.id.clone())
}

/// Fetch the most recent evolution-report artifact to recover the last analysis timestamp.
/// Falls back to `Utc::now()` if no previous report exists.
async fn recover_last_analysis_time(client: &NydusClient) -> DateTime<Utc> {
    match client.list_artifacts(None, Some(ARTIFACT_TYPE), None).await {
        Ok(artifacts) => {
            if let Some(latest) = artifacts
                .iter()
                .filter_map(|a| a.created_at.map(|ts| (a, ts)))
                .max_by_key(|(_, ts)| *ts)
            {
                tracing::info!(
                    artifact = %latest.0.name,
                    last_analysis = %latest.1,
                    "recovered last evolution analysis time"
                );
                latest.1
            } else {
                tracing::info!("no previous evolution reports found, starting fresh");
                Utc::now()
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to fetch evolution report history, starting fresh");
            Utc::now()
        }
    }
}
