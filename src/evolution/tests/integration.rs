use evolution::report::*;
use std::collections::HashMap;

#[test]
fn test_full_pipeline_with_fixtures() {
    let conversation_data = include_bytes!("fixtures/conversation.json");
    let session_data = include_bytes!("fixtures/session.jsonl");

    // Parse
    let conversation =
        evolution::parse::parse_conversation("run-fixture", conversation_data).unwrap();
    let session = evolution::parse::parse_session("run-fixture", session_data).unwrap();

    assert!(conversation.success);
    assert!(conversation.cost_usd > 1.0);
    assert_eq!(session.tool_calls.len(), 4);
    assert!(!session.tool_calls[0].is_error); // Read succeeded
    assert!(session.tool_calls[1].is_error); // Edit #1 failed
    assert!(session.tool_calls[2].is_error); // Edit #2 failed
    assert!(!session.tool_calls[3].is_error); // Edit #3 succeeded

    // Metrics
    let stage_map = HashMap::new();
    let cost = evolution::metrics::build_cost_summary(&[conversation.clone()], &stage_map).unwrap();
    let tools = evolution::metrics::build_tool_patterns(&[session.clone()]).unwrap();
    let context =
        evolution::metrics::build_context_pressure(&[conversation.clone()], &[session.clone()]);
    let failures = evolution::metrics::build_failure_analysis(&[conversation], &stage_map);

    assert!(cost.total_cost_usd > 1.0);
    assert_eq!(tools.call_counts["Edit"], 3);
    assert_eq!(tools.call_counts["Read"], 1);
    assert!(*tools.error_rates.get("Edit").unwrap() > 0.5);

    // Recommendations
    let recs = evolution::rules::generate_recommendations(&cost, &tools, &context, &failures);
    // Should flag Edit's high error rate
    assert!(recs.iter().any(|r| r.title.contains("Edit")));

    // Report serializes to JSON
    let report = AnalysisReport {
        generated_at: chrono::Utc::now(),
        scope: AnalysisScope::Global,
        period_start: chrono::Utc::now(),
        period_end: chrono::Utc::now(),
        runs_analyzed: 1,
        cost_summary: cost,
        tool_patterns: tools,
        context_pressure: context,
        failure_analysis: failures,
        recommendations: recs,
    };
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(json.contains("Edit"));
}
