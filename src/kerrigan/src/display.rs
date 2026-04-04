use std::collections::HashMap;
use std::io::IsTerminal;

use chrono::Datelike;
use nydus::{Artifact, Hatchery, JobDefinition, JobRun, Task};
use owo_colors::OwoColorize;
use serde_json::Value;

fn use_color() -> bool {
    std::io::stdout().is_terminal()
}

pub fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
}

fn colored_status(status: &str) -> String {
    if !use_color() {
        return status.to_string();
    }
    match status {
        "completed" => status.green().to_string(),
        "running" => status.yellow().to_string(),
        "pending" => status.cyan().to_string(),
        "failed" => status.red().to_string(),
        "cancelled" => status.dimmed().to_string(),
        _ => status.to_string(),
    }
}

fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

fn truncate_task(config_overrides: &Option<Value>, max_len: usize) -> String {
    let task = config_overrides
        .as_ref()
        .and_then(|c| c.get("task"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if task.is_empty() {
        return String::new();
    }
    let first_line = task.lines().next().unwrap_or(task);
    if first_line.chars().count() <= max_len {
        format!("\"{}\"", first_line)
    } else {
        format!("\"{}...\"", truncate_chars(first_line, max_len - 3))
    }
}

fn def_name(definition_id: &str, def_map: &HashMap<String, String>) -> String {
    def_map
        .get(definition_id)
        .cloned()
        .unwrap_or_else(|| short_id(definition_id).to_string())
}

fn build_def_map(definitions: &[JobDefinition]) -> HashMap<String, String> {
    definitions
        .iter()
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect()
}

/// Group runs into pipeline chains (linked by parent_id) and standalone runs.
fn group_pipelines(runs: &[JobRun]) -> (Vec<Vec<&JobRun>>, Vec<&JobRun>) {
    let run_map: HashMap<&str, &JobRun> = runs.iter().map(|r| (r.id.as_str(), r)).collect();

    // Find roots (no parent or parent not in our set)
    let mut roots: Vec<&JobRun> = runs
        .iter()
        .filter(|r| {
            r.parent_id.is_none() || !run_map.contains_key(r.parent_id.as_deref().unwrap_or(""))
        })
        .collect();
    roots.sort_by_key(|r| &r.id);

    // Build child index
    let mut children: HashMap<&str, Vec<&JobRun>> = HashMap::new();
    for run in runs {
        if let Some(ref pid) = run.parent_id {
            children.entry(pid.as_str()).or_default().push(run);
        }
    }

    let mut pipelines = Vec::new();
    let mut standalone = Vec::new();

    for root in roots {
        let mut chain = vec![root];
        let mut current = root;
        let mut visited = std::collections::HashSet::new();
        visited.insert(current.id.as_str());
        while let Some(kids) = children.get(current.id.as_str()) {
            if let Some(next) = kids.first() {
                if !visited.insert(next.id.as_str()) {
                    break;
                }
                chain.push(next);
                current = next;
            } else {
                break;
            }
        }
        if chain.len() > 1 {
            pipelines.push(chain);
        } else {
            standalone.push(root);
        }
    }

    (pipelines, standalone)
}

pub fn print_run_list(runs: &[JobRun], definitions: &[JobDefinition]) {
    if runs.is_empty() {
        println!("No runs found.");
        return;
    }

    let def_map = build_def_map(definitions);
    let (pipelines, standalone) = group_pipelines(runs);

    for (i, chain) in pipelines.iter().enumerate() {
        let task_desc = truncate_task(&chain[0].config_overrides, 60);
        let stage_names: Vec<String> = chain
            .iter()
            .map(|r| def_name(&r.definition_id, &def_map))
            .collect();

        if i > 0 {
            println!();
        }
        print!("Pipeline:");
        if !task_desc.is_empty() {
            print!(" {task_desc}");
        }
        println!();

        if use_color() {
            println!("  {}", stage_names.join(" -> ").dimmed());
        } else {
            println!("  {}", stage_names.join(" -> "));
        }

        for run in chain {
            let marker = match run.status.as_str() {
                "completed" => "  ",
                "failed" => "  ",
                "running" => " >",
                "pending" => " ?",
                _ => "  ",
            };
            println!(
                "{}  {}  {:<24} {}",
                marker,
                short_id(&run.id),
                def_name(&run.definition_id, &def_map),
                colored_status(&run.status),
            );
        }
    }

    if !standalone.is_empty() {
        if !pipelines.is_empty() {
            println!();
        }
        println!("Standalone:");
        for run in &standalone {
            let task_desc = truncate_task(&run.config_overrides, 50);
            let attention = if run.status == "pending" {
                "  [needs attention]"
            } else {
                ""
            };
            println!(
                "    {}  {:<24} {}  {}{}",
                short_id(&run.id),
                def_name(&run.definition_id, &def_map),
                colored_status(&run.status),
                task_desc,
                attention,
            );
        }
    }
}

pub fn print_run_detail(
    run: &JobRun,
    definitions: &[JobDefinition],
    tasks: &[Task],
    all_runs: &[JobRun],
) {
    let def_map = build_def_map(definitions);
    let task_desc = truncate_task(&run.config_overrides, 80);

    println!("Run:        {}", short_id(&run.id));
    println!("Definition: {}", def_name(&run.definition_id, &def_map));
    println!("Status:     {}", colored_status(&run.status));
    println!("Triggered:  {}", run.triggered_by);
    if !task_desc.is_empty() {
        println!("Task:       {task_desc}");
    }
    if let Some(ref err) = run.error {
        println!(
            "Error:      {}",
            if use_color() {
                err.as_str().red().to_string()
            } else {
                err.clone()
            }
        );
    }

    if !tasks.is_empty() {
        println!("\nTasks:");
        for task in tasks {
            println!("  [{}] {}", colored_status(&task.status), task.subject);
        }
    }

    // Pipeline chain
    let run_map: HashMap<&str, &JobRun> = all_runs.iter().map(|r| (r.id.as_str(), r)).collect();

    // Walk up to root
    let mut root_id: String = run.id.clone();
    let mut visited = std::collections::HashSet::<String>::new();
    loop {
        if !visited.insert(root_id.clone()) {
            break;
        }
        match run_map
            .get(root_id.as_str())
            .and_then(|r| r.parent_id.as_ref())
        {
            Some(pid) => root_id = pid.clone(),
            None => break,
        }
    }

    // Walk down from root, preferring the branch that contains our target run
    let mut children = HashMap::<&str, Vec<&JobRun>>::new();
    for r in all_runs {
        if let Some(ref pid) = r.parent_id {
            children.entry(pid.as_str()).or_default().push(r);
        }
    }

    // Collect all descendants of the target run for branch selection
    let target_id = &run.id;
    let mut target_ancestors = std::collections::HashSet::<&str>::new();
    {
        let mut walk = target_id.as_str();
        target_ancestors.insert(walk);
        while let Some(r) = run_map.get(walk) {
            if let Some(ref pid) = r.parent_id {
                target_ancestors.insert(pid.as_str());
                walk = pid.as_str();
            } else {
                break;
            }
        }
    }

    let mut chain: Vec<&JobRun> = Vec::new();
    let mut visited = std::collections::HashSet::<String>::new();
    let mut current_id: Option<String> = Some(root_id);
    while let Some(cid) = current_id {
        if !visited.insert(cid.clone()) {
            break;
        }
        if let Some(r) = run_map.get(cid.as_str()) {
            chain.push(*r);
            // Pick the child on the branch containing our target run, else first
            current_id = children.get(cid.as_str()).and_then(|kids| {
                kids.iter()
                    .find(|k| target_ancestors.contains(k.id.as_str()) || k.id == *target_id)
                    .or(kids.first())
                    .map(|r| r.id.clone())
            });
        } else {
            break;
        }
    }

    if chain.len() > 1 {
        println!("\nPipeline:");
        for r in &chain {
            let marker = if r.id == run.id {
                " >"
            } else {
                match r.status.as_str() {
                    "completed" => "  ",
                    "failed" => "  ",
                    _ => "  ",
                }
            };
            println!(
                "  {} {} {:<24} {}",
                marker,
                short_id(&r.id),
                def_name(&r.definition_id, &def_map),
                colored_status(&r.status),
            );
        }
    }
}

pub fn print_hatcheries(hatcheries: &[Hatchery]) {
    if hatcheries.is_empty() {
        println!("No hatcheries found.");
        return;
    }
    for h in hatcheries {
        let capacity = format!("{}/{}", h.active_drones, h.max_concurrency);
        println!(
            "  {}  {:<28} {}  drones: {}",
            short_id(&h.id),
            h.name,
            colored_status(&h.status),
            capacity,
        );
    }
}

fn colored_artifact_type(t: &str) -> String {
    if !use_color() {
        return t.to_string();
    }
    match t {
        "conversation" => t.cyan().to_string(),
        "session" => t.yellow().to_string(),
        "evolution-report" => t.green().to_string(),
        "generic" => t.dimmed().to_string(),
        _ => t.to_string(),
    }
}

pub fn print_artifacts_list(artifacts: &[Artifact]) {
    if artifacts.is_empty() {
        println!("No artifacts found.");
        return;
    }

    for a in artifacts {
        let type_label = colored_artifact_type(&a.artifact_type);
        let run_label = a.run_id.as_deref().map(short_id).unwrap_or("-");
        let ts = a
            .created_at
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();

        println!(
            "  {}  {:<40} {:<20} run:{}  {:>8}  {}",
            short_id(&a.id),
            a.name,
            type_label,
            run_label,
            humanize_bytes(a.size),
            if use_color() {
                ts.dimmed().to_string()
            } else {
                ts
            },
        );
    }
}

pub fn resolve_artifact<'a>(
    artifacts: &'a [Artifact],
    partial: &str,
) -> anyhow::Result<&'a Artifact> {
    if let Some(a) = artifacts.iter().find(|a| a.id == partial) {
        return Ok(a);
    }
    let matches: Vec<&Artifact> = artifacts
        .iter()
        .filter(|a| a.id.starts_with(partial))
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no artifact matching '{partial}'"),
        1 => Ok(matches[0]),
        n => anyhow::bail!("'{partial}' is ambiguous ({n} artifacts — use more characters)"),
    }
}

pub fn print_log(artifacts: &[Artifact], tasks: &[Task], run_id: &str) {
    if artifacts.is_empty() && tasks.is_empty() {
        println!("No artifacts or tasks for run {}.", short_id(run_id));
        return;
    }

    if !artifacts.is_empty() {
        println!("Artifacts:");
        for a in artifacts {
            let type_label = if a.artifact_type != "generic" {
                format!(" ({})", a.artifact_type)
            } else {
                String::new()
            };
            println!(
                "  {}  {}{}  {}",
                short_id(&a.id),
                a.name,
                type_label,
                humanize_bytes(a.size),
            );
        }
    }

    if !tasks.is_empty() {
        if !artifacts.is_empty() {
            println!();
        }
        println!("Tasks:");
        for task in tasks {
            println!("  [{}] {}", colored_status(&task.status), task.subject);
            if let Some(ref output) = task.output {
                if let Ok(pretty) = serde_json::to_string_pretty(output) {
                    for line in pretty.lines().take(5) {
                        println!("    {line}");
                    }
                }
            }
        }
    }
}

fn humanize_bytes(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn print_evolution_report(report: &evolution::report::AnalysisReport) {
    let scope_label = match &report.scope {
        evolution::report::AnalysisScope::Global => "global".to_string(),
        evolution::report::AnalysisScope::Repo(url) => url.clone(),
    };
    let period_start = if report.period_start.year() < 0 {
        "all time".to_string()
    } else {
        report.period_start.format("%Y-%m-%d %H:%M").to_string()
    };

    println!(
        "Evolution Report ({})",
        report.generated_at.format("%Y-%m-%dT%H:%M:%SZ")
    );
    println!(
        "Scope: {} | Period: {} -> {} | Runs analyzed: {}",
        scope_label,
        period_start,
        report.period_end.format("%Y-%m-%d %H:%M"),
        report.runs_analyzed,
    );

    // Cost Summary
    println!("\nCost Summary");
    let trend = match &report.cost_summary.cost_trend {
        evolution::report::Trend::Increasing => "Increasing",
        evolution::report::Trend::Stable => "Stable",
        evolution::report::Trend::Decreasing => "Decreasing",
    };
    println!(
        "  Total: ${:.2} | Trend: {}",
        report.cost_summary.total_cost_usd, trend
    );
    if !report.cost_summary.highest_cost_runs.is_empty() {
        let top: Vec<String> = report
            .cost_summary
            .highest_cost_runs
            .iter()
            .take(5)
            .map(|r| format!("{} ${:.2}", short_id(&r.run_id), r.cost_usd))
            .collect();
        println!("  Top runs: {}", top.join(", "));
    }
    if !report.cost_summary.cost_by_stage.is_empty() {
        let stages: Vec<String> = report
            .cost_summary
            .cost_by_stage
            .iter()
            .map(|(stage, cost)| format!("{stage}: ${cost:.2}"))
            .collect();
        println!("  By stage: {}", stages.join(", "));
    }

    // Tool Patterns
    println!("\nTool Patterns");
    let mut error_rates: Vec<(&String, &f64)> = report
        .tool_patterns
        .error_rates
        .iter()
        .filter(|(_, rate)| **rate > 0.0)
        .collect();
    error_rates.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    if error_rates.is_empty() {
        println!("  Error rates: none");
    } else {
        for (tool, rate) in &error_rates {
            let total = report
                .tool_patterns
                .call_counts
                .get(*tool)
                .copied()
                .unwrap_or(0);
            let errors = (*rate * total as f64).round() as u64;
            println!(
                "  Error rate: {} {:.0}% ({}/{})",
                tool,
                *rate * 100.0,
                errors,
                total
            );
        }
    }
    if !report.tool_patterns.retry_sequences.is_empty() {
        for seq in &report.tool_patterns.retry_sequences {
            println!(
                "  Retry: {} retried {}x on {} (run {})",
                seq.tool_name,
                seq.count,
                truncate_chars(&seq.first_arg_sample, 40),
                short_id(&seq.run_id),
            );
        }
    }
    if !report.tool_patterns.top_context_consumers.is_empty() {
        let consumers: Vec<String> = report
            .tool_patterns
            .top_context_consumers
            .iter()
            .map(|t| {
                let count = report
                    .tool_patterns
                    .call_counts
                    .get(t)
                    .copied()
                    .unwrap_or(0);
                format!("{t} ({count})")
            })
            .collect();
        println!("  Top consumers: {}", consumers.join(", "));
    }

    // Context Pressure
    println!("\nContext Pressure");
    println!(
        "  Avg turns: {:.0} | Median turns: {:.0}",
        report.context_pressure.avg_turns, report.context_pressure.median_turns,
    );
    println!(
        "  Compression events: {}",
        report.context_pressure.compression_events
    );
    println!(
        "  Cache hit ratio: {:.0}%",
        report.context_pressure.avg_cache_hit_ratio * 100.0
    );

    // Failure Analysis
    println!("\nFailure Analysis");
    println!(
        "  Overall: {:.0}% ({}/{})",
        report.failure_analysis.failure_rate * 100.0,
        (report.failure_analysis.failure_rate * report.runs_analyzed as f64).round() as usize,
        report.runs_analyzed,
    );
    if !report.failure_analysis.failure_by_stage.is_empty() {
        let stages: Vec<String> = report
            .failure_analysis
            .failure_by_stage
            .iter()
            .map(|(stage, rate)| format!("{stage} {:.0}%", rate * 100.0))
            .collect();
        println!("  By stage: {}", stages.join(", "));
    }

    // Recommendations
    if report.recommendations.is_empty() {
        println!("\nNo recommendations.");
    } else {
        println!("\nRecommendations");
        for rec in &report.recommendations {
            let severity_tag = match rec.severity {
                evolution::report::Severity::High => {
                    if use_color() {
                        "[HIGH]".red().bold().to_string()
                    } else {
                        "[HIGH]".to_string()
                    }
                }
                evolution::report::Severity::Medium => {
                    if use_color() {
                        "[MED] ".yellow().bold().to_string()
                    } else {
                        "[MED] ".to_string()
                    }
                }
                evolution::report::Severity::Low => {
                    if use_color() {
                        "[LOW] ".dimmed().to_string()
                    } else {
                        "[LOW] ".to_string()
                    }
                }
            };
            println!("  {} {} -- {}", severity_tag, rec.title, rec.detail);
            if !rec.evidence.is_empty() {
                if use_color() {
                    println!("        {}", rec.evidence.dimmed());
                } else {
                    println!("        {}", rec.evidence);
                }
            }
        }
    }
}

/// Resolve a partial (short) run ID to a full ID by prefix matching.
pub fn resolve_run_id<'a>(runs: &'a [JobRun], partial: &str) -> anyhow::Result<&'a str> {
    // Exact match first
    if let Some(run) = runs.iter().find(|r| r.id == partial) {
        return Ok(&run.id);
    }
    // Prefix match
    let matches: Vec<&JobRun> = runs.iter().filter(|r| r.id.starts_with(partial)).collect();
    match matches.len() {
        0 => anyhow::bail!("no run matching '{partial}'"),
        1 => Ok(&matches[0].id),
        n => anyhow::bail!("'{partial}' is ambiguous ({n} matches — use more characters)"),
    }
}
