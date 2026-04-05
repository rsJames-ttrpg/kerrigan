mod proto {
    tonic::include_proto!("creep.v1");
}

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use proto::file_index_client::FileIndexClient;
use proto::lsp_service_client::LspServiceClient;

#[derive(Parser)]
#[command(name = "creep-cli", about = "CLI client for Creep file index")]
struct Cli {
    /// Creep server address
    #[arg(
        long,
        default_value = "http://localhost:9090",
        global = true,
        env = "CREEP_ADDR"
    )]
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
    /// Search for symbols by name, or list symbols in a file
    Symbols {
        /// Symbol name to search for (substring match, case-insensitive)
        query: Option<String>,
        /// List all symbols in this file instead of searching by name
        #[arg(long)]
        file: Option<String>,
        /// Filter by symbol kind (function, struct, enum, trait, impl, const, static, type_alias, module, macro)
        #[arg(long)]
        kind: Option<String>,
        /// Filter by workspace path
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Get LSP diagnostics for a workspace or file
    Diagnostics {
        /// Workspace path to get diagnostics for
        workspace: String,
        /// Get diagnostics for a specific file only
        #[arg(long)]
        file: Option<String>,
        /// Minimum severity: error, warning, info, hint
        #[arg(long, default_value = "warning")]
        severity: String,
    },
    /// Go to definition of symbol at file:line:column
    Definition {
        /// Location in file:line:column format (1-indexed)
        location: String,
    },
    /// Find references to symbol at file:line:column
    References {
        /// Location in file:line:column format (1-indexed)
        location: String,
        /// Include the declaration in results
        #[arg(long)]
        include_declaration: bool,
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
        Commands::Symbols {
            query,
            file,
            kind,
            workspace,
        } => {
            if let Some(file_path) = file {
                cmd_list_file_symbols(&mut client, &file_path, cli.json).await
            } else {
                cmd_search_symbols(
                    &mut client,
                    query.as_deref().unwrap_or(""),
                    kind,
                    workspace,
                    cli.json,
                )
                .await
            }
        }
        Commands::Diagnostics {
            workspace,
            file,
            severity,
        } => {
            let mut lsp_client = LspServiceClient::connect(cli.addr.clone())
                .await
                .with_context(|| format!("failed to connect to Creep at {}", cli.addr))?;
            cmd_diagnostics(&mut lsp_client, &workspace, file, &severity, cli.json).await
        }
        Commands::Definition { location } => {
            let mut lsp_client = LspServiceClient::connect(cli.addr.clone())
                .await
                .with_context(|| format!("failed to connect to Creep at {}", cli.addr))?;
            cmd_definition(&mut lsp_client, &location, cli.json).await
        }
        Commands::References {
            location,
            include_declaration,
        } => {
            let mut lsp_client = LspServiceClient::connect(cli.addr.clone())
                .await
                .with_context(|| format!("failed to connect to Creep at {}", cli.addr))?;
            cmd_references(&mut lsp_client, &location, include_declaration, cli.json).await
        }
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
        print_json(&serde_json::json!({
            "files_indexed": count,
            "path": path,
        }))?;
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
        print_json(&serde_json::json!({
            "unregistered": true,
            "path": path,
        }))?;
    } else {
        println!("Unregistered workspace {path}");
    }
    Ok(())
}

async fn cmd_search_symbols(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    query: &str,
    kind: Option<String>,
    workspace: Option<String>,
    json: bool,
) -> Result<()> {
    let response = client
        .search_symbols(proto::SearchSymbolsRequest {
            query: query.to_string(),
            kind,
            workspace,
        })
        .await
        .context("search_symbols RPC failed")?;

    let symbols = response.into_inner().symbols;
    if json {
        print_json(&symbols)?;
    } else {
        for s in &symbols {
            let parent_str = match s.parent.as_deref() {
                Some(p) if !p.is_empty() => format!(" ({p})"),
                _ => String::new(),
            };
            let display_name = s.signature.as_deref().unwrap_or(&s.name);
            println!(
                "{:<10} {}{:<40} {}:{}",
                s.kind,
                display_name,
                parent_str,
                s.file,
                s.line + 1,
            );
        }
        if symbols.is_empty() {
            eprintln!("no symbols found matching '{query}'");
        }
    }
    Ok(())
}

async fn cmd_list_file_symbols(
    client: &mut FileIndexClient<tonic::transport::Channel>,
    path: &str,
    json: bool,
) -> Result<()> {
    let response = client
        .list_file_symbols(proto::ListFileSymbolsRequest {
            path: path.to_string(),
        })
        .await
        .context("list_file_symbols RPC failed")?;

    let symbols = response.into_inner().symbols;
    if json {
        print_json(&symbols)?;
    } else {
        for s in &symbols {
            let parent_str = match s.parent.as_deref() {
                Some(p) if !p.is_empty() => format!(" ({p})"),
                _ => String::new(),
            };
            let display_name = s.signature.as_deref().unwrap_or(&s.name);
            println!(
                "  {:>4}  {:<10} {}{}",
                s.line + 1,
                s.kind,
                display_name,
                parent_str,
            );
        }
        if symbols.is_empty() {
            eprintln!("no symbols found in '{path}'");
        }
    }
    Ok(())
}

async fn cmd_diagnostics(
    client: &mut LspServiceClient<tonic::transport::Channel>,
    workspace: &str,
    file: Option<String>,
    severity: &str,
    json: bool,
) -> Result<()> {
    if let Some(file_path) = file {
        let response = client
            .get_file_diagnostics(proto::GetFileDiagnosticsRequest {
                workspace_path: workspace.to_string(),
                file_path,
            })
            .await
            .context("get_file_diagnostics RPC failed")?;

        let diagnostics = response.into_inner().diagnostics;
        if json {
            print_json(&diagnostics)?;
        } else {
            print_diagnostics_markdown(&diagnostics);
        }
    } else {
        let min_severity = match severity {
            "error" => 1,
            "warning" => 2,
            "info" => 3,
            _ => 4,
        };

        let response = client
            .get_diagnostics(proto::GetDiagnosticsRequest {
                workspace_path: workspace.to_string(),
                min_severity,
                max_results: 0,
            })
            .await
            .context("get_diagnostics RPC failed")?;

        let inner = response.into_inner();
        if json {
            print_json(&inner.diagnostics)?;
        } else {
            print_diagnostics_markdown(&inner.diagnostics);
        }
    }
    Ok(())
}

fn print_diagnostics_markdown(diagnostics: &[proto::Diagnostic]) {
    let errors: Vec<_> = diagnostics.iter().filter(|d| d.severity == "error").collect();
    let warnings: Vec<_> = diagnostics.iter().filter(|d| d.severity == "warning").collect();
    let infos: Vec<_> = diagnostics.iter().filter(|d| d.severity == "info").collect();
    let hints: Vec<_> = diagnostics.iter().filter(|d| d.severity == "hint").collect();

    println!(
        "## Workspace Diagnostics ({} error{}, {} warning{})",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
        warnings.len(),
        if warnings.len() == 1 { "" } else { "s" },
    );
    println!();

    if !errors.is_empty() {
        println!("### Errors");
        for d in &errors {
            let source = if d.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", d.source)
            };
            println!("- `{}:{}:{}` — {}{}", d.file_path, d.line + 1, d.column + 1, d.message, source);
        }
        println!();
    }

    if !warnings.is_empty() {
        println!("### Warnings");
        for d in &warnings {
            let source = if d.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", d.source)
            };
            println!("- `{}:{}:{}` — {}{}", d.file_path, d.line + 1, d.column + 1, d.message, source);
        }
        println!();
    }

    if !infos.is_empty() {
        println!("### Info");
        for d in &infos {
            let source = if d.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", d.source)
            };
            println!("- `{}:{}:{}` — {}{}", d.file_path, d.line + 1, d.column + 1, d.message, source);
        }
        println!();
    }

    if !hints.is_empty() {
        println!("### Hints");
        for d in &hints {
            let source = if d.source.is_empty() {
                String::new()
            } else {
                format!(" ({})", d.source)
            };
            println!("- `{}:{}:{}` — {}{}", d.file_path, d.line + 1, d.column + 1, d.message, source);
        }
        println!();
    }

    if diagnostics.is_empty() {
        println!("No diagnostics found.");
    }
}

/// Parse a "file:line:column" location string (1-indexed).
fn parse_location(location: &str) -> Result<(String, u32, u32)> {
    let parts: Vec<&str> = location.rsplitn(3, ':').collect();
    if parts.len() != 3 {
        anyhow::bail!(
            "invalid location format '{}': expected file:line:column (1-indexed)",
            location
        );
    }
    let column: u32 = parts[0]
        .parse()
        .with_context(|| format!("invalid column in '{location}'"))?;
    let line: u32 = parts[1]
        .parse()
        .with_context(|| format!("invalid line in '{location}'"))?;
    let file = parts[2].to_string();

    if line == 0 || column == 0 {
        anyhow::bail!("line and column must be 1-indexed (got {line}:{column})");
    }

    Ok((file, line - 1, column - 1)) // Convert to 0-indexed for LSP
}

async fn cmd_definition(
    client: &mut LspServiceClient<tonic::transport::Channel>,
    location: &str,
    json: bool,
) -> Result<()> {
    let (file, line, column) = parse_location(location)?;

    let response = client
        .goto_definition(proto::GotoDefinitionRequest {
            file_path: file,
            line,
            column,
        })
        .await
        .context("goto_definition RPC failed")?;

    let locations = response.into_inner().locations;
    if json {
        print_json(&locations)?;
    } else {
        for loc in &locations {
            println!(
                "{}:{}:{}",
                loc.file_path,
                loc.start_line + 1,
                loc.start_column + 1
            );
        }
        if locations.is_empty() {
            eprintln!("no definition found");
        }
    }
    Ok(())
}

async fn cmd_references(
    client: &mut LspServiceClient<tonic::transport::Channel>,
    location: &str,
    include_declaration: bool,
    json: bool,
) -> Result<()> {
    let (file, line, column) = parse_location(location)?;

    let response = client
        .find_references(proto::FindReferencesRequest {
            file_path: file,
            line,
            column,
            include_declaration,
        })
        .await
        .context("find_references RPC failed")?;

    let locations = response.into_inner().locations;
    if json {
        print_json(&locations)?;
    } else {
        for loc in &locations {
            println!(
                "{}:{}:{}",
                loc.file_path,
                loc.start_line + 1,
                loc.start_column + 1
            );
        }
        if locations.is_empty() {
            eprintln!("no references found");
        }
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
    if hash.len() > 12 { &hash[..12] } else { hash }
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

    #[test]
    fn test_parse_location_valid() {
        let (file, line, col) = parse_location("/src/main.rs:42:5").unwrap();
        assert_eq!(file, "/src/main.rs");
        assert_eq!(line, 41); // 0-indexed
        assert_eq!(col, 4); // 0-indexed
    }

    #[test]
    fn test_parse_location_with_colons_in_path() {
        // Windows paths or URLs with colons
        let (file, line, col) = parse_location("C:/src/main.rs:10:1").unwrap();
        assert_eq!(file, "C:/src/main.rs");
        assert_eq!(line, 9);
        assert_eq!(col, 0);
    }

    #[test]
    fn test_parse_location_invalid_format() {
        assert!(parse_location("just/a/file").is_err());
        assert!(parse_location("file:10").is_err());
    }

    #[test]
    fn test_parse_location_zero_indexed_rejected() {
        assert!(parse_location("/src/main.rs:0:5").is_err());
        assert!(parse_location("/src/main.rs:1:0").is_err());
    }
}
