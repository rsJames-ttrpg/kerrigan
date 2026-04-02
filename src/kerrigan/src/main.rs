mod completers;
mod display;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use clap_complete::engine::ArgValueCompleter;
use nydus::NydusClient;

use completers::{DefinitionCompleter, RunIdCompleter};

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
        /// Optional message
        #[arg(long)]
        message: Option<String>,
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
    /// Generate shell completions
    Completions {
        /// Shell to generate for
        shell: Shell,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Dynamic completions — when COMPLETE env var is set, prints script and exits
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

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
        Command::Approve { run_id, message } => {
            cmd_approve(&client, &run_id, message.as_deref()).await
        }
        Command::Reject { run_id, message } => cmd_reject(&client, &run_id, &message).await,
        Command::Auth { run_id, code } => cmd_auth(&client, &run_id, &code).await,
        Command::Log { run_id } => cmd_log(&client, &run_id).await,
        Command::Watch { run_id, interval } => cmd_watch(&client, &run_id, interval).await,
        Command::Cancel { run_id } => cmd_cancel(&client, &run_id).await,
        Command::Hatcheries { status } => cmd_hatcheries(&client, status.as_deref()).await,
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

async fn cmd_approve(client: &NydusClient, run_id: &str, _message: Option<&str>) -> Result<()> {
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
        // Clear screen
        print!("\x1b[2J\x1b[H");

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
        .update_run(id, Some("failed"), None, Some("cancelled by operator"))
        .await?;
    println!("Cancelled: {}", display::short_id(id));
    Ok(())
}

async fn cmd_hatcheries(client: &NydusClient, status: Option<&str>) -> Result<()> {
    let hatcheries = client.list_hatcheries(status).await?;
    display::print_hatcheries(&hatcheries);
    Ok(())
}
