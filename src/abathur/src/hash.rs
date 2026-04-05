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
                anyhow::bail!("cannot hash source '{}': {e}", source.path.display());
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

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
