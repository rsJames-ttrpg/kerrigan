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
                Err(e) => {
                    eprintln!(
                        "warning: cannot read source '{}': {e}",
                        source.path.display(),
                    );
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::config::*;
    use crate::index::Index;

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
            sources: SourcesConfig {
                roots: vec![src_dir.clone()],
                exclude: vec![],
            },
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
            sources: SourcesConfig {
                roots: vec![],
                exclude: vec![],
            },
            generate: GenerateConfig::default(),
        };

        let index = Index::build(&config).unwrap();
        let stale = check(&index).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].slug, "test-doc");
    }
}
