use std::collections::HashMap;
use std::path::Path;

use globset::{Glob, GlobSetBuilder};

use crate::config::AbathurConfig;
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;
    use crate::config::*;

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
            sources: SourcesConfig {
                roots: vec![],
                exclude: vec![],
            },
            generate: GenerateConfig::default(),
        };

        let index = Index::build(&config).unwrap();
        assert_eq!(index.docs.len(), 1);
        assert_eq!(index.docs.values().next().unwrap().slug, "alpha");
    }
}
