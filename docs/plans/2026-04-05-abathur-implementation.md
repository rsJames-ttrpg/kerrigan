# Abathur Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a documentation indexing and generation system (library + CLI) that enables section-level documentation retrieval with blake3 staleness detection.

**Architecture:** Two crates — `src/abathur/` (library with config, frontmatter parsing, index, staleness, generation) and `src/abathur-cli/` (thin `abathur` binary wrapping the library with clap subcommands). A bare Claude Code skill at `.claude/skills/abathur/` teaches LLMs to generate docs using the CLI.

**Tech Stack:** Rust (edition 2024), serde + serde_yaml (frontmatter), blake3 (hashing), toml (config), clap (CLI), globset (path matching), chrono (dates), reqwest (Claude API), Buck2 + Cargo

---

## File Structure

### Library (`src/abathur/`)

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Crate manifest with deps |
| `BUCK` | Buck2 build targets (rust_library, rust_test) |
| `src/lib.rs` | Public module re-exports |
| `src/config.rs` | `AbathurConfig` parsed from `abathur.toml` |
| `src/frontmatter.rs` | YAML frontmatter parsing, `DocMeta` type, section extraction |
| `src/index.rs` | `Index` — build, query, query_by_tags, read |
| `src/staleness.rs` | blake3 file hashing, `Index::check`, `StaleDoc` |
| `src/hash.rs` | Update source hashes in frontmatter files on disk |
| `src/generator.rs` | `Generator` — Claude API doc generation |
| `src/code.rs` | `code_prompt()` — returns the schema/conventions prompt |

### CLI (`src/abathur-cli/`)

| File | Responsibility |
|------|---------------|
| `Cargo.toml` | Crate manifest, depends on `abathur` library |
| `BUCK` | Buck2 build targets (rust_binary, install_binary, rust_test) |
| `src/main.rs` | clap CLI struct, subcommand enum, dispatch, output formatting |

### Skill

| File | Responsibility |
|------|---------------|
| `.claude/skills/abathur/abathur.md` | Skill instructions for Claude Code |

### Config

| File | Responsibility |
|------|---------------|
| `abathur.toml` | Project-level abathur configuration |

---

### Task 1: Scaffold library crate

**Files:**
- Create: `src/abathur/Cargo.toml`
- Create: `src/abathur/BUCK`
- Create: `src/abathur/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "abathur"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
blake3 = "1"
toml = "0.8"
globset = "0.4"
chrono = { version = "0.4", features = ["serde"] }
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
anyhow = "1"
```

- [ ] **Step 2: Create lib.rs with module stubs**

```rust
pub mod config;
pub mod frontmatter;
pub mod index;
pub mod staleness;
pub mod hash;
pub mod generator;
pub mod code;
```

Create empty stub files for each module so the crate compiles:

`src/abathur/src/config.rs`:
```rust
```

`src/abathur/src/frontmatter.rs`:
```rust
```

`src/abathur/src/index.rs`:
```rust
```

`src/abathur/src/staleness.rs`:
```rust
```

`src/abathur/src/hash.rs`:
```rust
```

`src/abathur/src/generator.rs`:
```rust
```

`src/abathur/src/code.rs`:
```rust
```

- [ ] **Step 3: Add to workspace**

In root `Cargo.toml`, add `"src/abathur"` to the workspace members list.

- [ ] **Step 4: Create BUCK file**

```python
ABATHUR_SRCS = glob(["src/**/*.rs"])

ABATHUR_DEPS = [
    "//third-party:anyhow",
    "//third-party:blake3",
    "//third-party:chrono",
    "//third-party:globset",
    "//third-party:reqwest",
    "//third-party:serde",
    "//third-party:serde_yaml",
    "//third-party:toml",
]

rust_library(
    name = "abathur",
    srcs = ABATHUR_SRCS,
    crate_root = "src/lib.rs",
    deps = ABATHUR_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "abathur-test",
    srcs = ABATHUR_SRCS,
    crate_root = "src/lib.rs",
    deps = ABATHUR_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 5: Run buckify and verify build**

```bash
./tools/buckify.sh
cargo check -p abathur
```

Expected: clean build, no errors.

- [ ] **Step 6: Commit**

```bash
git add src/abathur/ Cargo.toml Cargo.lock
git commit -m "feat(abathur): scaffold library crate with module stubs"
```

---

### Task 2: Config parsing

**Files:**
- Create: `src/abathur/src/config.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write failing test for config parsing**

In `src/abathur/src/config.rs`:

```rust
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AbathurConfig {
    pub index: IndexConfig,
    pub sources: SourcesConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
}

#[derive(Debug, Deserialize)]
pub struct IndexConfig {
    pub doc_paths: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SourcesConfig {
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GenerateConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_api_key_env")]
    pub api_key_env: String,
}

fn default_model() -> String {
    "claude-sonnet-4-6".to_string()
}

fn default_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

impl AbathurConfig {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = r#"
[index]
doc_paths = ["docs/abathur"]
exclude = ["drafts/**"]

[sources]
roots = ["src/"]
exclude = ["**/target/**"]

[generate]
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
"#;
        let config: AbathurConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.index.doc_paths, vec![PathBuf::from("docs/abathur")]);
        assert_eq!(config.sources.roots, vec![PathBuf::from("src/")]);
        assert_eq!(config.generate.model, "claude-sonnet-4-6");
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[index]
doc_paths = ["docs/abathur"]

[sources]
roots = ["src/"]
"#;
        let config: AbathurConfig = toml::from_str(toml).unwrap();
        assert!(config.index.exclude.is_empty());
        assert_eq!(config.generate.model, "claude-sonnet-4-6");
        assert_eq!(config.generate.api_key_env, "ANTHROPIC_API_KEY");
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p abathur
```

Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/abathur/src/config.rs
git commit -m "feat(abathur): config parsing from abathur.toml"
```

---

### Task 3: Frontmatter parsing and section extraction

**Files:**
- Create: `src/abathur/src/frontmatter.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write failing tests**

In `src/abathur/src/frontmatter.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_DOC: &str = r#"---
title: Overseer API
slug: overseer-api
description: HTTP API layer
lastmod: 2026-04-05
tags: [overseer, api]
sources:
  - path: src/overseer/src/api/mod.rs
    hash: a1b2c3d4
sections: [summary, endpoints]
---

# Overseer API

## Summary

This is the summary section.
It has multiple lines.

## Endpoints

GET /api/jobs
POST /api/jobs
"#;

    #[test]
    fn parse_frontmatter() {
        let meta = parse(SAMPLE_DOC).unwrap();
        assert_eq!(meta.title, "Overseer API");
        assert_eq!(meta.slug, "overseer-api");
        assert_eq!(meta.tags, vec!["overseer", "api"]);
        assert_eq!(meta.sources.len(), 1);
        assert_eq!(meta.sources[0].hash, "a1b2c3d4");
        assert_eq!(meta.sections, vec!["summary", "endpoints"]);
    }

    #[test]
    fn extract_section_by_name() {
        let content = extract_section(SAMPLE_DOC, "summary").unwrap();
        assert!(content.contains("This is the summary section."));
        assert!(!content.contains("GET /api/jobs"));
    }

    #[test]
    fn extract_last_section() {
        let content = extract_section(SAMPLE_DOC, "endpoints").unwrap();
        assert!(content.contains("GET /api/jobs"));
        assert!(!content.contains("This is the summary section."));
    }

    #[test]
    fn extract_nonexistent_section() {
        let result = extract_section(SAMPLE_DOC, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn strip_frontmatter_returns_body() {
        let body = strip_frontmatter(SAMPLE_DOC);
        assert!(!body.contains("slug:"));
        assert!(body.starts_with("# Overseer API"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p abathur
```

Expected: compilation errors (types and functions don't exist yet).

- [ ] **Step 3: Implement frontmatter types and parsing**

Add implementation above the tests in `src/abathur/src/frontmatter.rs`:

```rust
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DocMeta {
    pub title: String,
    pub slug: String,
    pub description: String,
    pub lastmod: NaiveDate,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub sections: Vec<String>,

    /// Set after parsing — path to the file on disk. Not deserialized from YAML.
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub hash: String,
}

/// Parse YAML frontmatter from a markdown document into DocMeta.
pub fn parse(content: &str) -> anyhow::Result<DocMeta> {
    let yaml = extract_frontmatter_raw(content)
        .ok_or_else(|| anyhow::anyhow!("no YAML frontmatter found"))?;
    let meta: DocMeta = serde_yaml::from_str(yaml)?;
    Ok(meta)
}

/// Extract a named section from a markdown document.
/// Finds the `## <name>` heading (case-insensitive slug match) and returns
/// content from that heading to the next `## ` heading or EOF.
pub fn extract_section(content: &str, section_name: &str) -> anyhow::Result<String> {
    let body = strip_frontmatter(content);
    let target = normalize_section_name(section_name);

    let lines: Vec<&str> = body.lines().collect();
    let mut start = None;
    let mut end = None;

    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("## ") {
            let heading = normalize_section_name(&line[3..]);
            if heading == target {
                start = Some(i + 1); // skip the heading line itself
            } else if start.is_some() && end.is_none() {
                end = Some(i);
            }
        }
    }

    let start = start.ok_or_else(|| anyhow::anyhow!("section '{section_name}' not found"))?;
    let end = end.unwrap_or(lines.len());

    let section: String = lines[start..end]
        .join("\n")
        .trim()
        .to_string();

    Ok(section)
}

/// Strip YAML frontmatter, return the markdown body.
pub fn strip_frontmatter(content: &str) -> &str {
    let Some(start) = content.find("---") else {
        return content;
    };
    let after_first = &content[start + 3..];
    let Some(end) = after_first.find("---") else {
        return content;
    };
    after_first[end + 3..].trim_start_matches('\n')
}

/// Extract raw YAML string between --- delimiters.
fn extract_frontmatter_raw(content: &str) -> Option<&str> {
    let content = content.trim_start();
    let rest = content.strip_prefix("---")?;
    let end = rest.find("---")?;
    Some(&rest[..end])
}

/// Normalize a section name for matching: lowercase, replace spaces/underscores with hyphens.
fn normalize_section_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace(' ', "-")
        .replace('_', "-")
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p abathur
```

Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/abathur/src/frontmatter.rs
git commit -m "feat(abathur): frontmatter parsing and section extraction"
```

---

### Task 4: Index — build and query

**Files:**
- Create: `src/abathur/src/index.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write failing tests**

In `src/abathur/src/index.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_doc(dir: &std::path::Path, name: &str, content: &str) {
        let path = dir.join(name);
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn sample_config(doc_dir: &std::path::Path) -> AbathurConfig {
        AbathurConfig {
            index: IndexConfig {
                doc_paths: vec![doc_dir.to_path_buf()],
                exclude: vec![],
            },
            sources: SourcesConfig {
                roots: vec![],
                exclude: vec![],
            },
            generate: GenerateConfig::default(),
        }
    }

    const DOC_A: &str = r#"---
title: Alpha Service
slug: alpha
description: The alpha component
lastmod: 2026-04-05
tags: [api, http]
sources: []
sections: [summary]
---

# Alpha Service

## Summary

Alpha does things.
"#;

    const DOC_B: &str = r#"---
title: Beta Database
slug: beta
description: Database layer for beta
lastmod: 2026-04-05
tags: [database, sql]
sources: []
sections: [summary, schema]
---

# Beta Database

## Summary

Beta stores things.

## Schema

CREATE TABLE beta (...);
"#;

    #[test]
    fn build_index_from_dir() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        write_doc(dir.path(), "beta.md", DOC_B);
        let config = sample_config(dir.path());

        let index = Index::build(&config).unwrap();
        assert_eq!(index.docs.len(), 2);
        assert!(index.docs.contains_key("alpha"));
        assert!(index.docs.contains_key("beta"));
    }

    #[test]
    fn query_by_text() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        write_doc(dir.path(), "beta.md", DOC_B);
        let config = sample_config(dir.path());
        let index = Index::build(&config).unwrap();

        let results = index.query("database");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "beta");
    }

    #[test]
    fn query_by_tags() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        write_doc(dir.path(), "beta.md", DOC_B);
        let config = sample_config(dir.path());
        let index = Index::build(&config).unwrap();

        let results = index.query_by_tags(&["api"]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "alpha");
    }

    #[test]
    fn read_full_doc() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        let config = sample_config(dir.path());
        let index = Index::build(&config).unwrap();

        let content = index.read("alpha", None).unwrap();
        assert!(content.contains("Alpha does things."));
    }

    #[test]
    fn read_specific_section() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "beta.md", DOC_B);
        let config = sample_config(dir.path());
        let index = Index::build(&config).unwrap();

        let content = index.read("beta", Some("schema")).unwrap();
        assert!(content.contains("CREATE TABLE"));
        assert!(!content.contains("Beta stores things."));
    }

    #[test]
    fn read_nonexistent_slug() {
        let dir = TempDir::new().unwrap();
        let config = sample_config(dir.path());
        let index = Index::build(&config).unwrap();

        assert!(index.read("nope", None).is_err());
    }

    #[test]
    fn build_skips_non_md_files() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        write_doc(dir.path(), "readme.txt", "not a doc");
        let config = sample_config(dir.path());

        let index = Index::build(&config).unwrap();
        assert_eq!(index.docs.len(), 1);
    }

    #[test]
    fn build_applies_exclude_globs() {
        let dir = TempDir::new().unwrap();
        let drafts = dir.path().join("drafts");
        std::fs::create_dir(&drafts).unwrap();
        write_doc(dir.path(), "alpha.md", DOC_A);
        write_doc(&drafts, "wip.md", DOC_B);

        let config = AbathurConfig {
            index: IndexConfig {
                doc_paths: vec![dir.path().to_path_buf()],
                exclude: vec!["drafts/**".to_string()],
            },
            sources: SourcesConfig { roots: vec![], exclude: vec![] },
            generate: GenerateConfig::default(),
        };

        let index = Index::build(&config).unwrap();
        assert_eq!(index.docs.len(), 1);
        assert_eq!(index.docs.values().next().unwrap().slug, "alpha");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p abathur
```

Expected: compilation errors.

- [ ] **Step 3: Implement Index**

```rust
use std::collections::HashMap;
use std::path::Path;

use globset::{Glob, GlobSetBuilder};

use crate::config::{AbathurConfig, IndexConfig};
use crate::frontmatter::{self, DocMeta};

pub struct Index {
    pub(crate) docs: HashMap<String, DocMeta>,
}

impl Index {
    /// Scan all doc_paths, parse frontmatter, build slug-keyed index.
    pub fn build(config: &AbathurConfig) -> anyhow::Result<Self> {
        let exclude_set = build_glob_set(&config.index.exclude)?;
        let mut docs = HashMap::new();

        for doc_path in &config.index.doc_paths {
            if !doc_path.exists() {
                continue;
            }
            collect_docs(doc_path, doc_path, &exclude_set, &mut docs)?;
        }

        Ok(Self { docs })
    }

    /// Search by case-insensitive text match against title, description, and slug.
    pub fn query(&self, terms: &str) -> Vec<&DocMeta> {
        let lower = terms.to_lowercase();
        self.docs
            .values()
            .filter(|d| {
                d.title.to_lowercase().contains(&lower)
                    || d.description.to_lowercase().contains(&lower)
                    || d.slug.to_lowercase().contains(&lower)
            })
            .collect()
    }

    /// Filter docs that have any of the given tags.
    pub fn query_by_tags(&self, tags: &[&str]) -> Vec<&DocMeta> {
        self.docs
            .values()
            .filter(|d| tags.iter().any(|t| d.tags.iter().any(|dt| dt == t)))
            .collect()
    }

    /// Read a doc by slug. If section is Some, extract only that section.
    pub fn read(&self, slug: &str, section: Option<&str>) -> anyhow::Result<String> {
        let meta = self
            .docs
            .get(slug)
            .ok_or_else(|| anyhow::anyhow!("document '{slug}' not found"))?;

        let content = std::fs::read_to_string(&meta.path)?;

        match section {
            Some(name) => frontmatter::extract_section(&content, name),
            None => Ok(frontmatter::strip_frontmatter(&content).to_string()),
        }
    }
}

fn build_glob_set(patterns: &[String]) -> anyhow::Result<globset::GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

fn collect_docs(
    base: &Path,
    dir: &Path,
    exclude: &globset::GlobSet,
    docs: &mut HashMap<String, DocMeta>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(base).unwrap_or(&path);

        if exclude.is_match(rel) {
            continue;
        }

        if path.is_dir() {
            collect_docs(base, &path, exclude, docs)?;
            continue;
        }

        if path.extension().is_some_and(|e| e == "md") {
            let content = std::fs::read_to_string(&path)?;
            if let Ok(mut meta) = frontmatter::parse(&content) {
                meta.path = path;
                docs.insert(meta.slug.clone(), meta);
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p abathur
```

Expected: all 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/abathur/src/index.rs
git commit -m "feat(abathur): index build, query, and read operations"
```

---

### Task 5: Staleness detection

**Files:**
- Create: `src/abathur/src/staleness.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write failing tests**

In `src/abathur/src/staleness.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::index::Index;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn hash_file_returns_blake3_hex() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, b"fn main() {}").unwrap();

        let hash = hash_file(&path).unwrap();
        let expected = blake3::hash(b"fn main() {}").to_hex().to_string();
        assert_eq!(hash, expected);
    }

    #[test]
    fn check_detects_stale_doc() {
        let dir = TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir(&src_dir).unwrap();
        let source_file = src_dir.join("lib.rs");
        std::fs::write(&source_file, b"fn original() {}").unwrap();

        // Hash the original content
        let original_hash = hash_file(&source_file).unwrap();

        // Create a doc referencing the source with original hash
        let doc_dir = dir.path().join("docs");
        std::fs::create_dir(&doc_dir).unwrap();
        let doc = format!(
            r#"---
title: Test
slug: test-doc
description: A test
lastmod: 2026-04-05
tags: []
sources:
  - path: {}
    hash: {}
sections: [summary]
---

# Test

## Summary

Test content.
"#,
            source_file.display(),
            original_hash,
        );
        std::fs::write(doc_dir.join("test.md"), &doc).unwrap();

        let config = AbathurConfig {
            index: IndexConfig {
                doc_paths: vec![doc_dir],
                exclude: vec![],
            },
            sources: SourcesConfig { roots: vec![src_dir.clone()], exclude: vec![] },
            generate: GenerateConfig::default(),
        };

        // Not stale yet
        let index = Index::build(&config).unwrap();
        let stale = check(&index).unwrap();
        assert!(stale.is_empty());

        // Modify source file
        std::fs::write(&source_file, b"fn modified() {}").unwrap();

        let stale = check(&index).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].slug, "test-doc");
        assert_eq!(stale[0].changed_sources.len(), 1);
    }

    #[test]
    fn check_handles_missing_source_file() {
        let dir = TempDir::new().unwrap();
        let doc_dir = dir.path().join("docs");
        std::fs::create_dir(&doc_dir).unwrap();

        let doc = r#"---
title: Test
slug: test-doc
description: A test
lastmod: 2026-04-05
tags: []
sources:
  - path: /nonexistent/file.rs
    hash: abc123
sections: []
---

# Test
"#;
        std::fs::write(doc_dir.join("test.md"), doc).unwrap();

        let config = AbathurConfig {
            index: IndexConfig {
                doc_paths: vec![doc_dir],
                exclude: vec![],
            },
            sources: SourcesConfig { roots: vec![], exclude: vec![] },
            generate: GenerateConfig::default(),
        };

        let index = Index::build(&config).unwrap();
        let stale = check(&index).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].slug, "test-doc");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p abathur
```

Expected: compilation errors.

- [ ] **Step 3: Implement staleness detection**

```rust
use std::path::Path;
use std::path::PathBuf;

use crate::index::Index;

pub struct StaleDoc {
    pub slug: String,
    pub changed_sources: Vec<PathBuf>,
}

/// Hash a file's contents with blake3, return hex string.
pub fn hash_file(path: &Path) -> anyhow::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&file)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Check all docs in the index for staleness.
/// A doc is stale if any of its source files have a different blake3 hash
/// than what's recorded in the frontmatter, or if a source file is missing.
pub fn check(index: &Index) -> anyhow::Result<Vec<StaleDoc>> {
    let mut stale = Vec::new();

    for meta in index.docs.values() {
        let mut changed = Vec::new();

        for source in &meta.sources {
            let current_hash = match hash_file(&source.path) {
                Ok(h) => h,
                Err(_) => {
                    // Missing file counts as changed
                    changed.push(source.path.clone());
                    continue;
                }
            };

            if current_hash != source.hash {
                changed.push(source.path.clone());
            }
        }

        if !changed.is_empty() {
            stale.push(StaleDoc {
                slug: meta.slug.clone(),
                changed_sources: changed,
            });
        }
    }

    Ok(stale)
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p abathur
```

Expected: all 3 staleness tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/abathur/src/staleness.rs
git commit -m "feat(abathur): blake3 staleness detection for source files"
```

---

### Task 6: Hash update command

**Files:**
- Create: `src/abathur/src/hash.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write failing test**

In `src/abathur/src/hash.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn update_hashes_in_frontmatter() {
        let dir = TempDir::new().unwrap();

        // Create a source file
        let source = dir.path().join("lib.rs");
        std::fs::write(&source, b"fn hello() {}").unwrap();
        let real_hash = crate::staleness::hash_file(&source).unwrap();

        // Create a doc with a stale hash
        let doc_path = dir.path().join("doc.md");
        let doc = format!(
            r#"---
title: Test
slug: test
description: A test
lastmod: 2026-04-05
tags: []
sources:
  - path: {}
    hash: stale_hash_value
sections: []
---

# Test

Content here.
"#,
            source.display()
        );
        std::fs::write(&doc_path, &doc).unwrap();

        update_hashes(&doc_path).unwrap();

        let updated = std::fs::read_to_string(&doc_path).unwrap();
        assert!(updated.contains(&real_hash));
        assert!(!updated.contains("stale_hash_value"));
        // Body preserved
        assert!(updated.contains("Content here."));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p abathur
```

Expected: compilation error.

- [ ] **Step 3: Implement hash update**

```rust
use std::path::Path;

use crate::frontmatter;
use crate::staleness;

/// Read a doc file, recompute blake3 hashes for all sources, rewrite the file.
pub fn update_hashes(doc_path: &Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(doc_path)?;
    let mut meta = frontmatter::parse(&content)?;
    let body = frontmatter::strip_frontmatter(&content);

    for source in &mut meta.sources {
        match staleness::hash_file(&source.path) {
            Ok(hash) => source.hash = hash,
            Err(e) => {
                anyhow::bail!(
                    "cannot hash source '{}': {e}",
                    source.path.display()
                );
            }
        }
    }

    // Update lastmod to today
    meta.lastmod = chrono::Local::now().date_naive();

    let yaml = serde_yaml::to_string(&frontmatter::to_serializable(&meta))?;
    let output = format!("---\n{yaml}---\n\n{body}");
    std::fs::write(doc_path, output)?;

    Ok(())
}
```

This requires a serializable version of DocMeta. Add to `src/abathur/src/frontmatter.rs`:

```rust
use serde::Serialize;

/// Serializable view of DocMeta for writing back to YAML frontmatter.
#[derive(Serialize)]
pub struct DocMetaSerializable {
    pub title: String,
    pub slug: String,
    pub description: String,
    pub lastmod: NaiveDate,
    pub tags: Vec<String>,
    pub sources: Vec<Source>,
    pub sections: Vec<String>,
}

pub fn to_serializable(meta: &DocMeta) -> DocMetaSerializable {
    DocMetaSerializable {
        title: meta.title.clone(),
        slug: meta.slug.clone(),
        description: meta.description.clone(),
        lastmod: meta.lastmod,
        tags: meta.tags.clone(),
        sources: meta.sources.clone(),
        sections: meta.sections.clone(),
    }
}
```

Also add `Serialize` derive to `Source`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub path: PathBuf,
    pub hash: String,
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p abathur
```

Expected: hash update test passes.

- [ ] **Step 5: Commit**

```bash
git add src/abathur/src/hash.rs src/abathur/src/frontmatter.rs
git commit -m "feat(abathur): hash update rewrites source hashes in frontmatter"
```

---

### Task 7: Code prompt dump

**Files:**
- Create: `src/abathur/src/code.rs`
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Write test and implementation**

In `src/abathur/src/code.rs`:

```rust
/// Return the prompt text describing the abathur document schema and conventions.
/// Used by `abathur code` to give an LLM the context needed to write docs.
pub fn code_prompt() -> &'static str {
    r#"# Abathur Document Schema

You are generating documentation for the abathur indexing system. Each document is a
markdown file with YAML frontmatter.

## Frontmatter Format

```yaml
---
title: <Human-readable title>
slug: <unique-identifier-kebab-case>
description: <One-line summary for index listings>
lastmod: <YYYY-MM-DD>
tags: [<tag1>, <tag2>, ...]
sources:
  - path: <relative/path/to/source.rs>
    hash: <leave empty - will be computed by `abathur hash`>
sections: [<section1>, <section2>, ...]
---
```

## Rules

1. **slug** must be unique across all abathur docs. Use kebab-case.
2. **tags** should be lowercase, meaningful categories (e.g. `api`, `database`, `config`).
3. **sources** lists every source file this doc describes. Set `hash: ""` — run `abathur hash <file>` after writing to populate.
4. **sections** lists the `## Heading` names in the markdown body (kebab-case).
5. Each `## Heading` in the body must have a corresponding entry in `sections`.
6. Keep sections focused — each should be independently useful when loaded in isolation.
7. Start the body with a `# Title` heading matching the frontmatter `title`.
8. Write for an LLM audience: be precise, include types, signatures, and examples.

## After Writing

Run these commands to finalize:
```bash
abathur hash <path-to-doc.md>   # compute source hashes
abathur check                    # verify no staleness
```
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_prompt_contains_schema() {
        let prompt = code_prompt();
        assert!(prompt.contains("slug"));
        assert!(prompt.contains("sources"));
        assert!(prompt.contains("sections"));
        assert!(prompt.contains("abathur hash"));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p abathur
```

Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add src/abathur/src/code.rs
git commit -m "feat(abathur): code prompt for LLM-driven doc generation"
```

---

### Task 8: Scaffold CLI crate

**Files:**
- Create: `src/abathur-cli/Cargo.toml`
- Create: `src/abathur-cli/BUCK`
- Create: `src/abathur-cli/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "abathur-cli"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "abathur"
path = "src/main.rs"

[dependencies]
abathur = { path = "../abathur" }
clap = { version = "4", features = ["derive", "env"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
anyhow = "1"
```

- [ ] **Step 2: Create main.rs with CLI structure**

```rust
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

    match cli.command {
        Command::Query { terms, tag } => cmd_query(&cli, &terms, tag),
        Command::Read { slug, section } => cmd_read(&cli, &slug, section.as_deref()),
        Command::Check => cmd_check(&cli),
        Command::Generate { path, stale } => cmd_generate(&cli, path.as_deref(), stale),
        Command::Hash { doc, all } => cmd_hash(&cli, doc.as_deref(), all),
        Command::Init => cmd_init(&cli),
        Command::Code => cmd_code(&cli),
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
                .map(|d| serde_json::json!({
                    "slug": d.slug,
                    "title": d.title,
                    "description": d.description,
                    "tags": d.tags,
                }))
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
            println!("{}", serde_json::json!({ "slug": slug, "content": content }));
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
                .map(|s| serde_json::json!({
                    "slug": s.slug,
                    "changed_sources": s.changed_sources,
                }))
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
    Ok(())
}

fn cmd_generate(_cli: &Cli, _path: Option<&std::path::Path>, _stale: bool) -> anyhow::Result<()> {
    anyhow::bail!("generate command not yet implemented (requires Claude API integration)")
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

fn cmd_init(_cli: &Cli) -> anyhow::Result<()> {
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

    if std::path::Path::new("abathur.toml").exists() {
        anyhow::bail!("abathur.toml already exists");
    }
    std::fs::write("abathur.toml", default)?;
    println!("Created abathur.toml");
    Ok(())
}

fn cmd_code(_cli: &Cli) -> anyhow::Result<()> {
    print!("{}", abathur::code::code_prompt());
    Ok(())
}
```

- [ ] **Step 3: Add to workspace**

Add `"src/abathur-cli"` to workspace members in root `Cargo.toml`.

- [ ] **Step 4: Create BUCK file**

```python
load("//rules:install.bzl", "install_binary")

ABATHUR_CLI_SRCS = glob(["src/**/*.rs"])

ABATHUR_CLI_DEPS = [
    "//src/abathur:abathur",
    "//third-party:anyhow",
    "//third-party:clap",
    "//third-party:serde_json",
    "//third-party:tokio",
]

rust_binary(
    name = "abathur-cli",
    srcs = ABATHUR_CLI_SRCS,
    crate_root = "src/main.rs",
    deps = ABATHUR_CLI_DEPS,
    visibility = ["PUBLIC"],
)

install_binary(
    name = "install",
    binary = ":abathur-cli",
)
```

- [ ] **Step 5: Run buckify and verify build**

```bash
./tools/buckify.sh
cargo check -p abathur-cli
```

Expected: clean build.

- [ ] **Step 6: Commit**

```bash
git add src/abathur-cli/ Cargo.toml Cargo.lock
git commit -m "feat(abathur): CLI crate with query, read, check, hash, init, code commands"
```

---

### Task 9: Create abathur.toml and skill

**Files:**
- Create: `abathur.toml`
- Create: `.claude/skills/abathur/abathur.md`

- [ ] **Step 1: Create abathur.toml**

```toml
[index]
doc_paths = ["docs/abathur"]
exclude = []

[sources]
roots = ["src/"]
exclude = ["**/target/**", "**/proto_gen/**"]

[generate]
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
```

- [ ] **Step 2: Create docs/abathur directory**

```bash
mkdir -p docs/abathur
```

- [ ] **Step 3: Create the skill file**

Create `.claude/skills/abathur/abathur.md`:

```markdown
---
name: abathur
description: Generate and maintain indexed abathur documentation for the project
---

# Abathur Documentation Skill

Use this skill to generate, update, or check abathur documentation.

## Prerequisites

The `abathur` CLI must be available. Build it with:
```bash
buck2 build root//src/abathur-cli:abathur-cli
```

## Workflow: Check for stale docs

```bash
abathur check
```

If stale docs are found, follow the "Update existing doc" workflow below.

## Workflow: Create a new doc

1. Get the schema:
```bash
abathur code
```

2. Read the source files you want to document.

3. Write a new `.md` file in `docs/abathur/` following the schema from step 1.
   - Set `hash: ""` for all sources — the CLI will compute them.
   - List all `## Heading` names in the `sections` field.

4. Update hashes:
```bash
abathur hash docs/abathur/<new-doc>.md
```

5. Verify:
```bash
abathur check
```

## Workflow: Update existing doc

1. Identify what changed:
```bash
abathur check --json
```

2. Read the changed source files to understand the updates.

3. Read the current doc:
```bash
abathur read <slug>
```

4. Edit the doc file to reflect the source changes.

5. Update hashes:
```bash
abathur hash docs/abathur/<doc>.md
```

6. Verify:
```bash
abathur check
```

## Querying docs

```bash
abathur query "search terms"        # text search
abathur query "api" --tag            # tag search
abathur read <slug>                  # full doc
abathur read <slug> --section <name> # specific section
```

Add `--format json` to any command for structured output.
```

- [ ] **Step 4: Commit**

```bash
git add abathur.toml docs/abathur/.gitkeep .claude/skills/abathur/
git commit -m "feat(abathur): add config, docs directory, and Claude Code skill"
```

Create the `.gitkeep`:
```bash
touch docs/abathur/.gitkeep
```

---

### Task 10: Integration test — end-to-end workflow

**Files:**
- Test: `src/abathur/tests/integration.rs`

- [ ] **Step 1: Write end-to-end integration test**

Create `src/abathur/tests/integration.rs`:

```rust
use std::io::Write;
use std::path::PathBuf;

use tempfile::TempDir;

use abathur::config::*;
use abathur::index::Index;
use abathur::staleness;

#[test]
fn full_workflow_create_check_update() {
    let dir = TempDir::new().unwrap();

    // 1. Create source files
    let src_dir = dir.path().join("src");
    std::fs::create_dir(&src_dir).unwrap();
    let source = src_dir.join("api.rs");
    std::fs::write(&source, b"fn handle_request() -> Response { ok() }").unwrap();

    let original_hash = abathur::staleness::hash_file(&source).unwrap();

    // 2. Create doc with correct hash
    let doc_dir = dir.path().join("docs");
    std::fs::create_dir(&doc_dir).unwrap();
    let doc_content = format!(
        r#"---
title: API Handler
slug: api-handler
description: Request handling for the API
lastmod: 2026-04-05
tags: [api, http]
sources:
  - path: {source}
    hash: {original_hash}
sections: [summary, endpoints]
---

# API Handler

## Summary

Handles incoming HTTP requests.

## Endpoints

GET /api/status
"#,
        source = source.display(),
        original_hash = original_hash,
    );
    let doc_path = doc_dir.join("api-handler.md");
    std::fs::write(&doc_path, &doc_content).unwrap();

    let config = AbathurConfig {
        index: IndexConfig {
            doc_paths: vec![doc_dir.clone()],
            exclude: vec![],
        },
        sources: SourcesConfig {
            roots: vec![src_dir.clone()],
            exclude: vec![],
        },
        generate: GenerateConfig::default(),
    };

    // 3. Build index, verify not stale
    let index = Index::build(&config).unwrap();
    assert_eq!(index.docs.len(), 1);
    let stale = staleness::check(&index).unwrap();
    assert!(stale.is_empty(), "should not be stale initially");

    // 4. Query works
    let results = index.query("api");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slug, "api-handler");

    let results = index.query_by_tags(&["http"]);
    assert_eq!(results.len(), 1);

    // 5. Read full doc
    let content = index.read("api-handler", None).unwrap();
    assert!(content.contains("Handles incoming HTTP requests."));
    assert!(content.contains("GET /api/status"));

    // 6. Read specific section
    let section = index.read("api-handler", Some("endpoints")).unwrap();
    assert!(section.contains("GET /api/status"));
    assert!(!section.contains("Handles incoming HTTP requests."));

    // 7. Modify source → becomes stale
    std::fs::write(&source, b"fn handle_request() -> Response { ok_with_body() }").unwrap();
    let stale = staleness::check(&index).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].slug, "api-handler");

    // 8. Update hashes → no longer stale
    abathur::hash::update_hashes(&doc_path).unwrap();
    let index = Index::build(&config).unwrap();
    let stale = staleness::check(&index).unwrap();
    assert!(stale.is_empty(), "should not be stale after hash update");
}
```

- [ ] **Step 2: Run the integration test**

```bash
cargo test -p abathur --test integration
```

Expected: passes.

- [ ] **Step 3: Run all tests**

```bash
cargo test -p abathur
```

Expected: all unit tests + integration test pass.

- [ ] **Step 4: Commit**

```bash
git add src/abathur/tests/
git commit -m "test(abathur): end-to-end integration test for full workflow"
```

---

### Task 11: Generator stub (API mode)

**Files:**
- Create: `src/abathur/src/generator.rs`

- [ ] **Step 1: Implement generator with TODO for API call**

The generator needs the Claude API client, which will come from the `runtime` crate once it exists. For now, implement the structure with the API call as the only unimplemented part, using `reqwest` directly.

In `src/abathur/src/generator.rs`:

```rust
use std::path::Path;

use crate::config::AbathurConfig;
use crate::index::Index;
use crate::staleness;

pub struct Generator {
    config: AbathurConfig,
}

impl Generator {
    pub fn new(config: AbathurConfig) -> Self {
        Self { config }
    }

    /// Generate an abathur doc for the given source path by calling the Claude API.
    /// Reads the source files, sends them as context with the schema prompt,
    /// and returns the generated markdown document.
    pub async fn generate(&self, source_path: &Path) -> anyhow::Result<String> {
        let source_content = std::fs::read_to_string(source_path)?;
        let schema = crate::code::code_prompt();

        let api_key = std::env::var(&self.config.generate.api_key_env)
            .map_err(|_| anyhow::anyhow!(
                "environment variable '{}' not set",
                self.config.generate.api_key_env
            ))?;

        let prompt = format!(
            "{schema}\n\n---\n\n\
             Generate an abathur document for the following source file.\n\
             Source path: {path}\n\n\
             ```rust\n{source_content}\n```",
            schema = schema,
            path = source_path.display(),
            source_content = source_content,
        );

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": self.config.generate.model,
                "max_tokens": 4096,
                "messages": [{
                    "role": "user",
                    "content": prompt,
                }]
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Claude API error {status}: {body}");
        }

        let body: serde_json::Value = response.json().await?;
        let text = body["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("unexpected API response format"))?;

        Ok(text.to_string())
    }

    /// Regenerate all stale docs.
    pub async fn regenerate_stale(&self, index: &Index) -> anyhow::Result<Vec<String>> {
        let stale = staleness::check(index)?;
        let mut updated = Vec::new();

        for stale_doc in &stale {
            let meta = index.docs.get(&stale_doc.slug)
                .ok_or_else(|| anyhow::anyhow!("stale doc '{}' not in index", stale_doc.slug))?;

            // Regenerate from first source file
            if let Some(first_source) = meta.sources.first() {
                let content = self.generate(&first_source.path).await?;
                std::fs::write(&meta.path, &content)?;
                crate::hash::update_hashes(&meta.path)?;
                updated.push(stale_doc.slug.clone());
            }
        }

        Ok(updated)
    }
}
```

Add `serde_json` to `src/abathur/Cargo.toml` dependencies:

```toml
serde_json = "1"
```

- [ ] **Step 2: Verify build**

```bash
cargo check -p abathur
```

Expected: compiles. No tests for the API call — that requires a live key.

- [ ] **Step 3: Update the CLI generate command**

In `src/abathur-cli/src/main.rs`, replace the `cmd_generate` stub:

```rust
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
```

Add `Clone` derive to `AbathurConfig` and its sub-structs in `src/abathur/src/config.rs`:

```rust
#[derive(Debug, Clone, Deserialize)]
```

- [ ] **Step 4: Verify build**

```bash
cargo check -p abathur-cli
```

Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src/abathur/src/generator.rs src/abathur/Cargo.toml src/abathur/src/config.rs src/abathur-cli/src/main.rs
git commit -m "feat(abathur): generator with Claude API integration"
```

---

### Task 12: Final verification and CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Run all tests**

```bash
cargo test -p abathur
cargo test -p abathur-cli
```

Expected: all pass.

- [ ] **Step 2: Run clippy**

```bash
cargo clippy -p abathur -p abathur-cli
```

Expected: no warnings.

- [ ] **Step 3: Run buckify and buck2 build**

```bash
./tools/buckify.sh
buck2 build root//src/abathur:abathur root//src/abathur-cli:abathur-cli
```

Expected: clean build.

- [ ] **Step 4: Add Abathur section to CLAUDE.md**

Add after the Creep CLI section in `CLAUDE.md`:

```markdown
### Abathur (`src/abathur/`)
Documentation indexing and generation library. Parses markdown files with YAML frontmatter, builds an in-memory index by slug, supports section-level reads, and detects staleness via blake3 source file hashing. Claude API integration for doc generation.

- **Build:** `buck2 build root//src/abathur:abathur`
- **Test:** `cd src/abathur && cargo test`

### Abathur CLI (`src/abathur-cli/`)
CLI tool for querying, reading, checking, and generating abathur documentation. Installed as `abathur` binary.

- **Build:** `buck2 build root//src/abathur-cli:abathur-cli`
- **Install:** `buck2 run root//src/abathur-cli:install`
- **Usage:** `abathur query "api"`, `abathur read <slug> --section <name>`, `abathur check`, `abathur code`
- **Config:** `abathur.toml`
```

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add Abathur to CLAUDE.md components section"
```

Plan complete and saved to `docs/plans/2026-04-05-abathur-implementation.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
