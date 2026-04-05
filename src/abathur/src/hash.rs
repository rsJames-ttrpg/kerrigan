use std::path::Path;

use crate::frontmatter;
use crate::staleness;

/// Read a doc file, recompute blake3 hashes for all sources, rewrite the file.
/// Uses targeted replacements to preserve the original YAML formatting.
pub fn update_hashes(doc_path: &Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(doc_path)?;
    let meta = frontmatter::parse(&content)?;

    let mut updated = content.clone();

    // Replace each source hash in-place using the full "hash: <value>" pattern.
    // YAML may quote empty strings as "" so we try the quoted form first.
    for source in &meta.sources {
        let new_hash = match staleness::hash_file(&source.path) {
            Ok(hash) => hash,
            Err(e) => {
                anyhow::bail!("cannot hash source '{}': {e}", source.path.display());
            }
        };
        if new_hash != source.hash {
            let new_pattern = format!("hash: {new_hash}");
            // Try quoted form first (e.g. hash: "old_value"), then unquoted
            let quoted = format!("hash: \"{}\"", source.hash);
            let unquoted = format!("hash: {}", source.hash);
            if updated.contains(&quoted) {
                updated = updated.replacen(&quoted, &new_pattern, 1);
            } else {
                updated = updated.replacen(&unquoted, &new_pattern, 1);
            }
        }
    }

    // Update lastmod date in-place
    let today = chrono::Local::now().date_naive().to_string();
    let old_date = meta.lastmod.to_string();
    if today != old_date {
        // Replace only within frontmatter context (the lastmod line)
        let old_lastmod = format!("lastmod: {old_date}");
        let new_lastmod = format!("lastmod: {today}");
        updated = updated.replacen(&old_lastmod, &new_lastmod, 1);
    }

    std::fs::write(doc_path, updated)?;

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

    #[test]
    fn update_hashes_from_empty() {
        let dir = TempDir::new().unwrap();

        let source = dir.path().join("lib.rs");
        std::fs::write(&source, b"fn hello() {}").unwrap();
        let real_hash = crate::staleness::hash_file(&source).unwrap();

        let doc_path = dir.path().join("doc.md");
        let doc = format!(
            "---\ntitle: Test\nslug: test\ndescription: A test\nlastmod: 2026-04-05\n\
             tags: []\nsources:\n  - path: {}\n    hash: \"\"\nsections: []\n---\n\n# Test\n",
            source.display()
        );
        std::fs::write(&doc_path, &doc).unwrap();

        update_hashes(&doc_path).unwrap();

        let updated = std::fs::read_to_string(&doc_path).unwrap();
        assert!(updated.contains(&format!("hash: {real_hash}")));
        assert!(!updated.contains("hash: \"\""));
        // Frontmatter delimiter preserved
        assert!(updated.starts_with("---\n"));
    }
}
