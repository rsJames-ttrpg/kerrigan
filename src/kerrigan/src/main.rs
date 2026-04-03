mod completers;
mod display;

use std::io::IsTerminal;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use clap_complete::engine::ArgValueCompleter;
use nydus::NydusClient;

use completers::{ArtifactIdCompleter, DefinitionCompleter, RunIdCompleter};

#[derive(Parser)]
#[command(name = "kerrigan", about = "Kerrigan operator console")]
struct Cli {
    /// Overseer URL
    #[arg(long, env = "KERRIGAN_URL", default_value = "http://localhost:3100")]
    url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Submit a problem into the dev loop
    Submit {
        /// Problem description
        problem: String,
        /// Override config values (key=value)
        #[arg(long = "set", value_name = "KEY=VALUE")]
        overrides: Vec<String>,
        /// Job definition name to use
        #[arg(long, default_value = "default", add = ArgValueCompleter::new(DefinitionCompleter))]
        definition: String,
        /// Branch to clone (defaults to repo default branch)
        #[arg(long)]
        branch: Option<String>,
    },
    /// Show job status
    Status {
        /// Job run ID (lists all if omitted)
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: Option<String>,
    },
    /// Approve a job at a gate
    Approve {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
    },
    /// Reject a job at a gate
    Reject {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
        /// Rejection reason
        #[arg(long)]
        message: String,
    },
    /// Submit an OAuth code for a running job
    Auth {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
        /// OAuth code
        code: String,
    },
    /// View run output and decisions
    Log {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
    },
    /// Watch a run until completion
    Watch {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
        /// Poll interval in seconds
        #[arg(long, default_value = "3")]
        interval: u64,
    },
    /// Cancel a running or pending job
    Cancel {
        /// Job run ID
        #[arg(add = ArgValueCompleter::new(RunIdCompleter))]
        run_id: String,
    },
    /// List hatcheries and capacity
    Hatcheries {
        /// Filter by status (online, degraded, offline)
        #[arg(long)]
        status: Option<String>,
    },
    /// List and view artifacts
    Artifacts {
        #[command(subcommand)]
        action: ArtifactsAction,
    },
    /// Manage repository credentials
    Creds {
        #[command(subcommand)]
        action: CredsAction,
    },
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
    /// Generate shell completions
    Completions {
        /// Shell to generate for
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum ArtifactsAction {
    /// List artifacts with optional filters
    List {
        /// Filter by run ID (prefix ok)
        #[arg(long, add = ArgValueCompleter::new(RunIdCompleter))]
        run: Option<String>,
        /// Filter by artifact type (conversation, session, evolution-report, generic)
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,
        /// Only show artifacts created after this timestamp (RFC 3339)
        #[arg(long)]
        since: Option<String>,
    },
    /// Fetch and display artifact content
    Get {
        /// Artifact ID (prefix ok)
        #[arg(add = ArgValueCompleter::new(ArtifactIdCompleter))]
        id: String,
    },
}

#[derive(Subcommand)]
enum CredsAction {
    /// Add a credential for a repo pattern
    Add {
        /// URL pattern (e.g. "github.com/org/*" or "github.com/org/repo")
        #[arg(long)]
        pattern: String,
        /// Credential type
        #[arg(long = "type", default_value = "github_pat")]
        credential_type: String,
        /// Secret value (reads from KERRIGAN_CRED_SECRET env var, or stdin if omitted)
        #[arg(long, env = "KERRIGAN_CRED_SECRET")]
        secret: Option<String>,
    },
    /// List all credentials (secrets redacted)
    List,
    /// Remove a credential
    Rm {
        /// Credential ID (prefix ok)
        id: String,
    },
}

fn main() -> Result<()> {
    // Dynamic completions — runs before tokio runtime to avoid nested runtime panic
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = Cli::parse();
    let client = NydusClient::new(&cli.url);

    match cli.command {
        Command::Submit {
            problem,
            overrides,
            definition,
            branch,
        } => {
            cmd_submit(
                &client,
                &definition,
                &problem,
                &overrides,
                branch.as_deref(),
            )
            .await
        }
        Command::Status { run_id } => cmd_status(&client, run_id.as_deref()).await,
        Command::Approve { run_id } => cmd_approve(&client, &run_id).await,
        Command::Reject { run_id, message } => cmd_reject(&client, &run_id, &message).await,
        Command::Auth { run_id, code } => cmd_auth(&client, &run_id, &code).await,
        Command::Log { run_id } => cmd_log(&client, &run_id).await,
        Command::Watch { run_id, interval } => cmd_watch(&client, &run_id, interval).await,
        Command::Cancel { run_id } => cmd_cancel(&client, &run_id).await,
        Command::Artifacts { action } => match action {
            ArtifactsAction::List { run, r#type, since } => {
                cmd_artifacts_list(&client, run.as_deref(), r#type.as_deref(), since.as_deref())
                    .await
            }
            ArtifactsAction::Get { id } => cmd_artifacts_get(&client, &id).await,
        },
        Command::Hatcheries { status } => cmd_hatcheries(&client, status.as_deref()).await,
        Command::Creds { action } => match action {
            CredsAction::Add {
                pattern,
                credential_type,
                secret,
            } => {
                let secret = match secret {
                    Some(s) => s,
                    None => {
                        if std::io::stdin().is_terminal() {
                            eprint!("Secret: ");
                        }
                        let mut buf = String::new();
                        std::io::stdin().read_line(&mut buf)?;
                        buf.trim().to_string()
                    }
                };
                cmd_creds_add(&client, &pattern, &credential_type, &secret).await
            }
            CredsAction::List => cmd_creds_list(&client).await,
            CredsAction::Rm { id } => cmd_creds_rm(&client, &id).await,
        },
        Command::Evolve {
            since,
            min_sessions,
            submit,
            json,
        } => cmd_evolve(&client, since.as_deref(), min_sessions, submit, json).await,
        Command::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "kerrigan",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

async fn cmd_submit(
    client: &NydusClient,
    definition_name: &str,
    problem: &str,
    overrides: &[String],
    branch: Option<&str>,
) -> Result<()> {
    let definitions = client.list_definitions().await?;
    let def = definitions
        .iter()
        .find(|d| d.name == definition_name)
        .ok_or_else(|| anyhow::anyhow!("job definition '{}' not found", definition_name))?;

    let mut config = serde_json::json!({ "task": problem });
    for kv in overrides {
        let (key, value) = kv.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid override format '{}', expected key=value", kv)
        })?;
        let parts: Vec<&str> = key.splitn(2, '.').collect();
        if parts.len() == 2 {
            if config.get(parts[0]).is_none() {
                config[parts[0]] = serde_json::json!({});
            }
            config[parts[0]][parts[1]] = serde_json::Value::String(value.to_string());
        } else {
            config[key] = serde_json::Value::String(value.to_string());
        }
    }
    if let Some(b) = branch {
        config["branch"] = serde_json::Value::String(b.to_string());
    }

    let run = client
        .start_run(&def.id, "operator", None, Some(config))
        .await?;

    println!(
        "Started run {} ({}) -- watch with: kerrigan watch {}",
        display::short_id(&run.id),
        definition_name,
        display::short_id(&run.id),
    );

    Ok(())
}

async fn cmd_status(client: &NydusClient, run_id: Option<&str>) -> Result<()> {
    let (runs, definitions) = tokio::try_join!(client.list_runs(None), client.list_definitions(),)?;

    match run_id {
        Some(partial) => {
            let id = display::resolve_run_id(&runs, partial)?;
            let run = runs.iter().find(|r| r.id == id).unwrap();
            let tasks = client.list_tasks(None, None, Some(id)).await?;
            display::print_run_detail(run, &definitions, &tasks, &runs);
        }
        None => {
            display::print_run_list(&runs, &definitions);
        }
    }
    Ok(())
}

async fn cmd_approve(client: &NydusClient, run_id: &str) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?;

    let next_run = client.advance_run(id).await?;
    let definitions = client.list_definitions().await?;
    let def_name = definitions
        .iter()
        .find(|d| d.id == next_run.definition_id)
        .map(|d| d.name.as_str())
        .unwrap_or("?");

    println!(
        "Approved {}. Next: {} ({})",
        display::short_id(id),
        display::short_id(&next_run.id),
        def_name,
    );
    Ok(())
}

async fn cmd_reject(client: &NydusClient, run_id: &str, message: &str) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?;
    client
        .update_run(id, Some("failed"), None, Some(message))
        .await?;
    println!("Rejected: {}", display::short_id(id));
    Ok(())
}

async fn cmd_auth(client: &NydusClient, run_id: &str, code: &str) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?;
    client.submit_auth_code(id, code).await?;
    println!("Auth code submitted for run {}", display::short_id(id));
    Ok(())
}

async fn cmd_log(client: &NydusClient, run_id: &str) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?;
    let artifacts = client.list_artifacts(Some(id), None, None).await?;
    let tasks = client.list_tasks(None, None, Some(id)).await?;
    display::print_log(&artifacts, &tasks, id);
    Ok(())
}

async fn cmd_watch(client: &NydusClient, run_id: &str, interval_secs: u64) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?.to_string();
    let definitions = client.list_definitions().await?;

    loop {
        // Clear screen (only in terminal, not when piped)
        if std::io::stdout().is_terminal() {
            print!("\x1b[2J\x1b[H");
        }

        let runs = client.list_runs(None).await?;
        let run = runs
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| anyhow::anyhow!("run '{}' disappeared", id))?;
        let tasks = client.list_tasks(None, None, Some(&id)).await?;

        display::print_run_detail(run, &definitions, &tasks, &runs);
        println!("\nWatching... (Ctrl+C to stop)");

        match run.status.as_str() {
            "completed" | "failed" | "cancelled" => {
                println!("Run reached terminal state: {}", run.status);
                break;
            }
            _ => {}
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
    Ok(())
}

async fn cmd_cancel(client: &NydusClient, run_id: &str) -> Result<()> {
    let runs = client.list_runs(None).await?;
    let id = display::resolve_run_id(&runs, run_id)?;
    client
        .update_run(id, Some("cancelled"), None, Some("cancelled by operator"))
        .await?;
    println!("Cancelled: {}", display::short_id(id));
    Ok(())
}

async fn cmd_artifacts_list(
    client: &NydusClient,
    run: Option<&str>,
    artifact_type: Option<&str>,
    since: Option<&str>,
) -> Result<()> {
    let resolved_run_id = if let Some(partial) = run {
        let runs = client.list_runs(None).await?;
        Some(display::resolve_run_id(&runs, partial)?.to_string())
    } else {
        None
    };
    let artifacts = client
        .list_artifacts(resolved_run_id.as_deref(), artifact_type, since)
        .await?;
    display::print_artifacts_list(&artifacts);
    Ok(())
}

async fn cmd_artifacts_get(client: &NydusClient, partial_id: &str) -> Result<()> {
    let artifacts = client.list_artifacts(None, None, None).await?;
    let artifact = display::resolve_artifact(&artifacts, partial_id)?;
    let id = artifact.id.clone();
    let is_gzip = artifact.content_type == "application/gzip";

    let data = client.get_artifact(&id).await?;

    let content = if is_gzip {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(data.as_slice());
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)?;
        decompressed
    } else {
        data
    };

    use std::io::Write;
    std::io::stdout().write_all(&content)?;
    if std::io::stdout().is_terminal() && content.last() != Some(&b'\n') {
        println!();
    }
    Ok(())
}

async fn cmd_hatcheries(client: &NydusClient, status: Option<&str>) -> Result<()> {
    let hatcheries = client.list_hatcheries(status).await?;
    display::print_hatcheries(&hatcheries);
    Ok(())
}

async fn cmd_creds_add(
    client: &NydusClient,
    pattern: &str,
    credential_type: &str,
    secret: &str,
) -> Result<()> {
    let cred = client
        .create_credential(pattern, credential_type, secret)
        .await?;
    println!(
        "Created credential {} for pattern '{}' (type: {})",
        display::short_id(&cred.id),
        cred.pattern,
        cred.credential_type,
    );
    Ok(())
}

async fn cmd_creds_list(client: &NydusClient) -> Result<()> {
    let creds = client.list_credentials().await?;
    if creds.is_empty() {
        println!("No credentials configured.");
        return Ok(());
    }
    for cred in &creds {
        println!(
            "  {} {} [{}]",
            display::short_id(&cred.id),
            cred.pattern,
            cred.credential_type,
        );
    }
    Ok(())
}

async fn cmd_creds_rm(client: &NydusClient, id: &str) -> Result<()> {
    // For prefix matching, list creds and find the one that starts with id
    let creds = client.list_credentials().await?;
    let matching: Vec<_> = creds.iter().filter(|c| c.id.starts_with(id)).collect();
    match matching.len() {
        0 => anyhow::bail!("no credential matching '{id}'"),
        1 => {
            client.delete_credential(&matching[0].id).await?;
            println!("Deleted credential {}", display::short_id(&matching[0].id));
        }
        n => anyhow::bail!("ambiguous prefix '{id}' matches {n} credentials"),
    }
    Ok(())
}

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
