---
title: Abathur Documentation Indexer
slug: abathur
description: Library for markdown doc indexing with YAML frontmatter, staleness detection, and AI generation
lastmod: 2026-04-05
tags: [abathur, documentation, indexing, generation]
sources:
  - path: src/abathur/src/config.rs
    hash: ""
  - path: src/abathur/src/frontmatter.rs
    hash: ""
  - path: src/abathur/src/index.rs
    hash: ""
  - path: src/abathur/src/hash.rs
    hash: ""
  - path: src/abathur/src/staleness.rs
    hash: ""
  - path: src/abathur/src/generator.rs
    hash: ""
  - path: src/abathur/src/code.rs
    hash: ""
sections: [config, frontmatter, index, staleness, hash-management, generator]
---

# Abathur Documentation Indexer

## Config

```rust
pub struct AbathurConfig {
    pub index: IndexConfig,      // doc_paths, exclude globs
    pub sources: SourcesConfig,  // roots, exclude globs
    pub generate: GenerateConfig, // model, api_key_env
}
```

Loaded from `abathur.toml` via `AbathurConfig::load(path)`. Defaults: model `claude-sonnet-4-6`, API key env `ANTHROPIC_API_KEY`.

## Frontmatter

```rust
pub struct DocMeta {
    pub title: String,
    pub slug: String,        // unique kebab-case identifier
    pub description: String,
    pub lastmod: NaiveDate,
    pub tags: Vec<String>,
    pub sources: Vec<Source>, // { path: PathBuf, hash: String }
    pub sections: Vec<String>,
    pub path: PathBuf,        // set after parsing, #[serde(skip)]
}
```

Key functions:
- `parse(content) -> DocMeta` — extract YAML between `---` delimiters, deserialize
- `extract_section(content, name) -> String` — find `## <name>` heading (case-insensitive, kebab-normalized), return content to next `## ` or EOF
- `strip_frontmatter(content) -> &str` — return markdown body after frontmatter

## Index

```rust
pub struct Index {
    pub docs: HashMap<String, DocMeta>,  // keyed by slug
}
```

- `build(config)` — recursively scan `doc_paths`, parse `.md` files, apply exclude globs. Warns on slug collisions and frontmatter parse failures.
- `query(terms)` — case-insensitive substring match against title, description, slug
- `query_by_tags(tags)` — filter docs containing any of the given tags
- `read(slug, section)` — load doc, strip frontmatter, optionally extract named section

## Staleness

```rust
pub struct StaleDoc {
    pub slug: String,
    pub changed_sources: Vec<PathBuf>,
}
```

`check(index) -> Vec<StaleDoc>` — for each doc, recompute blake3 hash of every source file and compare against stored hash. Missing or unreadable files count as changed (with warning). `hash_file(path) -> String` computes blake3 hex digest.

## Hash Management

`update_hashes(doc_path)` — reads doc, recomputes blake3 hashes for all sources, updates `lastmod` to today. Uses targeted in-place string replacement to preserve original YAML formatting (field order, quoting, comments).

## Generator

```rust
pub struct Generator { config: AbathurConfig }
```

- `generate(source_path) -> String` — reads source file, sends to Claude API with `code_prompt()` schema, returns generated markdown. Uses 120s HTTP timeout.
- `regenerate_stale(index) -> Vec<String>` — finds stale docs, regenerates from first source file, updates hashes. Warns and skips docs with zero sources.

API call: `POST https://api.anthropic.com/v1/messages` with model from config, max_tokens 4096.
