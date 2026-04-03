# Manual Evolution Trigger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kerrigan evolve` CLI command for on-demand evolution analysis and fix the Queen actor recovery bug.

**Architecture:** The kerrigan CLI gains a direct dependency on the `evolution` crate, calling `EvolutionChamber::analyze()` client-side with the existing nydus HTTP client. No Overseer API changes. The Queen actor bugfix is a one-line change.

**Tech Stack:** Rust (edition 2024), clap, owo-colors, evolution crate, nydus client, chrono

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| Modify | `src/kerrigan/Cargo.toml` | Add `evolution` and `chrono` deps |
| Modify | `src/kerrigan/BUCK` | Add Buck2 deps for evolution and chrono |
| Modify | `src/kerrigan/src/main.rs` | Add `Evolve` command variant and `cmd_evolve` handler |
| Modify | `src/kerrigan/src/display.rs` | Add `print_evolution_report` formatted output |
| Modify | `src/queen/src/actors/evolution.rs` | Fix `recover_last_analysis_time` fallback |

---

### Task 1: Fix Queen actor recovery bug

**Files:**
- Modify: `src/queen/src/actors/evolution.rs:176-200`

- [ ] **Step 1: Fix the "no reports" fallback**

In `recover_last_analysis_time`, change the two `Utc::now()` fallbacks to `DateTime::<Utc>::MIN_UTC`. This ensures the first analysis catches all historical artifacts.

In `src/queen/src/actors/evolution.rs`, replace the function body:

```rust
/// Fetch the most recent evolution-report artifact to recover the last analysis timestamp.
/// Falls back to the minimum DateTime if no previous report exists, so the first analysis
/// catches all historical artifacts.
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
                DateTime::<Utc>::MIN_UTC
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to fetch evolution report history, starting fresh");
            DateTime::<Utc>::MIN_UTC
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/queen && cargo check`
Expected: compiles cleanly

- [ ] **Step 3: Commit**

```bash
git add src/queen/src/actors/evolution.rs
git commit -m "fix: evolution actor recovery uses MIN_UTC instead of now()

First boot no longer drops pre-existing artifacts from analysis window."
```

---

### Task 2: Add evolution and chrono deps to kerrigan

**Files:**
- Modify: `src/kerrigan/Cargo.toml`
- Modify: `src/kerrigan/BUCK`

- [ ] **Step 1: Add Cargo dependencies**

In `src/kerrigan/Cargo.toml`, add to `[dependencies]`:

```toml
evolution = { path = "../evolution" }
chrono = { version = "0.4", features = ["serde"] }
```

- [ ] **Step 2: Add Buck2 dependencies**

In `src/kerrigan/BUCK`, add to `KERRIGAN_DEPS`:

```python
"//src/evolution:evolution",
"//third-party:chrono",
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src/kerrigan && cargo check`
Expected: compiles cleanly (no new code using the deps yet, but linkage is verified)

- [ ] **Step 4: Commit**

```bash
git add src/kerrigan/Cargo.toml src/kerrigan/BUCK
git commit -m "deps: add evolution and chrono to kerrigan CLI"
```

---

### Task 3: Add report display formatting

**Files:**
- Modify: `src/kerrigan/src/display.rs`

- [ ] **Step 1: Add the `print_evolution_report` function**

Add this at the bottom of `src/kerrigan/src/display.rs`:

```rust
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

    println!("Evolution Report ({})", report.generated_at.format("%Y-%m-%dT%H:%M:%SZ"));
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
    println!("  Total: ${:.2} | Trend: {}", report.cost_summary.total_cost_usd, trend);
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
            let total = report.tool_patterns.call_counts.get(*tool).copied().unwrap_or(0);
            let errors = (*rate * total as f64).round() as u64;
            println!("  Error rate: {} {:.0}% ({}/{})", tool, rate * 100.0, errors, total);
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
                let count = report.tool_patterns.call_counts.get(t).copied().unwrap_or(0);
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
    println!("  Compression events: {}", report.context_pressure.compression_events);
    println!("  Cache hit ratio: {:.0}%", report.context_pressure.avg_cache_hit_ratio * 100.0);

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
```

- [ ] **Step 2: Verify it compiles**

Run: `cd src/kerrigan && cargo check`
Expected: compiles cleanly (function exists but isn't called yet)

- [ ] **Step 3: Commit**

```bash
git add src/kerrigan/src/display.rs
git commit -m "feat: add evolution report terminal display formatting"
```

---

### Task 4: Add Evolve command to CLI

**Files:**
- Modify: `src/kerrigan/src/main.rs`

- [ ] **Step 1: Add the Evolve variant to the Command enum**

In `src/kerrigan/src/main.rs`, add this variant to the `Command` enum (after `Creds`):

```rust
    /// Run evolution analysis on demand
    Evolve {
        /// Only analyze artifacts after this time (RFC 3339)
        #[arg(long)]
        since: Option<String>,
        /// Minimum sessions required for analysis
        #[arg(long, default_value = "5")]
        min_sessions: usize,
        /// Also submit an evolve-from-analysis job
        #[arg(long)]
        submit: bool,
        /// Output raw JSON instead of formatted report
        #[arg(long)]
        json: bool,
    },
```

- [ ] **Step 2: Add the match arm in async_main**

In the `match cli.command` block (before `Command::Completions`), add:

```rust
        Command::Evolve {
            since,
            min_sessions,
            submit,
            json,
        } => cmd_evolve(&client, since.as_deref(), min_sessions, submit, json).await,
```

- [ ] **Step 3: Add the cmd_evolve function**

Add this function to `src/kerrigan/src/main.rs`:

```rust
async fn cmd_evolve(
    client: &NydusClient,
    since: Option<&str>,
    min_sessions: usize,
    submit: bool,
    json: bool,
) -> Result<()> {
    use chrono::{DateTime, Utc};
    use evolution::EvolutionChamber;
    use evolution::report::AnalysisScope;

    let since: DateTime<Utc> = match since {
        Some(s) => s.parse().map_err(|e| anyhow::anyhow!("invalid --since timestamp: {e}"))?,
        None => DateTime::<Utc>::MIN_UTC,
    };

    eprintln!("Running evolution analysis...");
    let chamber = EvolutionChamber::new(client.clone());
    let report = chamber
        .analyze(AnalysisScope::Global, since, min_sessions)
        .await?;

    let report = match report {
        Some(r) => r,
        None => {
            eprintln!(
                "Insufficient data for analysis (need at least {} conversation artifacts).",
                min_sessions,
            );
            return Ok(());
        }
    };

    // Display
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        display::print_evolution_report(&report);
    }

    // Store artifact
    let report_json = serde_json::to_string_pretty(&report)?;
    let artifact_name = format!(
        "evolution-report-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
    );
    let artifact = client
        .store_artifact(
            &artifact_name,
            "application/json",
            report_json.as_bytes(),
            None,
            Some("evolution-report"),
        )
        .await?;
    eprintln!("\nReport stored: {}", display::short_id(&artifact.id));

    // Optionally submit evolve job
    if submit {
        let definitions = client.list_definitions().await?;
        let def = definitions
            .iter()
            .find(|d| d.name == "evolve-from-analysis")
            .ok_or_else(|| anyhow::anyhow!("job definition 'evolve-from-analysis' not found"))?;

        let task = format!(
            "Analyze the following Evolution Chamber report and create GitHub issues \
             for each high/medium severity recommendation.\n\n\
             Label issues with `evolution-chamber`.\n\n```json\n{}\n```",
            report_json,
        );
        let config_overrides = serde_json::json!({ "task": task, "stage": "evolve" });
        let run = client
            .start_run(&def.id, "operator", None, Some(config_overrides))
            .await?;
        eprintln!(
            "Submitted evolve job: {} -- watch with: kerrigan watch {}",
            display::short_id(&run.id),
            display::short_id(&run.id),
        );
    }

    Ok(())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src/kerrigan && cargo check`
Expected: compiles cleanly

- [ ] **Step 5: Verify Buck2 build**

Run: `buck2 build root//src/kerrigan:kerrigan`
Expected: builds successfully

- [ ] **Step 6: Commit**

```bash
git add src/kerrigan/src/main.rs
git commit -m "feat: add kerrigan evolve command for on-demand evolution analysis"
```

---

### Task 5: Smoke test

- [ ] **Step 1: Run kerrigan evolve --help**

Run: `buck2 run root//src/kerrigan:kerrigan -- evolve --help`
Expected: shows help text with `--since`, `--min-sessions`, `--submit`, `--json` flags

- [ ] **Step 2: Run against live Overseer (if available)**

Run: `buck2 run root//src/kerrigan:kerrigan -- evolve`
Expected: either prints a formatted report and "Report stored: <id>", or prints "Insufficient data" if fewer than 5 conversation artifacts exist.

- [ ] **Step 3: Test --json flag**

Run: `buck2 run root//src/kerrigan:kerrigan -- evolve --json`
Expected: outputs valid JSON to stdout (report), status messages to stderr.

- [ ] **Step 4: Verify Queen actor still builds**

Run: `cd src/queen && cargo check`
Expected: compiles cleanly with the recovery bug fix from Task 1.

- [ ] **Step 5: Run existing tests**

Run: `cd src/evolution && cargo test`
Run: `cd src/queen && cargo test`
Expected: all pass

- [ ] **Step 6: Final commit (if any fixups needed)**

Only if previous steps required adjustments. Otherwise skip.
