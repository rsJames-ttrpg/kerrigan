use std::collections::HashMap;

use anyhow::Result;

use crate::parse::{ConversationSummary, SessionDetail};
use crate::report::{ContextPressure, CostSummary, ErrorPattern, FailureAnalysis, RetrySequence, RunCostEntry, Trend, ToolPatterns};

pub fn build_cost_summary(conversations: &[ConversationSummary], stage_map: &HashMap<String, String>) -> Result<CostSummary> {
    let total_cost_usd: f64 = conversations.iter().map(|c| c.cost_usd).sum();

    let mut cost_by_stage: HashMap<String, f64> = HashMap::new();
    for c in conversations {
        let stage = stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string());
        *cost_by_stage.entry(stage).or_default() += c.cost_usd;
    }

    let mut highest_cost_runs: Vec<RunCostEntry> = conversations
        .iter()
        .map(|c| RunCostEntry {
            run_id: c.run_id.clone(),
            cost_usd: c.cost_usd,
            stage: stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string()),
        })
        .collect();
    highest_cost_runs.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
    highest_cost_runs.truncate(10);

    // Simple trend: compare first half avg to second half avg
    let cost_trend = if conversations.len() >= 4 {
        let mid = conversations.len() / 2;
        let first_avg: f64 = conversations[..mid].iter().map(|c| c.cost_usd).sum::<f64>() / mid as f64;
        let second_avg: f64 = conversations[mid..].iter().map(|c| c.cost_usd).sum::<f64>() / (conversations.len() - mid) as f64;
        if second_avg > first_avg * 1.2 {
            Trend::Increasing
        } else if second_avg < first_avg * 0.8 {
            Trend::Decreasing
        } else {
            Trend::Stable
        }
    } else {
        Trend::Stable
    };

    Ok(CostSummary {
        total_cost_usd,
        cost_by_stage,
        cost_trend,
        highest_cost_runs,
    })
}

pub fn build_tool_patterns(sessions: &[SessionDetail]) -> Result<ToolPatterns> {
    let mut call_counts: HashMap<String, u64> = HashMap::new();
    let mut error_counts: HashMap<String, u64> = HashMap::new();
    let mut total_counts: HashMap<String, u64> = HashMap::new();

    for session in sessions {
        for tc in &session.tool_calls {
            *call_counts.entry(tc.name.clone()).or_default() += 1;
            *total_counts.entry(tc.name.clone()).or_default() += 1;
            if tc.is_error {
                *error_counts.entry(tc.name.clone()).or_default() += 1;
            }
        }
    }

    let error_rates: HashMap<String, f64> = total_counts
        .iter()
        .map(|(name, total)| {
            let errors = error_counts.get(name).copied().unwrap_or(0);
            (name.clone(), errors as f64 / *total as f64)
        })
        .collect();

    let retry_sequences = detect_retries(sessions);

    // Top context consumers: tools with highest total call count (proxy for context usage)
    let mut sorted_tools: Vec<(String, u64)> = call_counts.clone().into_iter().collect();
    sorted_tools.sort_by(|a, b| b.1.cmp(&a.1));
    let top_context_consumers: Vec<String> = sorted_tools.into_iter().take(5).map(|(name, _)| name).collect();

    Ok(ToolPatterns {
        call_counts,
        retry_sequences,
        error_rates,
        top_context_consumers,
    })
}

fn detect_retries(sessions: &[SessionDetail]) -> Vec<RetrySequence> {
    let mut retries = Vec::new();
    for session in sessions {
        let calls = &session.tool_calls;
        let mut i = 0;
        while i < calls.len() {
            let mut count = 1;
            while i + count < calls.len()
                && calls[i + count].name == calls[i].name
                && calls[i + count].first_arg == calls[i].first_arg
                && !calls[i].first_arg.is_empty()
            {
                count += 1;
            }
            if count >= 3 {
                retries.push(RetrySequence {
                    run_id: session.run_id.clone(),
                    tool_name: calls[i].name.clone(),
                    count,
                    first_arg_sample: calls[i].first_arg.clone(),
                });
            }
            i += count;
        }
    }
    retries
}

pub fn build_context_pressure(conversations: &[ConversationSummary], sessions: &[SessionDetail]) -> ContextPressure {
    let turns: Vec<f64> = conversations.iter().map(|c| c.num_turns as f64).collect();
    let avg_turns = if turns.is_empty() { 0.0 } else { turns.iter().sum::<f64>() / turns.len() as f64 };

    let median_turns = if turns.is_empty() {
        0.0
    } else {
        let mut sorted = turns.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };

    let compression_events: usize = sessions.iter().map(|s| s.compression_events).sum();

    let avg_cache_hit_ratio = if conversations.is_empty() {
        0.0
    } else {
        let ratios: Vec<f64> = conversations
            .iter()
            .filter_map(|c| {
                let total = c.cache_read_tokens + c.cache_creation_tokens;
                if total > 0 {
                    Some(c.cache_read_tokens as f64 / total as f64)
                } else {
                    None
                }
            })
            .collect();
        if ratios.is_empty() { 0.0 } else { ratios.iter().sum::<f64>() / ratios.len() as f64 }
    };

    ContextPressure {
        avg_turns,
        median_turns,
        compression_events,
        avg_cache_hit_ratio,
    }
}

pub fn build_failure_analysis(conversations: &[ConversationSummary], stage_map: &HashMap<String, String>) -> FailureAnalysis {
    let total = conversations.len();
    let failures: Vec<&ConversationSummary> = conversations.iter().filter(|c| !c.success).collect();
    let failure_rate = if total > 0 { failures.len() as f64 / total as f64 } else { 0.0 };

    let mut stage_failures: HashMap<String, (u64, u64)> = HashMap::new(); // (failures, total)
    for c in conversations {
        let stage = stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string());
        let entry = stage_failures.entry(stage).or_insert((0, 0));
        entry.1 += 1;
        if !c.success {
            entry.0 += 1;
        }
    }
    let failure_by_stage: HashMap<String, f64> = stage_failures
        .into_iter()
        .map(|(stage, (f, t))| (stage, if t > 0 { f as f64 / t as f64 } else { 0.0 }))
        .collect();

    // For common_errors, we'd need error messages from run results — for v1, keep it simple
    let common_errors: Vec<ErrorPattern> = Vec::new();

    FailureAnalysis {
        failure_rate,
        failure_by_stage,
        common_errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_conversation(run_id: &str, cost: f64, turns: u64, success: bool) -> ConversationSummary {
        ConversationSummary {
            run_id: run_id.to_string(),
            cost_usd: cost,
            num_turns: turns,
            duration_ms: 60000,
            success,
            input_tokens: 100,
            output_tokens: 500,
            cache_read_tokens: 50000,
            cache_creation_tokens: 10000,
        }
    }

    #[test]
    fn test_cost_summary_basic() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, true),
            make_conversation("r3", 0.5, 10, false),
        ];
        let mut stages = HashMap::new();
        stages.insert("r1".to_string(), "implement".to_string());
        stages.insert("r2".to_string(), "implement".to_string());
        stages.insert("r3".to_string(), "review".to_string());

        let summary = build_cost_summary(&convos, &stages).unwrap();
        assert!((summary.total_cost_usd - 3.5).abs() < f64::EPSILON);
        assert_eq!(summary.cost_by_stage["implement"], 3.0);
        assert_eq!(summary.cost_by_stage["review"], 0.5);
        assert_eq!(summary.highest_cost_runs[0].run_id, "r2");
    }

    #[test]
    fn test_retry_detection() {
        let sessions = vec![SessionDetail {
            run_id: "r1".to_string(),
            tool_calls: vec![
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: true },
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: true },
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: false },
                crate::parse::ToolCall { name: "Read".to_string(), first_arg: "/src/lib.rs".to_string(), is_error: false },
            ],
            message_count: 8,
            compression_events: 0,
        }];

        let patterns = build_tool_patterns(&sessions).unwrap();
        assert_eq!(patterns.retry_sequences.len(), 1);
        assert_eq!(patterns.retry_sequences[0].tool_name, "Edit");
        assert_eq!(patterns.retry_sequences[0].count, 3);
    }

    #[test]
    fn test_failure_analysis() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, false),
            make_conversation("r3", 0.5, 10, true),
            make_conversation("r4", 1.5, 30, false),
        ];
        let mut stages = HashMap::new();
        stages.insert("r1".to_string(), "implement".to_string());
        stages.insert("r2".to_string(), "implement".to_string());
        stages.insert("r3".to_string(), "review".to_string());
        stages.insert("r4".to_string(), "review".to_string());

        let analysis = build_failure_analysis(&convos, &stages);
        assert!((analysis.failure_rate - 0.5).abs() < f64::EPSILON);
        assert!((analysis.failure_by_stage["implement"] - 0.5).abs() < f64::EPSILON);
        assert!((analysis.failure_by_stage["review"] - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_context_pressure() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, true),
            make_conversation("r3", 0.5, 60, true),
        ];
        let sessions = vec![
            SessionDetail { run_id: "r1".to_string(), tool_calls: vec![], message_count: 20, compression_events: 0 },
            SessionDetail { run_id: "r2".to_string(), tool_calls: vec![], message_count: 40, compression_events: 1 },
            SessionDetail { run_id: "r3".to_string(), tool_calls: vec![], message_count: 60, compression_events: 2 },
        ];

        let pressure = build_context_pressure(&convos, &sessions);
        assert!((pressure.avg_turns - 40.0).abs() < f64::EPSILON);
        assert_eq!(pressure.compression_events, 3);
    }
}
