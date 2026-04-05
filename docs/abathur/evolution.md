---
title: Evolution Chamber
slug: evolution
description: Heuristic analysis of drone sessions — cost, tool patterns, context pressure, failure rates
lastmod: 2026-04-05
tags: [evolution, analysis, metrics, recommendations]
sources:
  - path: src/evolution/src/lib.rs
    hash: fb5a1ffc803d59f08617266278d4cd221c7e7f1d5879d38427128eddc0d292d0
  - path: src/evolution/src/fetch.rs
    hash: 2f4c635dfee5f287dd95c7da74975f78cc759f879ad63c5cf893be8d1dfddb72
  - path: src/evolution/src/parse.rs
    hash: f83b654fead3e1ba8e876327ee53b609290c595e1706df89528bbc6ef408ab00
  - path: src/evolution/src/metrics.rs
    hash: c879d7c463aa5164b745b673cd304f8acc338cdc906375042e2af4788828a97e
  - path: src/evolution/src/rules.rs
    hash: 2a72fb8c17a635d5f61f821669733265d38fcaceed2b7ee3aeb2a651775d1563
  - path: src/evolution/src/report.rs
    hash: a4f38501f0abb711c953a914fb8308a892d8e3cbf79dcce0c5f1ad17499d1e62
sections: [pipeline, fetch, parse, metrics, rules, report]
---

# Evolution Chamber

## Pipeline

```
fetch artifacts → parse → aggregate metrics → apply rules → report
```

Entry point:

```rust
pub struct EvolutionChamber { client: NydusClient }

impl EvolutionChamber {
    pub async fn analyze(
        &self,
        scope: AnalysisScope,   // Global | Repo(url)
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> anyhow::Result<Option<AnalysisReport>>
}
```

Returns `None` if fewer than `min_sessions` conversations are available.

## Fetch

`fetch_artifacts(client, artifact_type, since) -> Vec<FetchedArtifact>`

Queries Overseer for artifacts by type ("conversation", "session") since a timestamp. Decompresses gzip via `flate2::GzDecoder`. Skips individual artifacts on failure (logs warnings).

## Parse

**ConversationSummary** — from conversation artifacts:
- `run_id`, `cost_usd`, `num_turns`, `duration_ms`, `success`
- `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_creation_tokens`
- Success determined from `subtype == "success"` in JSON

**SessionDetail** — from session JSONL:
- `run_id`, `tool_calls: Vec<ToolCall>`, `message_count`, `compression_events`
- `ToolCall { name, first_arg, is_error }`
- Tool calls extracted from assistant `tool_use` blocks, errors detected from user `tool_result` blocks
- Compression events detected from system messages containing "summary" or "context window"
- `extract_first_arg(input)` prioritizes `file_path`, `command`, `pattern`, `path`, `url` fields

## Metrics

**CostSummary:**
- `total_cost_usd`, `cost_by_stage: HashMap<String, f64>`
- `cost_trend: Trend` (Increasing/Stable/Decreasing) — compares first-half vs second-half avg (±20% threshold)
- `highest_cost_runs: Vec<RunCostEntry>` (top 10)

**ToolPatterns:**
- `call_counts: HashMap<String, u64>` — frequency per tool
- `retry_sequences: Vec<RetrySequence>` — 3+ consecutive identical tool+arg calls
- `error_rates: HashMap<String, f64>` — error ratio per tool
- `top_context_consumers: Vec<String>` — top 5 tools by call count

**ContextPressure:**
- `avg_turns`, `median_turns`, `compression_events` (total)
- `avg_cache_hit_ratio` — `cache_read / (cache_read + cache_creation)`

**FailureAnalysis:**
- `failure_rate` — failed / total
- `failure_by_stage: HashMap<String, f64>`

## Rules

```rust
pub struct Recommendation {
    pub category: RecommendationCategory,  // Tooling, Skills, SharedService, Context, Workflow
    pub severity: Severity,                // High, Medium, Low
    pub title: String,
    pub detail: String,
    pub evidence: String,
}
```

Rule triggers:
- Cost increasing trend → Medium
- Runs > $3.00 → High
- Tool error rate > 15% → High
- 3+ retry sequences → Medium
- Single tool > 40% of calls → Low (dominance)
- Compression events → High (>5) or Medium (any)
- Failure rate > 30% → High
- Stage failure rate > 50% → High

Sorted by severity (High first).

## Report

```rust
pub struct AnalysisReport {
    pub generated_at: DateTime<Utc>,
    pub scope: AnalysisScope,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub runs_analyzed: usize,
    pub cost_summary: CostSummary,
    pub tool_patterns: ToolPatterns,
    pub context_pressure: ContextPressure,
    pub failure_analysis: FailureAnalysis,
    pub recommendations: Vec<Recommendation>,
}
```

All types implement `Serialize`/`Deserialize` for JSON export. Reports stored as "evolution-report" artifacts in Overseer for restart recovery.
