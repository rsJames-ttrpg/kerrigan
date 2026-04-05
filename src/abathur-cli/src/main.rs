use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "abathur", about = "Documentation indexing and generation")]
struct Cli {
    /// Path to abathur.toml config file
    #[arg(long, default_value = "abathur.toml")]
    config: PathBuf,

    /// Output format
    #[arg(long, default_value = "md")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, clap::ValueEnum)]
enum OutputFormat {
    Md,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Search docs by text or tag
    Query {
        /// Search terms
        terms: String,
        /// Filter by tag instead of text
        #[arg(long)]
        tag: bool,
    },
    /// Read a document by slug
    Read {
        /// Document slug
        slug: String,
        /// Read only this section
        #[arg(long)]
        section: Option<String>,
    },
    /// Check for stale documents
    Check,
    /// Generate docs for a source path via Claude API
    Generate {
        /// Source path to document
        path: Option<PathBuf>,
        /// Regenerate all stale docs
        #[arg(long)]
        stale: bool,
    },
    /// Update source hashes in document frontmatter
    Hash {
        /// Path to specific doc (or --all)
        doc: Option<PathBuf>,
        /// Update all docs
        #[arg(long)]
        all: bool,
    },
    /// Create abathur.toml with defaults
    Init,
    /// Dump prompt with frontmatter schema for LLM use
    Code,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::Query { terms, tag } => cmd_query(&cli, terms, *tag),
        Command::Read { slug, section } => cmd_read(&cli, slug, section.as_deref()),
        Command::Check => cmd_check(&cli),
        Command::Generate { path, stale } => cmd_generate(&cli, path.as_deref(), *stale),
        Command::Hash { doc, all } => cmd_hash(&cli, doc.as_deref(), *all),
        Command::Init => cmd_init(&cli),
        Command::Code => cmd_code(),
    }
}

fn load_config(cli: &Cli) -> anyhow::Result<abathur::config::AbathurConfig> {
    abathur::config::AbathurConfig::load(&cli.config)
}

fn cmd_query(cli: &Cli, terms: &str, tag: bool) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let index = abathur::index::Index::build(&config)?;

    let results = if tag {
        index.query_by_tags(&[terms])
    } else {
        index.query(terms)
    };

    match cli.format {
        OutputFormat::Json => {
            let items: Vec<_> = results
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "slug": d.slug,
                        "title": d.title,
                        "description": d.description,
                        "tags": d.tags,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        OutputFormat::Md => {
            for doc in &results {
                println!("- **{}** (`{}`) — {}", doc.title, doc.slug, doc.description);
            }
            if results.is_empty() {
                println!("No matching documents found.");
            }
        }
    }
    Ok(())
}

fn cmd_read(cli: &Cli, slug: &str, section: Option<&str>) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let index = abathur::index::Index::build(&config)?;
    let content = index.read(slug, section)?;

    match cli.format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({ "slug": slug, "content": content })
            );
        }
        OutputFormat::Md => {
            println!("{content}");
        }
    }
    Ok(())
}

fn cmd_check(cli: &Cli) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let index = abathur::index::Index::build(&config)?;
    let stale = abathur::staleness::check(&index)?;

    match cli.format {
        OutputFormat::Json => {
            let items: Vec<_> = stale
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "slug": s.slug,
                        "changed_sources": s.changed_sources,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        OutputFormat::Md => {
            if stale.is_empty() {
                println!("All documents are up to date.");
            } else {
                for s in &stale {
                    println!("- **{}** — stale sources:", s.slug);
                    for src in &s.changed_sources {
                        println!("  - {}", src.display());
                    }
                }
            }
        }
    }

    if !stale.is_empty() {
        anyhow::bail!(
            "{} stale document(s) — run `abathur hash --all` to update",
            stale.len(),
        );
    }
    Ok(())
}

fn cmd_generate(cli: &Cli, path: Option<&std::path::Path>, stale: bool) -> anyhow::Result<()> {
    let config = load_config(cli)?;
    let generator = abathur::generator::Generator::new(config.clone());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    if stale {
        let index = abathur::index::Index::build(&config)?;
        let updated = rt.block_on(generator.regenerate_stale(&index))?;
        if updated.is_empty() {
            println!("All documents are up to date.");
        } else {
            for slug in &updated {
                println!("Regenerated: {slug}");
            }
        }
    } else if let Some(p) = path {
        let content = rt.block_on(generator.generate(p))?;
        println!("{content}");
    } else {
        anyhow::bail!("specify a source path or use --stale");
    }
    Ok(())
}

fn cmd_hash(cli: &Cli, doc: Option<&std::path::Path>, all: bool) -> anyhow::Result<()> {
    if all {
        let config = load_config(cli)?;
        let index = abathur::index::Index::build(&config)?;
        for meta in index.docs.values() {
            abathur::hash::update_hashes(&meta.path)?;
            println!("Updated: {}", meta.path.display());
        }
    } else if let Some(path) = doc {
        abathur::hash::update_hashes(path)?;
        println!("Updated: {}", path.display());
    } else {
        anyhow::bail!("specify a doc path or use --all");
    }
    Ok(())
}

fn cmd_init(cli: &Cli) -> anyhow::Result<()> {
    let default = r#"[index]
doc_paths = ["docs/abathur"]
exclude = []

[sources]
roots = ["src/"]
exclude = ["**/target/**"]

[generate]
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
"#;

    if cli.config.exists() {
        anyhow::bail!("{} already exists", cli.config.display());
    }
    std::fs::write(&cli.config, default)?;
    println!("Created {}", cli.config.display());
    Ok(())
}

fn cmd_code() -> anyhow::Result<()> {
    print!("{}", abathur::code::code_prompt());
    Ok(())
}
