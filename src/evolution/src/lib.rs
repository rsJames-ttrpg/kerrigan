pub mod report;

pub mod fetch;
pub mod metrics;
pub mod parse;
pub mod rules;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nydus::NydusClient;

use report::{AnalysisReport, AnalysisScope};

pub struct EvolutionChamber {
    client: NydusClient,
}

impl EvolutionChamber {
    pub fn new(client: NydusClient) -> Self {
        Self { client }
    }

    pub async fn analyze(
        &self,
        scope: AnalysisScope,
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> anyhow::Result<Option<AnalysisReport>> {
        // 1. Fetch artifacts
        let conversation_artifacts = fetch::fetch_artifacts(&self.client, "conversation", &since).await?;
        let session_artifacts = fetch::fetch_artifacts(&self.client, "session", &since).await?;

        if conversation_artifacts.len() < min_sessions {
            tracing::info!(
                found = conversation_artifacts.len(),
                min = min_sessions,
                "insufficient sessions for analysis"
            );
            return Ok(None);
        }

        // 2. Parse
        let mut conversations = Vec::new();
        for artifact in &conversation_artifacts {
            match parse::parse_conversation(&artifact.run_id, &artifact.data) {
                Ok(summary) => conversations.push(summary),
                Err(e) => tracing::warn!(run_id = %artifact.run_id, error = %e, "failed to parse conversation"),
            }
        }

        let mut sessions = Vec::new();
        for artifact in &session_artifacts {
            match parse::parse_session(&artifact.run_id, &artifact.data) {
                Ok(detail) => sessions.push(detail),
                Err(e) => tracing::warn!(run_id = %artifact.run_id, error = %e, "failed to parse session"),
            }
        }

        // 3. Build stage map from job runs (run_id -> stage)
        // For v1, we extract stage from the conversation data or default to "unknown"
        // TODO: fetch job run configs from Overseer for accurate stage mapping
        let stage_map: HashMap<String, String> = HashMap::new();

        // 4. Filter by scope
        // For Repo scope, we'd filter by repo_url from job run config
        // For v1, Global processes everything; Repo filtering requires job run API extension
        let scope_label = match &scope {
            AnalysisScope::Global => "global".to_string(),
            AnalysisScope::Repo(url) => url.clone(),
        };
        tracing::info!(
            scope = %scope_label,
            conversations = conversations.len(),
            sessions = sessions.len(),
            "starting analysis"
        );

        // 5. Compute metrics
        let cost_summary = metrics::build_cost_summary(&conversations, &stage_map)?;
        let tool_patterns = metrics::build_tool_patterns(&sessions)?;
        let context_pressure = metrics::build_context_pressure(&conversations, &sessions);
        let failure_analysis = metrics::build_failure_analysis(&conversations, &stage_map);

        // 6. Generate recommendations
        let recommendations = rules::generate_recommendations(
            &cost_summary,
            &tool_patterns,
            &context_pressure,
            &failure_analysis,
        );

        let now = Utc::now();
        Ok(Some(AnalysisReport {
            generated_at: now,
            scope,
            period_start: since,
            period_end: now,
            runs_analyzed: conversations.len(),
            cost_summary,
            tool_patterns,
            context_pressure,
            failure_analysis,
            recommendations,
        }))
    }
}
