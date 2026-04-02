use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
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
        /// Job definition name to use
        #[arg(long, default_value = "default")]
        definition: String,
        /// Branch to clone (defaults to repo default branch)
        #[arg(long)]
        branch: Option<String>,
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
    /// Generate shell completions
    Completions {
        /// Shell to generate for
        shell: Shell,
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
    // 1. Resolve definition by name
    let definitions = client.list_definitions().await?;
    let def = definitions
        .iter()
        .find(|d| d.name == definition_name)
        .ok_or_else(|| anyhow::anyhow!("job definition '{}' not found", definition_name))?;

    // 2. Build config overrides
    // The problem description becomes the task for the drone
    let mut config = serde_json::json!({ "task": problem });
    for kv in overrides {
        let (key, value) = kv.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid override format '{}', expected key=value", kv)
        })?;
        // Support nested keys: "secrets.github_pat=xxx" -> {"secrets": {"github_pat": "xxx"}}
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

    // 3. Start run (Queen polls for pending runs and claims them)
    let run = client
        .start_run(&def.id, "operator", None, Some(config))
        .await?;
    println!("Started run: {}", run.id);
    println!("Waiting for a Queen to claim the job...");

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

            // Show pipeline chain
            // Walk up to find root (with cycle guard)
            let mut root_id = id.to_string();
            let mut visited = std::collections::HashSet::new();
            loop {
                if !visited.insert(root_id.clone()) {
                    break; // cycle detected
                }
                let r = runs.iter().find(|r| r.id == root_id);
                match r.and_then(|r| r.parent_id.as_ref()) {
                    Some(pid) => root_id = pid.clone(),
                    None => break,
                }
            }

            // Walk down from root to collect chain (with cycle guard)
            let mut chain = Vec::new();
            let mut visited = std::collections::HashSet::new();
            let mut current_id = Some(root_id);
            while let Some(cid) = current_id {
                if !visited.insert(cid.clone()) {
                    break; // cycle detected
                }
                if let Some(r) = runs.iter().find(|r| r.id == cid) {
                    chain.push(r);
                    current_id = runs
                        .iter()
                        .find(|r| r.parent_id.as_deref() == Some(&cid))
                        .map(|r| r.id.clone());
                } else {
                    break;
                }
            }

            if chain.len() > 1 {
                println!("\n  Pipeline:");
                for r in &chain {
                    let marker = if r.id == id {
                        "→"
                    } else if r.status == "completed" {
                        "✓"
                    } else if r.status == "failed" {
                        "✗"
                    } else {
                        " "
                    };
                    println!("    {} {} — {}", marker, r.id, r.status);
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
    let next_run = client.advance_run(run_id).await?;
    println!("Approved: {}", run_id);
    println!(
        "Next stage started: {} (definition: {})",
        next_run.id, next_run.definition_id
    );
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
