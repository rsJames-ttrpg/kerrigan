use std::collections::HashMap;
use std::io::IsTerminal;

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
    if first_line.len() <= max_len {
        format!("\"{}\"", first_line)
    } else {
        format!("\"{}...\"", &first_line[..max_len - 3])
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

    // Walk down from root
    let mut children = HashMap::<&str, &JobRun>::new();
    for r in all_runs {
        if let Some(ref pid) = r.parent_id {
            children.insert(pid.as_str(), r);
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
            current_id = children.get(cid.as_str()).map(|r| r.id.clone());
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
