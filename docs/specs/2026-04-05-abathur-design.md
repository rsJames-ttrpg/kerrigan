# Abathur: Documentation Indexing & Generation

**Date:** 2026-04-05
**Status:** Draft

## Problem

The native drone needs a way to inject relevant project documentation into its conversation context without loading everything. Current CLAUDE.md files are monolithic — ~440 lines loaded in full regardless of what the agent is actually working on. There's no way to:

1. **Segment documentation** — load only the sections relevant to the current task/stage
2. **Detect staleness** — know when docs are out of date with the source code they describe
3. **Generate docs** — produce indexed documentation from source code using LLM capabilities
4. **Query docs** — search and retrieve specific documentation by topic, tag, or section

## Solution

Two crates forming a documentation indexing and generation system:

- **`src/abathur/`** — library. Frontmatter parsing, in-memory index, section extraction, blake3 staleness detection, Claude API-based generation.
- **`src/abathur-cli/`** — `abathur` binary. CLI for querying, reading, generating, and maintaining documentation. Includes a `--code` mode that dumps prompt + schema for LLM-driven generation within Claude Code sessions.

A bare skill (`.claude/skills/abathur/`) teaches Claude Code how to use the CLI for interactive doc generation and maintenance.

## Document Format

An abathur document is a markdown file with YAML frontmatter. The frontmatter provides metadata for indexing and section mappings for partial reads:

```markdown
---
title: Overseer API
slug: overseer-api
description: HTTP API layer for the Overseer service
lastmod: 2026-04-05
tags: [overseer, api, http]
sources:
  - path: src/overseer/src/api/mod.rs
    hash: a1b2c3d4
  - path: src/overseer/src/api/routes.rs
    hash: e5f6a7b8
sections: [summary, endpoints, error-handling, examples]
---

# Overseer API

## Summary
...

## Endpoints
...
```

### Frontmatter Fields

| Field | Type | Description |
|-------|------|-------------|
| `title` | string | Human-readable document title |
| `slug` | string | Unique identifier for queries (`abathur read <slug>`) |
| `description` | string | One-line summary for index listings |
| `lastmod` | date | When the doc was last generated or updated |
| `tags` | string[] | Categories for filtering (`abathur query --tag`) |
| `sources` | object[] | Source files this doc covers, each with `path` and blake3 `hash` |
| `sections` | string[] | Named sections present in the document |

### Section Mapping

Section names in the frontmatter correspond to `## Heading` lines in the markdown body. The library resolves names to line offsets at read time by scanning for matching headings. A section runs from its heading to the next heading (or EOF). No line numbers stored in frontmatter — they're computed on the fly.

## Configuration (`abathur.toml`)

Lives at project root. Defines where to find docs, what to exclude, and generation settings:

```toml
[index]
doc_paths = ["docs/abathur"]
exclude = ["drafts/**", "*.bak"]

[sources]
roots = ["src/"]
exclude = ["**/proto_gen/**", "**/target/**"]

[generate]
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
```

| Section | Purpose |
|---------|---------|
| `[index]` | Where abathur docs live and glob patterns to skip |
| `[sources]` | Source code roots for staleness checks and generation |
| `[generate]` | Model and auth for API-mode generation |

## Library (`src/abathur/`)

### Public API

```rust
// Config — parsed from abathur.toml
pub struct AbathurConfig {
    pub index: IndexConfig,
    pub sources: SourcesConfig,
    pub generate: GenerateConfig,
}

// Document metadata — parsed from frontmatter
pub struct DocMeta {
    pub title: String,
    pub slug: String,
    pub description: String,
    pub lastmod: NaiveDate,
    pub tags: Vec<String>,
    pub sources: Vec<Source>,
    pub sections: Vec<String>,
    pub path: PathBuf,
}

pub struct Source {
    pub path: PathBuf,
    pub hash: String,
}

// In-memory index — built by scanning doc_paths
pub struct Index {
    docs: HashMap<String, DocMeta>, // slug → metadata
}

impl Index {
    /// Scan doc_paths, parse frontmatter, build index
    pub fn build(config: &AbathurConfig) -> Result<Self>;

    /// Search by text match against title/description/slug
    pub fn query(&self, terms: &str) -> Vec<&DocMeta>;

    /// Filter by tags
    pub fn query_by_tags(&self, tags: &[&str]) -> Vec<&DocMeta>;

    /// Read full doc or specific section by slug
    pub fn read(&self, slug: &str, section: Option<&str>) -> Result<String>;

    /// Compare source hashes, report stale docs
    pub fn check(&self) -> Result<Vec<StaleDoc>>;
}

pub struct StaleDoc {
    pub slug: String,
    pub changed_sources: Vec<PathBuf>,
}

// Generation — API mode (calls Claude directly)
pub struct Generator {
    config: AbathurConfig,
}

impl Generator {
    /// Generate doc for a source path via Claude API
    pub async fn generate(&self, source_path: &Path) -> Result<String>;

    /// Regenerate all stale docs
    pub async fn regenerate_stale(&self, index: &Index) -> Result<Vec<String>>;
}
```

### Key Implementation Details

- **No persistence layer** — the frontmatter IS the index. The library parses it into a queryable in-memory structure at runtime.
- **blake3 hashing** — standalone, no Creep dependency. Hashes source files to detect staleness.
- **Section extraction** — scans markdown body for `## Heading` lines matching section names, extracts content between headings. Computed at read time, not stored.
- **Frontmatter parsing** — YAML between `---` delimiters, serde deserialization into `DocMeta`.

## CLI (`abathur`)

```
abathur query <terms>                  # search by tag/text, returns summaries
abathur read <slug>                    # full doc content
abathur read <slug> --section <name>   # specific section only
abathur check                          # list stale docs (source hashes changed)
abathur generate <path>                # generate doc for source path via API
abathur generate --stale               # regenerate all stale docs via API
abathur hash <doc>                     # update source hashes in frontmatter
abathur hash --all                     # update all docs' source hashes
abathur init                           # create abathur.toml with defaults
abathur code                           # dump prompt with frontmatter schema for LLM use
```

### Output Modes

All commands support:
- `--md` (default) — markdown output for human/LLM readability
- `--json` — structured JSON for programmatic consumers (native drone, scripts)
- `--config <path>` — override `abathur.toml` location

### Deterministic vs LLM Operations

| Command | Mode | Description |
|---------|------|-------------|
| `query`, `read`, `check` | Deterministic | Index/read/compare, no LLM needed |
| `hash` | Deterministic | Recompute source hashes in frontmatter |
| `generate` | LLM (API) | Calls Claude API to produce doc content |
| `code` | Deterministic | Dumps prompt/schema for LLM-in-session use |
| `init` | Deterministic | Scaffold config file |

## Skill (`.claude/skills/abathur/`)

A bare markdown skill directory in the repo (not a plugin — no install step). Teaches Claude Code how to generate and maintain abathur docs using the calling LLM as the generator.

### Skill Workflow

1. Run `abathur code` → get frontmatter schema and conventions
2. Run `abathur check` → find stale docs (or identify sources needing new docs)
3. Read the relevant source files to understand the code
4. Write/update the markdown content with correct frontmatter
5. Run `abathur hash <doc>` → update source hashes in frontmatter
6. Run `abathur check` → verify hashes are current

The skill bypasses `abathur generate` entirely — the LLM in the Claude Code session IS the generator. It uses `abathur code` to get the schema/conventions, then the CLI handles only the deterministic operations (staleness detection, hash updates, section recomputation).

## Native Drone Integration (Future)

The native drone imports the `abathur` library directly:

1. Build `Index` at startup from `abathur.toml` config
2. During prompt construction, query the index based on current stage/task context (tags, relevant source paths)
3. Pull specific sections into the system prompt, respecting token budgets
4. The prompt builder uses section-level granularity to fit documentation within context limits

This replaces monolithic CLAUDE.md injection with targeted, relevant documentation loading.

## Crate Layout

```
src/abathur/
  Cargo.toml
  BUCK
  src/
    lib.rs          # pub mod re-exports
    config.rs       # AbathurConfig, parsed from abathur.toml
    frontmatter.rs  # YAML frontmatter parsing into DocMeta
    index.rs        # Index::build, query, query_by_tags, read
    staleness.rs    # blake3 hashing, Index::check, StaleDoc
    generator.rs    # Generator — Claude API doc generation

src/abathur-cli/
  Cargo.toml
  BUCK
  src/
    main.rs         # clap CLI, command dispatch
    commands/
      query.rs
      read.rs
      check.rs
      generate.rs
      hash.rs
      init.rs
      code.rs       # --code prompt dump

.claude/skills/abathur/
  abathur.md        # skill instructions for Claude Code
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `serde`, `serde_yaml` | Frontmatter deserialization |
| `blake3` | Source file content hashing |
| `toml` | `abathur.toml` config parsing |
| `clap` | CLI argument parsing |
| `glob` / `globset` | Path matching for excludes |
| `chrono` | Date handling in frontmatter |
| `anthropic` (or raw HTTP) | API-mode generation (Generator only) |

## Related Specs

- [Native Drone Overview](native-drone/00-overview.md) — the primary consumer; abathur library feeds into the native drone's `SystemPromptBuilder`
- [Native Drone Config & Prompts](native-drone/05-drone-config-and-prompts.md) — prompt construction that will integrate abathur queries
