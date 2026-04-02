mod proto {
    tonic::include_proto!("creep.v1");
}

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use proto::file_index_client::FileIndexClient;

#[derive(Parser)]
#[command(name = "creep-cli", about = "CLI client for Creep file index")]
struct Cli {
    /// Creep server address
    #[arg(long, default_value = "http://localhost:9090", global = true)]
    addr: String,

    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search for files by glob pattern
    Search {
        /// Glob pattern to match (e.g. "*.rs", "src/**/*.toml")
        pattern: String,
        /// Filter by workspace path
        #[arg(long)]
        workspace: Option<String>,
        /// Filter by file type (e.g. "rust", "python")
        #[arg(long, name = "type")]
        file_type: Option<String>,
    },
    /// Get metadata for a specific file
    Metadata {
        /// Absolute path to the file
        path: String,
    },
    /// Register a workspace for indexing
    Register {
        /// Absolute path to the workspace directory
        path: String,
    },
    /// Unregister a workspace
    Unregister {
        /// Absolute path to the workspace directory
        path: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut client = FileIndexClient::connect(cli.addr.clone())
        .await
        .with_context(|| format!("failed to connect to Creep at {}", cli.addr))?;

    match cli.command {
        Commands::Search {
            pattern,
            workspace,
            file_type,
        } => cmd_search(&mut client, &pattern, workspace, file_type, cli.json).await,
        Commands::Metadata { path } => cmd_metadata(&mut client, &path, cli.json).await,
        Commands::Register { path } => cmd_register(&mut client, &path, cli.json).await,
        Commands::Unregister { path } => cmd_unregister(&mut client, &path, cli.json).await,
    }
}

async fn cmd_search(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    pattern: &str,
    workspace: Option<String>,
    file_type: Option<String>,
    json: bool,
) -> Result<()> {
    let response = client
        .search_files(proto::SearchFilesRequest {
            pattern: pattern.to_string(),
            workspace,
            file_type,
        })
        .await
        .context("search_files RPC failed")?;

    let files = response.into_inner().files;
    if json {
        print_json(&files)?;
    } else {
        for f in &files {
            println!(
                "{:<60} {:>8}  {}  {}  {}",
                f.path,
                format_size(f.size),
                format_time(f.modified_at),
                f.file_type,
                truncate_hash(&f.content_hash),
            );
        }
        if files.is_empty() {
            eprintln!("no files matched pattern '{pattern}'");
        }
    }
    Ok(())
}

async fn cmd_metadata(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .get_file_metadata(proto::GetFileMetadataRequest {
            path: path.to_string(),
        })
        .await
        .context("get_file_metadata RPC failed")?;

    match response.into_inner().file {
        Some(f) => {
            if json {
                print_json(&f)?;
            } else {
                println!("path:    {}", f.path);
                println!("size:    {}", format_size(f.size));
                println!("modified: {}", format_time(f.modified_at));
                println!("type:    {}", f.file_type);
                println!("hash:    {}", f.content_hash);
            }
        }
        None => {
            eprintln!("file not found in index: {path}");
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn cmd_register(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .register_workspace(proto::RegisterWorkspaceRequest {
            path: path.to_string(),
        })
        .await
        .context("register_workspace RPC failed")?;

    let count = response.into_inner().files_indexed;
    if json {
        println!(r#"{{"files_indexed":{count},"path":"{path}"}}"#);
    } else {
        println!("Indexed {count} files in {path}");
    }
    Ok(())
}

async fn cmd_unregister(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    client
        .unregister_workspace(proto::UnregisterWorkspaceRequest {
            path: path.to_string(),
        })
        .await
        .context("unregister_workspace RPC failed")?;

    if json {
        println!(r#"{{"unregistered":true,"path":"{path}"}}"#);
    } else {
        println!("Unregistered workspace {path}");
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_time(epoch_secs: i64) -> String {
    epoch_secs.to_string()
}

fn truncate_hash(hash: &str) -> &str {
    if hash.len() > 12 {
        &hash[..12]
    } else {
        hash
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn test_truncate_hash() {
        assert_eq!(truncate_hash("abcdefghijklmnop"), "abcdefghijkl");
        assert_eq!(truncate_hash("short"), "short");
        assert_eq!(truncate_hash("exactly12345"), "exactly12345");
    }

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(0), "0");
        assert_eq!(format_time(1700000000), "1700000000");
    }
}
