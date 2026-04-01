use anyhow::Result;
use clap::{Parser, Subcommand};
use nydus::NydusClient;

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
        /// Target hatchery name (auto-selects if omitted)
        #[arg(long)]
        hatchery: Option<String>,
        /// Job definition name to use
        #[arg(long, default_value = "spec-from-problem")]
        definition: String,
    },
    /// Show job status
    Status {
        /// Job run ID (lists all if omitted)
        run_id: Option<String>,
    },
    /// Approve a job at a gate
    Approve {
        /// Job run ID
        run_id: String,
        /// Optional message
        #[arg(long)]
        message: Option<String>,
    },
    /// Reject a job at a gate
    Reject {
        /// Job run ID
        run_id: String,
        /// Rejection reason
        #[arg(long)]
        message: String,
    },
    /// Submit an OAuth code for a running job
    Auth {
        /// Job run ID
        run_id: String,
        /// OAuth code
        code: String,
    },
    /// View run output and decisions
    Log {
        /// Job run ID
        run_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = NydusClient::new(&cli.url);

    match cli.command {
        Command::Submit {
            problem,
            overrides,
            hatchery,
            definition,
        } => {
            cmd_submit(
                &client,
                &definition,
                &problem,
                &overrides,
                hatchery.as_deref(),
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
    }
}

async fn cmd_submit(
    client: &NydusClient,
    definition_name: &str,
    problem: &str,
    overrides: &[String],
    hatchery_name: Option<&str>,
) -> Result<()> {
    // 1. Resolve definition by name
    let definitions = client.list_definitions().await?;
    let def = definitions
        .iter()
        .find(|d| d.name == definition_name)
        .ok_or_else(|| anyhow::anyhow!("job definition '{}' not found", definition_name))?;

    // 2. Build config overrides
    let mut config = serde_json::json!({ "problem": problem });
    for kv in overrides {
        let (key, value) = kv.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid override format '{}', expected key=value", kv)
        })?;
        config[key] = serde_json::Value::String(value.to_string());
    }

    // 3. Start run
    let run = client
        .start_run(&def.id, "operator", None, Some(config))
        .await?;
    println!("Started run: {}", run.id);

    // 4. Find hatchery
    let hatchery = if let Some(name) = hatchery_name {
        let hatcheries = client.list_hatcheries(Some("online")).await?;
        hatcheries
            .into_iter()
            .find(|h| h.name == name)
            .ok_or_else(|| anyhow::anyhow!("hatchery '{}' not found or not online", name))?
    } else {
        let hatcheries = client.list_hatcheries(Some("online")).await?;
        hatcheries
            .into_iter()
            .find(|h| h.active_drones < h.max_concurrency)
            .ok_or_else(|| anyhow::anyhow!("no hatcheries with available capacity"))?
    };

    // 5. Assign
    client.assign_job(&hatchery.id, &run.id).await?;
    println!("Assigned to hatchery: {} ({})", hatchery.name, hatchery.id);

    Ok(())
}

async fn cmd_status(client: &NydusClient, run_id: Option<&str>) -> Result<()> {
    match run_id {
        Some(id) => {
            let runs = client.list_runs(None).await?;
            let run = runs
                .iter()
                .find(|r| r.id == id)
                .ok_or_else(|| anyhow::anyhow!("run '{}' not found", id))?;
            println!("Run: {}", run.id);
            println!("  Status:       {}", run.status);
            println!("  Definition:   {}", run.definition_id);
            println!("  Triggered by: {}", run.triggered_by);
            if let Some(ref err) = run.error {
                println!("  Error:        {}", err);
            }

            let tasks = client.list_tasks(None, None, Some(id)).await?;
            if !tasks.is_empty() {
                println!("  Tasks:");
                for task in &tasks {
                    println!("    - [{}] {}", task.status, task.subject);
                }
            }
        }
        None => {
            let runs = client.list_runs(None).await?;
            if runs.is_empty() {
                println!("No runs found.");
            } else {
                for run in &runs {
                    let marker = if run.status == "pending" {
                        " [needs attention]"
                    } else {
                        ""
                    };
                    println!(
                        "{} — {} ({}){}", // changed from em dash
                        run.id, run.status, run.triggered_by, marker
                    );
                }
            }
        }
    }
    Ok(())
}

async fn cmd_approve(client: &NydusClient, run_id: &str, _message: Option<&str>) -> Result<()> {
    client
        .update_run(run_id, Some("running"), None, None)
        .await?;
    println!("Approved: {}", run_id);
    Ok(())
}

async fn cmd_reject(client: &NydusClient, run_id: &str, message: &str) -> Result<()> {
    client
        .update_run(run_id, Some("failed"), None, Some(message))
        .await?;
    println!("Rejected: {}", run_id);
    Ok(())
}

async fn cmd_auth(client: &NydusClient, run_id: &str, code: &str) -> Result<()> {
    client.submit_auth_code(run_id, code).await?;
    println!("Auth code submitted for run: {}", run_id);
    Ok(())
}

async fn cmd_log(client: &NydusClient, run_id: &str) -> Result<()> {
    let artifacts = client.list_artifacts(Some(run_id)).await?;
    if artifacts.is_empty() {
        println!("No artifacts for run {}.", run_id);
    } else {
        println!("Artifacts for run {}:", run_id);
        for a in &artifacts {
            println!("  {} — {} ({})", a.id, a.name, a.content_type);
        }
    }

    let tasks = client.list_tasks(None, None, Some(run_id)).await?;
    if !tasks.is_empty() {
        println!("\nTasks:");
        for task in &tasks {
            println!("  [{}] {}", task.status, task.subject);
            if let Some(ref output) = task.output {
                println!("    Output: {}", serde_json::to_string_pretty(output)?);
            }
        }
    }
    Ok(())
}
