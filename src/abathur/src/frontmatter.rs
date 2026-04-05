use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub path: PathBuf,
    pub hash: String,
}

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
        if let Some(rest) = line.strip_prefix("## ") {
            let heading = normalize_section_name(rest);
            if heading == target {
                start = Some(i + 1); // skip the heading line itself
            } else if start.is_some() && end.is_none() {
                end = Some(i);
            }
        }
    }

    let start = start.ok_or_else(|| anyhow::anyhow!("section '{section_name}' not found"))?;
    let end = end.unwrap_or(lines.len());

    let section: String = lines[start..end].join("\n").trim().to_string();

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
    name.trim().to_lowercase().replace([' ', '_'], "-")
}

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
