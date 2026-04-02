use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use evolution::EvolutionChamber;
use evolution::report::AnalysisScope;
use nydus::NydusClient;
use tokio_util::sync::CancellationToken;

use crate::config::EvolutionConfig;

pub async fn run(client: NydusClient, config: EvolutionConfig, token: CancellationToken) {
    if !config.enabled {
        tracing::info!("evolution chamber disabled");
        return;
    }

    let chamber = EvolutionChamber::new(client.clone());
    let time_interval =
        crate::parse_duration(&config.time_interval).unwrap_or(Duration::from_secs(86400));

    let mut last_analysis = Instant::now();
    let mut completed_since_last = 0usize;
    let mut last_analysis_time: DateTime<Utc> = Utc::now();
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
                tracing::debug!(error = %e, "failed to poll completed runs for evolution");
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
                // Submit report as job via start_run with task in config_overrides
                let report_json = match serde_json::to_string_pretty(&report) {
                    Ok(json) => json,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to serialize evolution report");
                        continue;
                    }
                };
                let task = format!(
                    "Analyze the following Evolution Chamber report and create GitHub issues for each high/medium severity recommendation.\n\nLabel issues with `evolution-chamber`.\n\n```json\n{}\n```",
                    report_json
                );
                let config_overrides = serde_json::json!({ "task": task, "stage": "evolve" });
                if let Err(e) = client
                    .start_run(
                        &config.drone_definition,
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
