---
title: Abathur Documentation Indexer
slug: abathur
description: Library for markdown doc indexing with YAML frontmatter, staleness detection, and AI generation
lastmod: 2026-04-05
tags: [abathur, documentation, indexing, generation]
sources:
  - path: src/abathur/src/config.rs
    hash: 4ce041a96fe24bef22bb252e1cf520593e7f11c136cd9b9c353dfc9bcfbcd2e8
  - path: src/abathur/src/frontmatter.rs
    hash: 5ed7fa3e7f55ce75ecd0b5c18b28da8f3c8fcdeaa03e04917584981309b0bc1c
  - path: src/abathur/src/index.rs
    hash: ada8973f4918bd6c1696b2ea10507933748c3b2211345aaf1502dcaff0c19a87
  - path: src/abathur/src/hash.rs
    hash: 5954217ea82aaf776758b125afa3ff80d48cb689c847fc8ad75a60ad2dc3b0e6
  - path: src/abathur/src/staleness.rs
    hash: 6ff29e78c5ed1029bcb4361bfeda4188e509fcf888ed4cbd2f16f1badde4d144
  - path: src/abathur/src/generator.rs
    hash: 44e9a67ca2552917241c62f68dca765f9af300e3e446b6693de72908a29005a5
  - path: src/abathur/src/code.rs
    hash: e149de477e07eae46b540465c06f4cca44d14c7042ac1cb55b8323ca116144ee
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
