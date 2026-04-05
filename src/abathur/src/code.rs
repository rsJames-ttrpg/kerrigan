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
