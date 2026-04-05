---
title: Nydus Client Library
slug: nydus
description: Shared typed HTTP client for Overseer REST API — used by Queen, Kerrigan CLI, and Evolution
lastmod: 2026-04-05
tags: [nydus, client, http, api]
sources:
  - path: src/nydus/src/client.rs
    hash: 00b11dbe374d32749e300351af364b9038586a307f527ac97ed8a1082203f6e1
  - path: src/nydus/src/types.rs
    hash: 71b7ec1f573adfe7940ead957b9afc183c63e2068f06e835a27f17a80da163f9
  - path: src/nydus/src/error.rs
    hash: 11c821f3a84189c1c9b5bebd70f4d7971b6fb41ad17c1a4addd1069c5b93210b
  - path: src/nydus/src/normalize.rs
    hash: bf571537fb53f519976de43cace85a9040035178b464c3d11b0fce8ff0468404
sections: [client, methods, types, error-handling, url-normalization]
---

# Nydus Client Library

## Client

```rust
pub struct NydusClient {
    base_url: String,
    client: reqwest::Client,
}

impl NydusClient {
    pub fn new(base_url: impl Into<String>) -> Self  // strips trailing slashes
}
```

Stateless wrapper — no connection pooling config, no auth headers. Each method makes a single HTTP request and deserializes the JSON response.

## Methods

**Job Definitions:**
- `create_definition(name, description, config) -> JobDefinition`
- `get_definition(id) -> JobDefinition`
- `list_definitions() -> Vec<JobDefinition>`

**Job Runs:**
- `start_run(definition_id, triggered_by, parent_id, config_overrides) -> JobRun`
- `list_runs(status: Option) -> Vec<JobRun>`
- `list_pending_runs() -> Vec<JobRun>` — shorthand for status="pending"
- `update_run(id, status, result, error) -> JobRun`
- `advance_run(id) -> JobRun` — pipeline stage advancement

**Tasks:**
- `create_task(subject, run_id, assigned_to) -> Task`
- `list_tasks(status, assigned_to, run_id) -> Vec<Task>`
- `update_task(id, status, assigned_to, output) -> Task`

**Hatcheries:**
- `register_hatchery(name, capabilities, max_concurrency) -> Hatchery`
- `heartbeat(hatchery_id, status, active_drones) -> Hatchery`
- `get_hatchery(id) -> Hatchery`
- `list_hatcheries(status: Option) -> Vec<Hatchery>`
- `deregister_hatchery(id)`
- `list_hatchery_jobs(hatchery_id, status) -> Vec<JobRun>`
- `assign_job(hatchery_id, job_run_id) -> JobRun`

**Artifacts:**
- `store_artifact(name, content_type, data, run_id, artifact_type) -> Artifact` — base64-encodes binary data
- `get_artifact(id) -> Vec<u8>` — raw bytes
- `list_artifacts(run_id, artifact_type, since) -> Vec<Artifact>`

**Auth:**
- `submit_auth_code(job_run_id, code)`
- `poll_auth_code(job_run_id) -> Option<String>`

**Credentials:**
- `create_credential(pattern, credential_type, secret) -> Credential`
- `list_credentials() -> Vec<Credential>`
- `delete_credential(id)`
- `match_credentials(repo_url) -> Vec<MatchedCredential>`

## Types

```rust
pub struct JobDefinition { pub id, name, description, config: Value }
pub struct JobRun { pub id, definition_id, parent_id, status, triggered_by, config_overrides, result, error }
pub struct Task { pub id, run_id, subject, status, assigned_to, output, updated_at }
pub struct Hatchery { pub id, name, status, capabilities, max_concurrency, active_drones }
pub struct Artifact { pub id, name, content_type, size, run_id, artifact_type, created_at }
pub struct Credential { pub id, pattern, credential_type, created_at, updated_at }
pub struct MatchedCredential { pub id, pattern, credential_type, secret }
```

## Error Handling

```rust
pub enum Error {
    Request(reqwest::Error),        // network/transport error
    Api { status: u16, body: String }, // non-2xx response
}
```

Internal `check_response()` helper converts non-2xx status codes to `Error::Api`.

## URL Normalization

`normalize_repo_url(url) -> String` — strips `https://`, `http://`, `git@` prefixes, converts SSH colons to slashes, removes `.git` suffix, trims trailing `/`.

`pattern_matches(normalized_url, normalized_pattern) -> Option<usize>` — exact match or wildcard (`*` suffix). Returns specificity score (pattern length) for ranking when multiple patterns match.
