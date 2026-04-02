use crate::report::*;

pub fn generate_recommendations(
    cost: &CostSummary,
    tools: &ToolPatterns,
    context: &ContextPressure,
    failures: &FailureAnalysis,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    recs.extend(cost_rules(cost));
    recs.extend(tool_rules(tools));
    recs.extend(context_rules(context));
    recs.extend(failure_rules(failures));
    // Sort by severity: High first
    recs.sort_by(|a, b| severity_ord(&a.severity).cmp(&severity_ord(&b.severity)));
    recs
}

fn severity_ord(s: &Severity) -> u8 {
    match s {
        Severity::High => 0,
        Severity::Medium => 1,
        Severity::Low => 2,
    }
}

fn cost_rules(cost: &CostSummary) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if matches!(cost.cost_trend, Trend::Increasing) {
        recs.push(Recommendation {
            category: RecommendationCategory::Workflow,
            severity: Severity::Medium,
            title: "Cost trend is increasing".to_string(),
            detail: "Average cost per run is trending upward. Review prompt sizes, task complexity, or stage configuration.".to_string(),
            evidence: format!("Total cost: ${:.2}", cost.total_cost_usd),
        });
    }
    for entry in cost.highest_cost_runs.iter().take(3) {
        if entry.cost_usd > 3.0 {
            recs.push(Recommendation {
                category: RecommendationCategory::Workflow,
                severity: Severity::High,
                title: format!("High-cost run: ${:.2} ({})", entry.cost_usd, entry.stage),
                detail: "This run exceeded $3.00. Consider splitting into smaller tasks or reviewing the stage prompt.".to_string(),
                evidence: format!("Run {} cost ${:.2}", entry.run_id, entry.cost_usd),
            });
        }
    }
    recs
}

fn tool_rules(tools: &ToolPatterns) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    // High error rate tools
    for (name, rate) in &tools.error_rates {
        if *rate > 0.15 && tools.call_counts.get(name).copied().unwrap_or(0) >= 5 {
            recs.push(Recommendation {
                category: RecommendationCategory::Tooling,
                severity: Severity::High,
                title: format!("High error rate for tool `{name}`: {:.0}%", rate * 100.0),
                detail: format!("Tool `{name}` fails more than 15% of the time. Investigate common failure modes and consider pre-validation or better error messages."),
                evidence: format!("{} calls, {:.0}% error rate", tools.call_counts[name], rate * 100.0),
            });
        }
    }

    // Retry sequences
    for retry in &tools.retry_sequences {
        if retry.count >= 3 {
            recs.push(Recommendation {
                category: RecommendationCategory::Tooling,
                severity: Severity::Medium,
                title: format!("`{}` retried {} times on same target", retry.tool_name, retry.count),
                detail: format!("Drone called `{}` {} times with the same first argument '{}'. This wastes context and suggests the drone needs better feedback or a different approach.", retry.tool_name, retry.count, retry.first_arg_sample),
                evidence: format!("Run {}", retry.run_id),
            });
        }
    }

    // Dominant tool (>40% of all calls)
    let total_calls: u64 = tools.call_counts.values().sum();
    if total_calls > 0 {
        for (name, count) in &tools.call_counts {
            let ratio = *count as f64 / total_calls as f64;
            if ratio > 0.40 && *count > 20 {
                recs.push(Recommendation {
                    category: RecommendationCategory::Skills,
                    severity: Severity::Low,
                    title: format!("`{name}` dominates tool usage at {:.0}%", ratio * 100.0),
                    detail: format!("Consider whether a higher-level skill or MCP tool could replace multiple `{name}` calls."),
                    evidence: format!("{count} of {total_calls} total calls"),
                });
            }
        }
    }

    recs
}

fn context_rules(context: &ContextPressure) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if context.compression_events > 0 {
        recs.push(Recommendation {
            category: RecommendationCategory::Context,
            severity: if context.compression_events > 3 { Severity::High } else { Severity::Medium },
            title: format!("{} context compression events detected", context.compression_events),
            detail: "Drones are hitting context limits and losing conversation history. Consider splitting tasks, reducing prompt sizes, or adding shared services to reduce tool call volume.".to_string(),
            evidence: format!("Avg turns: {:.0}, Median turns: {:.0}, Cache hit ratio: {:.0}%", context.avg_turns, context.median_turns, context.avg_cache_hit_ratio * 100.0),
        });
    }
    recs
}

fn failure_rules(failures: &FailureAnalysis) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if failures.failure_rate > 0.3 {
        recs.push(Recommendation {
            category: RecommendationCategory::Workflow,
            severity: Severity::High,
            title: format!("High failure rate: {:.0}%", failures.failure_rate * 100.0),
            detail: "More than 30% of runs are failing. Review drone configuration, task quality, and error patterns.".to_string(),
            evidence: format!("Failure rate by stage: {:?}", failures.failure_by_stage),
        });
    }
    for (stage, rate) in &failures.failure_by_stage {
        if *rate > 0.5 {
            recs.push(Recommendation {
                category: RecommendationCategory::Workflow,
                severity: Severity::High,
                title: format!("Stage `{stage}` failing >50% of the time"),
                detail: format!("The `{stage}` stage has a {:.0}% failure rate. The stage prompt, tool access, or task format likely needs adjustment.", rate * 100.0),
                evidence: format!("{stage} failure rate: {:.0}%", rate * 100.0),
            });
        }
    }
    recs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_high_cost_recommendation() {
        let cost = CostSummary {
            total_cost_usd: 15.0,
            cost_by_stage: HashMap::new(),
            cost_trend: Trend::Increasing,
            highest_cost_runs: vec![
                RunCostEntry { run_id: "r1".into(), cost_usd: 5.0, stage: "implement".into() },
            ],
        };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("Cost trend")));
        assert!(recs.iter().any(|r| r.title.contains("High-cost run")));
    }

    #[test]
    fn test_tool_error_rate_recommendation() {
        let mut error_rates = HashMap::new();
        error_rates.insert("Edit".to_string(), 0.25);
        let mut call_counts = HashMap::new();
        call_counts.insert("Edit".to_string(), 20);

        let tools = ToolPatterns { call_counts, retry_sequences: vec![], error_rates, top_context_consumers: vec![] };
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("High error rate") && r.title.contains("Edit")));
    }

    #[test]
    fn test_compression_recommendation() {
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 80.0, median_turns: 75.0, compression_events: 5, avg_cache_hit_ratio: 0.6 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("compression")));
        assert!(recs.iter().any(|r| matches!(r.severity, Severity::High)));
    }

    #[test]
    fn test_no_recommendations_when_clean() {
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.1, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.is_empty());
    }
}
