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

    // 7. Modify source -> becomes stale
    std::fs::write(
        &source,
        b"fn handle_request() -> Response { ok_with_body() }",
    )
    .unwrap();
    let stale = staleness::check(&index).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].slug, "api-handler");

    // 8. Update hashes -> no longer stale
    abathur::hash::update_hashes(&doc_path).unwrap();
    let index = Index::build(&config).unwrap();
    let stale = staleness::check(&index).unwrap();
    assert!(stale.is_empty(), "should not be stale after hash update");
}
