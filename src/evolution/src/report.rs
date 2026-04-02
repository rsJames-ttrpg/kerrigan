use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalysisScope {
    Global,
    Repo(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trend {
    Increasing,
    Stable,
    Decreasing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCostEntry {
    pub run_id: String,
    pub cost_usd: f64,
    pub stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    pub total_cost_usd: f64,
    pub cost_by_stage: HashMap<String, f64>,
    pub cost_trend: Trend,
    pub highest_cost_runs: Vec<RunCostEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySequence {
    pub run_id: String,
    pub tool_name: String,
    pub count: usize,
    pub first_arg_sample: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPatterns {
    pub call_counts: HashMap<String, u64>,
    pub retry_sequences: Vec<RetrySequence>,
    pub error_rates: HashMap<String, f64>,
    pub top_context_consumers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPressure {
    pub avg_turns: f64,
    pub median_turns: f64,
    pub compression_events: usize,
    pub avg_cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPattern {
    pub pattern: String,
    pub count: usize,
    pub example_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureAnalysis {
    pub failure_rate: f64,
    pub failure_by_stage: HashMap<String, f64>,
    pub common_errors: Vec<ErrorPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendationCategory {
    Tooling,
    Skills,
    SharedService,
    Context,
    Workflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub category: RecommendationCategory,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub evidence: String,
}
