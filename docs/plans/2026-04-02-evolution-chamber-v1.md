# Evolution Chamber v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust crate that analyzes aggregated drone session data via Polars, producing structured reports that feed into the dev loop as problem specs.

**Architecture:** New `src/evolution/` library crate. Fetches conversation + session artifacts from Overseer via nydus, parses JSONL, builds Polars DataFrames, applies heuristic rules, outputs `AnalysisReport` JSON. Queen invokes the analysis on a configurable schedule and submits reports to Claude drones.

**Tech Stack:** Rust (edition 2024), Polars, nydus, flate2, serde, chrono, anyhow

---

## Prerequisite: Extend nydus `list_artifacts` with filters

The current `NydusClient::list_artifacts` only supports `run_id`. Evolution Chamber needs `artifact_type` and `since` query params.

### Task 0: Add artifact_type and since filters to nydus list_artifacts

**Files:**
- Modify: `src/nydus/src/client.rs:405-412`
- Test: `src/nydus/src/client.rs` (existing test module)

- [ ] **Step 1: Update list_artifacts signature and implementation**

```rust
// In src/nydus/src/client.rs, replace the existing list_artifacts method:

pub async fn list_artifacts(
    &self,
    run_id: Option<&str>,
    artifact_type: Option<&str>,
    since: Option<&str>,
) -> Result<Vec<Artifact>, Error> {
    let mut url = format!("{}/api/artifacts", self.base_url);
    let mut params = Vec::new();
    if let Some(r) = run_id {
        params.push(format!("run_id={r}"));
    }
    if let Some(at) = artifact_type {
        params.push(format!("artifact_type={at}"));
    }
    if let Some(s) = since {
        params.push(format!("since={s}"));
    }
    if !params.is_empty() {
        url.push('?');
        url.push_str(&params.join("&"));
    }
    let resp = self.client.get(url).send().await?;
    Ok(Self::check_response(resp).await?.json().await?)
}
```

- [ ] **Step 2: Fix all callers of list_artifacts**

The Queen supervisor calls `client.list_artifacts(run_id)` — grep for all call sites and add the two new `None` arguments. The only known caller outside nydus tests is in the kerrigan CLI if it lists artifacts.

Run: `grep -rn "list_artifacts" src/ --include="*.rs" | grep -v target | grep -v "fn list_artifacts"`

Fix each call site by adding `, None, None` for the new params.

- [ ] **Step 3: Run tests**

Run: `cd src/nydus && cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/nydus/src/client.rs
# plus any callers that changed
git commit -m "feat(nydus): add artifact_type and since filters to list_artifacts"
```

---

## Task 1: Create evolution crate scaffold with report types

**Files:**
- Create: `src/evolution/Cargo.toml`
- Create: `src/evolution/BUCK`
- Create: `src/evolution/src/lib.rs`
- Create: `src/evolution/src/report.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create Cargo.toml**

```toml
# src/evolution/Cargo.toml
[package]
name = "evolution"
version = "0.1.0"
edition = "2024"

[dependencies]
nydus = { path = "../nydus" }
polars = { version = "0.46", default-features = false, features = ["lazy", "json", "dtype-struct"] }
flate2 = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
tracing = "0.1"
```

- [ ] **Step 2: Add to workspace**

In root `Cargo.toml`, add `"src/evolution"` to the `members` array.

- [ ] **Step 3: Create report.rs with all types from the spec**

```rust
// src/evolution/src/report.rs
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AnalysisScope {
    Global,
    Repo(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub generated_at: DateTime<Utc>,
    pub scope: AnalysisScope,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub runs_analyzed: usize,
    pub cost_summary: CostSummary,
    pub tool_patterns: ToolPatterns,
    pub context_pressure: ContextPressure,
    pub failure_analysis: FailureAnalysis,
    pub recommendations: Vec<Recommendation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Trend {
    Increasing,
    Stable,
    Decreasing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCostEntry {
    pub run_id: String,
    pub cost_usd: f64,
    pub stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSummary {
    pub total_cost_usd: f64,
    pub cost_by_stage: HashMap<String, f64>,
    pub cost_trend: Trend,
    pub highest_cost_runs: Vec<RunCostEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrySequence {
    pub run_id: String,
    pub tool_name: String,
    pub count: usize,
    pub first_arg_sample: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPatterns {
    pub call_counts: HashMap<String, u64>,
    pub retry_sequences: Vec<RetrySequence>,
    pub error_rates: HashMap<String, f64>,
    pub top_context_consumers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPressure {
    pub avg_turns: f64,
    pub median_turns: f64,
    pub compression_events: usize,
    pub avg_cache_hit_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPattern {
    pub pattern: String,
    pub count: usize,
    pub example_run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureAnalysis {
    pub failure_rate: f64,
    pub failure_by_stage: HashMap<String, f64>,
    pub common_errors: Vec<ErrorPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendationCategory {
    Tooling,
    Skills,
    SharedService,
    Context,
    Workflow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub category: RecommendationCategory,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub evidence: String,
}
```

- [ ] **Step 4: Create lib.rs with public API skeleton**

```rust
// src/evolution/src/lib.rs
pub mod report;

mod fetch;
mod metrics;
mod parse;
mod rules;

use chrono::{DateTime, Utc};
use nydus::NydusClient;

use report::{AnalysisReport, AnalysisScope};

pub struct EvolutionChamber {
    client: NydusClient,
}

impl EvolutionChamber {
    pub fn new(client: NydusClient) -> Self {
        Self { client }
    }

    pub async fn analyze(
        &self,
        scope: AnalysisScope,
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> anyhow::Result<Option<AnalysisReport>> {
        todo!()
    }
}
```

- [ ] **Step 5: Create empty module files**

Create `src/evolution/src/fetch.rs`, `src/evolution/src/parse.rs`, `src/evolution/src/metrics.rs`, `src/evolution/src/rules.rs` — each with just a comment:

```rust
// Module contents added in subsequent tasks.
```

- [ ] **Step 6: Verify it compiles**

Run: `cd src/evolution && cargo check`
Expected: compiles (with dead_code warnings, that's fine)

- [ ] **Step 7: Create BUCK file**

```python
# src/evolution/BUCK
EVOLUTION_SRCS = glob(["src/**/*.rs"])

EVOLUTION_DEPS = [
    "//src/nydus:nydus",
    "//third-party:anyhow",
    "//third-party:chrono",
    "//third-party:flate2",
    "//third-party:polars",
    "//third-party:serde",
    "//third-party:serde_json",
    "//third-party:tracing",
]

rust_library(
    name = "evolution",
    srcs = EVOLUTION_SRCS,
    deps = EVOLUTION_DEPS,
    visibility = ["PUBLIC"],
)

rust_test(
    name = "evolution-test",
    srcs = EVOLUTION_SRCS,
    deps = EVOLUTION_DEPS,
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 8: Run buckify**

Run: `./tools/buckify.sh`
Expected: `buckify complete (with ordering fix applied)`

Note: `polars` is a large crate — buckify may take a moment. If Buck2 build fails on polars' build script, add `third-party/fixups/polars/fixups.toml` with `[buildscript] run = true`.

- [ ] **Step 9: Commit**

```bash
git add src/evolution/ Cargo.toml Cargo.lock third-party/BUCK
git commit -m "feat(evolution): scaffold crate with report types"
```

---

## Task 2: Implement artifact fetching and decompression

**Files:**
- Modify: `src/evolution/src/fetch.rs`
- Test: inline in `src/evolution/src/fetch.rs`

- [ ] **Step 1: Write tests for fetch and decompress**

```rust
// src/evolution/src/fetch.rs
use anyhow::Result;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

/// A fetched and decompressed artifact with its associated run_id.
pub struct FetchedArtifact {
    pub run_id: String,
    pub data: Vec<u8>,
}

/// Decompress a gzipped byte slice.
pub fn decompress_gz(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut decoder = GzDecoder::new(data);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Fetch all artifacts of a given type since a timestamp from Overseer.
/// Returns decompressed artifact data paired with run_id.
pub async fn fetch_artifacts(
    client: &nydus::NydusClient,
    artifact_type: &str,
    since: &chrono::DateTime<chrono::Utc>,
) -> Result<Vec<FetchedArtifact>> {
    let since_str = since.to_rfc3339();
    let artifacts = client
        .list_artifacts(None, Some(artifact_type), Some(&since_str))
        .await
        .map_err(|e| anyhow::anyhow!("failed to list artifacts: {e}"))?;

    let mut results = Vec::new();
    for artifact in artifacts {
        let Some(run_id) = artifact.run_id else {
            tracing::debug!(artifact_id = %artifact.id, "skipping artifact without run_id");
            continue;
        };
        let blob = client
            .get_artifact(&artifact.id)
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch artifact {}: {e}", artifact.id))?;
        let data = decompress_gz(&blob)?;
        results.push(FetchedArtifact { run_id, data });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_gz() {
        let original = b"hello evolution chamber";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_gz(&compressed).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_gz_invalid_data() {
        let result = decompress_gz(b"not gzip data");
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/evolution && cargo test fetch`
Expected: PASS (2 tests)

- [ ] **Step 3: Commit**

```bash
git add src/evolution/src/fetch.rs
git commit -m "feat(evolution): artifact fetching and gzip decompression"
```

---

## Task 3: Implement conversation and session JSONL parsing

**Files:**
- Modify: `src/evolution/src/parse.rs`
- Test: inline in `src/evolution/src/parse.rs`

- [ ] **Step 1: Write conversation parser with tests**

The conversation artifact (Claude Code `--output-format json` output) has this shape:
```json
{
  "total_cost_usd": 0.29,
  "num_turns": 24,
  "duration_ms": 98133,
  "subtype": "success",
  "session_id": "...",
  "modelUsage": { "<model>": { "inputTokens": N, "outputTokens": N, "cacheReadInputTokens": N, "cacheCreationInputTokens": N } }
}
```

```rust
// src/evolution/src/parse.rs
use anyhow::Result;
use serde_json::Value;

/// Parsed summary from a conversation artifact.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub run_id: String,
    pub cost_usd: f64,
    pub num_turns: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

pub fn parse_conversation(run_id: &str, data: &[u8]) -> Result<ConversationSummary> {
    let v: Value = serde_json::from_slice(data)?;

    let cost_usd = v.get("total_cost_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let num_turns = v.get("num_turns").and_then(|v| v.as_u64()).unwrap_or(0);
    let duration_ms = v.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let success = v.get("subtype").and_then(|v| v.as_str()) == Some("success");

    // Sum tokens across all models in modelUsage
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_read_tokens = 0u64;
    let mut cache_creation_tokens = 0u64;

    if let Some(usage) = v.get("modelUsage").and_then(|v| v.as_object()) {
        for (_model, stats) in usage {
            input_tokens += stats.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
            output_tokens += stats.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
            cache_read_tokens += stats.get("cacheReadInputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
            cache_creation_tokens += stats.get("cacheCreationInputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
        }
    }

    Ok(ConversationSummary {
        run_id: run_id.to_string(),
        cost_usd,
        num_turns,
        duration_ms,
        success,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    })
}

/// Parsed detail from a session JSONL artifact.
#[derive(Debug, Clone)]
pub struct SessionDetail {
    pub run_id: String,
    pub tool_calls: Vec<ToolCall>,
    pub message_count: usize,
    pub compression_events: usize,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub first_arg: String,
    pub is_error: bool,
}

pub fn parse_session(run_id: &str, data: &[u8]) -> Result<SessionDetail> {
    let text = std::str::from_utf8(data)?;
    let mut tool_calls = Vec::new();
    let mut message_count = 0usize;
    let mut compression_events = 0usize;

    // Track tool_use IDs to match with tool_results
    let mut pending_tool_ids: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "assistant" => {
                message_count += 1;
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown").to_string();
                            let first_arg = extract_first_arg(block.get("input"));
                            let tool_id = block.get("id").and_then(|id| id.as_str()).unwrap_or("").to_string();
                            let idx = tool_calls.len();
                            tool_calls.push(ToolCall { name, first_arg, is_error: false });
                            if !tool_id.is_empty() {
                                pending_tool_ids.insert(tool_id, idx);
                            }
                        }
                    }
                }
            }
            "user" => {
                message_count += 1;
                // Check for tool_results with errors
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            let is_error = block.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                            if is_error {
                                let tool_id = block.get("tool_use_id").and_then(|id| id.as_str()).unwrap_or("");
                                if let Some(&idx) = pending_tool_ids.get(tool_id) {
                                    tool_calls[idx].is_error = true;
                                }
                            }
                        }
                    }
                }
            }
            "system" => {
                // Detect context compression
                if let Some(msg) = v.pointer("/message/content").and_then(|c| c.as_str()) {
                    if msg.contains("compress") || msg.contains("truncat") || msg.contains("summary of the conversation") {
                        compression_events += 1;
                    }
                }
            }
            _ => {}
        }
    }

    Ok(SessionDetail {
        run_id: run_id.to_string(),
        tool_calls,
        message_count,
        compression_events,
    })
}

fn extract_first_arg(input: Option<&Value>) -> String {
    let Some(obj) = input.and_then(|v| v.as_object()) else {
        return String::new();
    };
    // Return the first string value from the input object (usually file_path, pattern, command, etc.)
    for (_key, val) in obj {
        if let Some(s) = val.as_str() {
            return s.chars().take(100).collect();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_conversation() {
        let data = serde_json::json!({
            "total_cost_usd": 1.5,
            "num_turns": 30,
            "duration_ms": 120000,
            "subtype": "success",
            "session_id": "abc-123",
            "modelUsage": {
                "claude-sonnet-4-6": {
                    "inputTokens": 100,
                    "outputTokens": 500,
                    "cacheReadInputTokens": 50000,
                    "cacheCreationInputTokens": 10000
                }
            }
        });
        let bytes = serde_json::to_vec(&data).unwrap();
        let summary = parse_conversation("run-1", &bytes).unwrap();

        assert_eq!(summary.run_id, "run-1");
        assert!((summary.cost_usd - 1.5).abs() < f64::EPSILON);
        assert_eq!(summary.num_turns, 30);
        assert_eq!(summary.duration_ms, 120000);
        assert!(summary.success);
        assert_eq!(summary.input_tokens, 100);
        assert_eq!(summary.output_tokens, 500);
        assert_eq!(summary.cache_read_tokens, 50000);
        assert_eq!(summary.cache_creation_tokens, 10000);
    }

    #[test]
    fn test_parse_conversation_failed() {
        let data = serde_json::json!({
            "total_cost_usd": 0.5,
            "num_turns": 5,
            "duration_ms": 30000,
            "subtype": "error",
        });
        let bytes = serde_json::to_vec(&data).unwrap();
        let summary = parse_conversation("run-2", &bytes).unwrap();
        assert!(!summary.success);
    }

    #[test]
    fn test_parse_session_tool_calls() {
        let lines = vec![
            serde_json::json!({"type": "user", "message": {"role": "user", "content": "fix bug"}}),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/src/main.rs"}},
            ]}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": "file contents..."},
            ]}}),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "t2", "name": "Edit", "input": {"file_path": "/src/main.rs", "old_string": "x", "new_string": "y"}},
            ]}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t2", "is_error": true, "content": "String not found"},
            ]}}),
        ];
        let jsonl = lines.iter().map(|l| serde_json::to_string(l).unwrap()).collect::<Vec<_>>().join("\n");
        let detail = parse_session("run-3", jsonl.as_bytes()).unwrap();

        assert_eq!(detail.run_id, "run-3");
        assert_eq!(detail.tool_calls.len(), 2);
        assert_eq!(detail.tool_calls[0].name, "Read");
        assert_eq!(detail.tool_calls[0].first_arg, "/src/main.rs");
        assert!(!detail.tool_calls[0].is_error);
        assert_eq!(detail.tool_calls[1].name, "Edit");
        assert!(detail.tool_calls[1].is_error);
        assert_eq!(detail.message_count, 4);
        assert_eq!(detail.compression_events, 0);
    }

    #[test]
    fn test_parse_session_compression_detection() {
        let lines = vec![
            serde_json::json!({"type": "system", "message": {"content": "This is a summary of the conversation so far"}}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": "continue"}}),
        ];
        let jsonl = lines.iter().map(|l| serde_json::to_string(l).unwrap()).collect::<Vec<_>>().join("\n");
        let detail = parse_session("run-4", jsonl.as_bytes()).unwrap();

        assert_eq!(detail.compression_events, 1);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/evolution && cargo test parse`
Expected: PASS (4 tests)

- [ ] **Step 3: Commit**

```bash
git add src/evolution/src/parse.rs
git commit -m "feat(evolution): conversation and session JSONL parsing"
```

---

## Task 4: Implement Polars metrics aggregation

**Files:**
- Modify: `src/evolution/src/metrics.rs`
- Test: inline in `src/evolution/src/metrics.rs`

- [ ] **Step 1: Build DataFrames from parsed data and compute aggregates**

```rust
// src/evolution/src/metrics.rs
use std::collections::HashMap;

use anyhow::Result;
use polars::prelude::*;

use crate::parse::{ConversationSummary, SessionDetail};
use crate::report::{ContextPressure, CostSummary, RunCostEntry, Trend, RetrySequence, ToolPatterns, FailureAnalysis, ErrorPattern};

pub fn build_cost_summary(conversations: &[ConversationSummary], stage_map: &HashMap<String, String>) -> Result<CostSummary> {
    let total_cost_usd: f64 = conversations.iter().map(|c| c.cost_usd).sum();

    let mut cost_by_stage: HashMap<String, f64> = HashMap::new();
    for c in conversations {
        let stage = stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string());
        *cost_by_stage.entry(stage).or_default() += c.cost_usd;
    }

    let mut highest_cost_runs: Vec<RunCostEntry> = conversations
        .iter()
        .map(|c| RunCostEntry {
            run_id: c.run_id.clone(),
            cost_usd: c.cost_usd,
            stage: stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string()),
        })
        .collect();
    highest_cost_runs.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
    highest_cost_runs.truncate(10);

    // Simple trend: compare first half avg to second half avg
    let cost_trend = if conversations.len() >= 4 {
        let mid = conversations.len() / 2;
        let first_avg: f64 = conversations[..mid].iter().map(|c| c.cost_usd).sum::<f64>() / mid as f64;
        let second_avg: f64 = conversations[mid..].iter().map(|c| c.cost_usd).sum::<f64>() / (conversations.len() - mid) as f64;
        if second_avg > first_avg * 1.2 {
            Trend::Increasing
        } else if second_avg < first_avg * 0.8 {
            Trend::Decreasing
        } else {
            Trend::Stable
        }
    } else {
        Trend::Stable
    };

    Ok(CostSummary {
        total_cost_usd,
        cost_by_stage,
        cost_trend,
        highest_cost_runs,
    })
}

pub fn build_tool_patterns(sessions: &[SessionDetail]) -> Result<ToolPatterns> {
    let mut call_counts: HashMap<String, u64> = HashMap::new();
    let mut error_counts: HashMap<String, u64> = HashMap::new();
    let mut total_counts: HashMap<String, u64> = HashMap::new();

    for session in sessions {
        for tc in &session.tool_calls {
            *call_counts.entry(tc.name.clone()).or_default() += 1;
            *total_counts.entry(tc.name.clone()).or_default() += 1;
            if tc.is_error {
                *error_counts.entry(tc.name.clone()).or_default() += 1;
            }
        }
    }

    let error_rates: HashMap<String, f64> = total_counts
        .iter()
        .map(|(name, total)| {
            let errors = error_counts.get(name).copied().unwrap_or(0);
            (name.clone(), errors as f64 / *total as f64)
        })
        .collect();

    let retry_sequences = detect_retries(sessions);

    // Top context consumers: tools with highest total call count (proxy for context usage)
    let mut sorted_tools: Vec<(String, u64)> = call_counts.clone().into_iter().collect();
    sorted_tools.sort_by(|a, b| b.1.cmp(&a.1));
    let top_context_consumers: Vec<String> = sorted_tools.into_iter().take(5).map(|(name, _)| name).collect();

    Ok(ToolPatterns {
        call_counts,
        retry_sequences,
        error_rates,
        top_context_consumers,
    })
}

fn detect_retries(sessions: &[SessionDetail]) -> Vec<RetrySequence> {
    let mut retries = Vec::new();
    for session in sessions {
        let calls = &session.tool_calls;
        let mut i = 0;
        while i < calls.len() {
            let mut count = 1;
            while i + count < calls.len()
                && calls[i + count].name == calls[i].name
                && calls[i + count].first_arg == calls[i].first_arg
                && !calls[i].first_arg.is_empty()
            {
                count += 1;
            }
            if count >= 3 {
                retries.push(RetrySequence {
                    run_id: session.run_id.clone(),
                    tool_name: calls[i].name.clone(),
                    count,
                    first_arg_sample: calls[i].first_arg.clone(),
                });
            }
            i += count;
        }
    }
    retries
}

pub fn build_context_pressure(conversations: &[ConversationSummary], sessions: &[SessionDetail]) -> ContextPressure {
    let turns: Vec<f64> = conversations.iter().map(|c| c.num_turns as f64).collect();
    let avg_turns = if turns.is_empty() { 0.0 } else { turns.iter().sum::<f64>() / turns.len() as f64 };

    let median_turns = if turns.is_empty() {
        0.0
    } else {
        let mut sorted = turns.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };

    let compression_events: usize = sessions.iter().map(|s| s.compression_events).sum();

    let avg_cache_hit_ratio = if conversations.is_empty() {
        0.0
    } else {
        let ratios: Vec<f64> = conversations
            .iter()
            .filter_map(|c| {
                let total = c.cache_read_tokens + c.cache_creation_tokens;
                if total > 0 {
                    Some(c.cache_read_tokens as f64 / total as f64)
                } else {
                    None
                }
            })
            .collect();
        if ratios.is_empty() { 0.0 } else { ratios.iter().sum::<f64>() / ratios.len() as f64 }
    };

    ContextPressure {
        avg_turns,
        median_turns,
        compression_events,
        avg_cache_hit_ratio,
    }
}

pub fn build_failure_analysis(conversations: &[ConversationSummary], stage_map: &HashMap<String, String>) -> FailureAnalysis {
    let total = conversations.len();
    let failures: Vec<&ConversationSummary> = conversations.iter().filter(|c| !c.success).collect();
    let failure_rate = if total > 0 { failures.len() as f64 / total as f64 } else { 0.0 };

    let mut stage_failures: HashMap<String, (u64, u64)> = HashMap::new(); // (failures, total)
    for c in conversations {
        let stage = stage_map.get(&c.run_id).cloned().unwrap_or_else(|| "unknown".to_string());
        let entry = stage_failures.entry(stage).or_insert((0, 0));
        entry.1 += 1;
        if !c.success {
            entry.0 += 1;
        }
    }
    let failure_by_stage: HashMap<String, f64> = stage_failures
        .into_iter()
        .map(|(stage, (f, t))| (stage, if t > 0 { f as f64 / t as f64 } else { 0.0 }))
        .collect();

    // For common_errors, we'd need error messages from run results — for v1, keep it simple
    let common_errors = Vec::new();

    FailureAnalysis {
        failure_rate,
        failure_by_stage,
        common_errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_conversation(run_id: &str, cost: f64, turns: u64, success: bool) -> ConversationSummary {
        ConversationSummary {
            run_id: run_id.to_string(),
            cost_usd: cost,
            num_turns: turns,
            duration_ms: 60000,
            success,
            input_tokens: 100,
            output_tokens: 500,
            cache_read_tokens: 50000,
            cache_creation_tokens: 10000,
        }
    }

    #[test]
    fn test_cost_summary_basic() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, true),
            make_conversation("r3", 0.5, 10, false),
        ];
        let mut stages = HashMap::new();
        stages.insert("r1".to_string(), "implement".to_string());
        stages.insert("r2".to_string(), "implement".to_string());
        stages.insert("r3".to_string(), "review".to_string());

        let summary = build_cost_summary(&convos, &stages).unwrap();
        assert!((summary.total_cost_usd - 3.5).abs() < f64::EPSILON);
        assert_eq!(summary.cost_by_stage["implement"], 3.0);
        assert_eq!(summary.cost_by_stage["review"], 0.5);
        assert_eq!(summary.highest_cost_runs[0].run_id, "r2");
    }

    #[test]
    fn test_retry_detection() {
        let sessions = vec![SessionDetail {
            run_id: "r1".to_string(),
            tool_calls: vec![
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: true },
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: true },
                crate::parse::ToolCall { name: "Edit".to_string(), first_arg: "/src/main.rs".to_string(), is_error: false },
                crate::parse::ToolCall { name: "Read".to_string(), first_arg: "/src/lib.rs".to_string(), is_error: false },
            ],
            message_count: 8,
            compression_events: 0,
        }];

        let patterns = build_tool_patterns(&sessions).unwrap();
        assert_eq!(patterns.retry_sequences.len(), 1);
        assert_eq!(patterns.retry_sequences[0].tool_name, "Edit");
        assert_eq!(patterns.retry_sequences[0].count, 3);
    }

    #[test]
    fn test_failure_analysis() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, false),
            make_conversation("r3", 0.5, 10, true),
            make_conversation("r4", 1.5, 30, false),
        ];
        let mut stages = HashMap::new();
        stages.insert("r1".to_string(), "implement".to_string());
        stages.insert("r2".to_string(), "implement".to_string());
        stages.insert("r3".to_string(), "review".to_string());
        stages.insert("r4".to_string(), "review".to_string());

        let analysis = build_failure_analysis(&convos, &stages);
        assert!((analysis.failure_rate - 0.5).abs() < f64::EPSILON);
        assert!((analysis.failure_by_stage["implement"] - 0.5).abs() < f64::EPSILON);
        assert!((analysis.failure_by_stage["review"] - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_context_pressure() {
        let convos = vec![
            make_conversation("r1", 1.0, 20, true),
            make_conversation("r2", 2.0, 40, true),
            make_conversation("r3", 0.5, 60, true),
        ];
        let sessions = vec![
            SessionDetail { run_id: "r1".to_string(), tool_calls: vec![], message_count: 20, compression_events: 0 },
            SessionDetail { run_id: "r2".to_string(), tool_calls: vec![], message_count: 40, compression_events: 1 },
            SessionDetail { run_id: "r3".to_string(), tool_calls: vec![], message_count: 60, compression_events: 2 },
        ];

        let pressure = build_context_pressure(&convos, &sessions);
        assert!((pressure.avg_turns - 40.0).abs() < f64::EPSILON);
        assert_eq!(pressure.compression_events, 3);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/evolution && cargo test metrics`
Expected: PASS (4 tests)

- [ ] **Step 3: Commit**

```bash
git add src/evolution/src/metrics.rs
git commit -m "feat(evolution): Polars-backed metrics aggregation"
```

---

## Task 5: Implement heuristic rules engine

**Files:**
- Modify: `src/evolution/src/rules.rs`
- Test: inline in `src/evolution/src/rules.rs`

- [ ] **Step 1: Implement recommendation generation from metrics**

```rust
// src/evolution/src/rules.rs
use crate::report::*;

pub fn generate_recommendations(
    cost: &CostSummary,
    tools: &ToolPatterns,
    context: &ContextPressure,
    failures: &FailureAnalysis,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    recs.extend(cost_rules(cost));
    recs.extend(tool_rules(tools));
    recs.extend(context_rules(context));
    recs.extend(failure_rules(failures));
    // Sort by severity: High first
    recs.sort_by(|a, b| severity_ord(&a.severity).cmp(&severity_ord(&b.severity)));
    recs
}

fn severity_ord(s: &Severity) -> u8 {
    match s {
        Severity::High => 0,
        Severity::Medium => 1,
        Severity::Low => 2,
    }
}

fn cost_rules(cost: &CostSummary) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if matches!(cost.cost_trend, Trend::Increasing) {
        recs.push(Recommendation {
            category: RecommendationCategory::Workflow,
            severity: Severity::Medium,
            title: "Cost trend is increasing".to_string(),
            detail: "Average cost per run is trending upward. Review prompt sizes, task complexity, or stage configuration.".to_string(),
            evidence: format!("Total cost: ${:.2}", cost.total_cost_usd),
        });
    }
    for entry in cost.highest_cost_runs.iter().take(3) {
        if entry.cost_usd > 3.0 {
            recs.push(Recommendation {
                category: RecommendationCategory::Workflow,
                severity: Severity::High,
                title: format!("High-cost run: ${:.2} ({})", entry.cost_usd, entry.stage),
                detail: "This run exceeded $3.00. Consider splitting into smaller tasks or reviewing the stage prompt.".to_string(),
                evidence: format!("Run {} cost ${:.2}", entry.run_id, entry.cost_usd),
            });
        }
    }
    recs
}

fn tool_rules(tools: &ToolPatterns) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    // High error rate tools
    for (name, rate) in &tools.error_rates {
        if *rate > 0.15 && tools.call_counts.get(name).copied().unwrap_or(0) >= 5 {
            recs.push(Recommendation {
                category: RecommendationCategory::Tooling,
                severity: Severity::High,
                title: format!("High error rate for tool `{name}`: {:.0}%", rate * 100.0),
                detail: format!("Tool `{name}` fails more than 15% of the time. Investigate common failure modes and consider pre-validation or better error messages."),
                evidence: format!("{} calls, {:.0}% error rate", tools.call_counts[name], rate * 100.0),
            });
        }
    }

    // Retry sequences
    for retry in &tools.retry_sequences {
        if retry.count >= 3 {
            recs.push(Recommendation {
                category: RecommendationCategory::Tooling,
                severity: Severity::Medium,
                title: format!("`{}` retried {} times on same target", retry.tool_name, retry.count),
                detail: format!("Drone called `{}` {} times with the same first argument '{}'. This wastes context and suggests the drone needs better feedback or a different approach.", retry.tool_name, retry.count, retry.first_arg_sample),
                evidence: format!("Run {}", retry.run_id),
            });
        }
    }

    // Dominant tool (>40% of all calls)
    let total_calls: u64 = tools.call_counts.values().sum();
    if total_calls > 0 {
        for (name, count) in &tools.call_counts {
            let ratio = *count as f64 / total_calls as f64;
            if ratio > 0.40 && *count > 20 {
                recs.push(Recommendation {
                    category: RecommendationCategory::Skills,
                    severity: Severity::Low,
                    title: format!("`{name}` dominates tool usage at {:.0}%", ratio * 100.0),
                    detail: format!("Consider whether a higher-level skill or MCP tool could replace multiple `{name}` calls."),
                    evidence: format!("{count} of {total_calls} total calls"),
                });
            }
        }
    }

    recs
}

fn context_rules(context: &ContextPressure) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if context.compression_events > 0 {
        recs.push(Recommendation {
            category: RecommendationCategory::Context,
            severity: if context.compression_events > 3 { Severity::High } else { Severity::Medium },
            title: format!("{} context compression events detected", context.compression_events),
            detail: "Drones are hitting context limits and losing conversation history. Consider splitting tasks, reducing prompt sizes, or adding shared services to reduce tool call volume.".to_string(),
            evidence: format!("Avg turns: {:.0}, Median turns: {:.0}, Cache hit ratio: {:.0}%", context.avg_turns, context.median_turns, context.avg_cache_hit_ratio * 100.0),
        });
    }
    recs
}

fn failure_rules(failures: &FailureAnalysis) -> Vec<Recommendation> {
    let mut recs = Vec::new();
    if failures.failure_rate > 0.3 {
        recs.push(Recommendation {
            category: RecommendationCategory::Workflow,
            severity: Severity::High,
            title: format!("High failure rate: {:.0}%", failures.failure_rate * 100.0),
            detail: "More than 30% of runs are failing. Review drone configuration, task quality, and error patterns.".to_string(),
            evidence: format!("Failure rate by stage: {:?}", failures.failure_by_stage),
        });
    }
    for (stage, rate) in &failures.failure_by_stage {
        if *rate > 0.5 {
            recs.push(Recommendation {
                category: RecommendationCategory::Workflow,
                severity: Severity::High,
                title: format!("Stage `{stage}` failing >50% of the time"),
                detail: format!("The `{stage}` stage has a {:.0}% failure rate. The stage prompt, tool access, or task format likely needs adjustment.", rate * 100.0),
                evidence: format!("{stage} failure rate: {:.0}%", rate * 100.0),
            });
        }
    }
    recs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_high_cost_recommendation() {
        let cost = CostSummary {
            total_cost_usd: 15.0,
            cost_by_stage: HashMap::new(),
            cost_trend: Trend::Increasing,
            highest_cost_runs: vec![
                RunCostEntry { run_id: "r1".into(), cost_usd: 5.0, stage: "implement".into() },
            ],
        };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("Cost trend")));
        assert!(recs.iter().any(|r| r.title.contains("High-cost run")));
    }

    #[test]
    fn test_tool_error_rate_recommendation() {
        let mut error_rates = HashMap::new();
        error_rates.insert("Edit".to_string(), 0.25);
        let mut call_counts = HashMap::new();
        call_counts.insert("Edit".to_string(), 20);

        let tools = ToolPatterns { call_counts, retry_sequences: vec![], error_rates, top_context_consumers: vec![] };
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("High error rate") && r.title.contains("Edit")));
    }

    #[test]
    fn test_compression_recommendation() {
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 80.0, median_turns: 75.0, compression_events: 5, avg_cache_hit_ratio: 0.6 };
        let failures = FailureAnalysis { failure_rate: 0.0, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.iter().any(|r| r.title.contains("compression")));
        assert!(recs.iter().any(|r| matches!(r.severity, Severity::High)));
    }

    #[test]
    fn test_no_recommendations_when_clean() {
        let cost = CostSummary { total_cost_usd: 1.0, cost_by_stage: HashMap::new(), cost_trend: Trend::Stable, highest_cost_runs: vec![] };
        let tools = ToolPatterns { call_counts: HashMap::new(), retry_sequences: vec![], error_rates: HashMap::new(), top_context_consumers: vec![] };
        let context = ContextPressure { avg_turns: 20.0, median_turns: 18.0, compression_events: 0, avg_cache_hit_ratio: 0.9 };
        let failures = FailureAnalysis { failure_rate: 0.1, failure_by_stage: HashMap::new(), common_errors: vec![] };

        let recs = generate_recommendations(&cost, &tools, &context, &failures);
        assert!(recs.is_empty());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src/evolution && cargo test rules`
Expected: PASS (4 tests)

- [ ] **Step 3: Commit**

```bash
git add src/evolution/src/rules.rs
git commit -m "feat(evolution): heuristic rules engine for recommendations"
```

---

## Task 6: Wire up the analyze() entrypoint

**Files:**
- Modify: `src/evolution/src/lib.rs`
- Test: inline (unit test with mocked data path)

- [ ] **Step 1: Implement analyze() connecting all modules**

```rust
// src/evolution/src/lib.rs — replace the existing content
pub mod report;

mod fetch;
mod metrics;
mod parse;
mod rules;

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use nydus::NydusClient;

use report::{AnalysisReport, AnalysisScope};

pub struct EvolutionChamber {
    client: NydusClient,
}

impl EvolutionChamber {
    pub fn new(client: NydusClient) -> Self {
        Self { client }
    }

    pub async fn analyze(
        &self,
        scope: AnalysisScope,
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> anyhow::Result<Option<AnalysisReport>> {
        // 1. Fetch artifacts
        let conversation_artifacts = fetch::fetch_artifacts(&self.client, "conversation", &since).await?;
        let session_artifacts = fetch::fetch_artifacts(&self.client, "session", &since).await?;

        if conversation_artifacts.len() < min_sessions {
            tracing::info!(
                found = conversation_artifacts.len(),
                min = min_sessions,
                "insufficient sessions for analysis"
            );
            return Ok(None);
        }

        // 2. Parse
        let mut conversations = Vec::new();
        for artifact in &conversation_artifacts {
            match parse::parse_conversation(&artifact.run_id, &artifact.data) {
                Ok(summary) => conversations.push(summary),
                Err(e) => tracing::warn!(run_id = %artifact.run_id, error = %e, "failed to parse conversation"),
            }
        }

        let mut sessions = Vec::new();
        for artifact in &session_artifacts {
            match parse::parse_session(&artifact.run_id, &artifact.data) {
                Ok(detail) => sessions.push(detail),
                Err(e) => tracing::warn!(run_id = %artifact.run_id, error = %e, "failed to parse session"),
            }
        }

        // 3. Build stage map from job runs (run_id -> stage)
        // For v1, we extract stage from the conversation data or default to "unknown"
        // TODO: fetch job run configs from Overseer for accurate stage mapping
        let stage_map: HashMap<String, String> = HashMap::new();

        // 4. Filter by scope
        // For Repo scope, we'd filter by repo_url from job run config
        // For v1, Global processes everything; Repo filtering requires job run API extension
        let scope_label = match &scope {
            AnalysisScope::Global => "global".to_string(),
            AnalysisScope::Repo(url) => url.clone(),
        };
        tracing::info!(
            scope = %scope_label,
            conversations = conversations.len(),
            sessions = sessions.len(),
            "starting analysis"
        );

        // 5. Compute metrics
        let cost_summary = metrics::build_cost_summary(&conversations, &stage_map)?;
        let tool_patterns = metrics::build_tool_patterns(&sessions)?;
        let context_pressure = metrics::build_context_pressure(&conversations, &sessions);
        let failure_analysis = metrics::build_failure_analysis(&conversations, &stage_map);

        // 6. Generate recommendations
        let recommendations = rules::generate_recommendations(
            &cost_summary,
            &tool_patterns,
            &context_pressure,
            &failure_analysis,
        );

        let now = Utc::now();
        Ok(Some(AnalysisReport {
            generated_at: now,
            scope,
            period_start: since,
            period_end: now,
            runs_analyzed: conversations.len(),
            cost_summary,
            tool_patterns,
            context_pressure,
            failure_analysis,
            recommendations,
        }))
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cd src/evolution && cargo check`
Expected: compiles cleanly

- [ ] **Step 3: Run all evolution tests**

Run: `cd src/evolution && cargo test`
Expected: all tests from tasks 2-5 pass

- [ ] **Step 4: Commit**

```bash
git add src/evolution/src/lib.rs
git commit -m "feat(evolution): wire up analyze() entrypoint"
```

---

## Task 7: Add Queen evolution config and trigger logic

**Files:**
- Modify: `src/queen/src/config.rs`
- Modify: `src/queen/src/main.rs`
- Modify: `src/queen/Cargo.toml`
- Modify: `src/queen/BUCK`
- Test: `src/queen/src/config.rs` (existing test module)

- [ ] **Step 1: Add EvolutionConfig to Queen config**

Add to `src/queen/src/config.rs`:

```rust
fn default_evolution_enabled() -> bool {
    false
}

fn default_min_sessions() -> usize {
    5
}

fn default_run_interval() -> usize {
    10
}

fn default_time_interval() -> String {
    "24h".to_string()
}

fn default_repo_run_interval() -> usize {
    5
}

fn default_evolution_definition() -> String {
    "evolve-from-analysis".to_string()
}

#[derive(Debug, Deserialize)]
pub struct EvolutionConfig {
    #[serde(default = "default_evolution_enabled")]
    pub enabled: bool,
    #[serde(default = "default_min_sessions")]
    pub min_sessions: usize,
    #[serde(default = "default_run_interval")]
    pub run_interval: usize,
    #[serde(default = "default_time_interval")]
    pub time_interval: String,
    #[serde(default = "default_repo_run_interval")]
    pub repo_run_interval: usize,
    #[serde(default = "default_evolution_definition")]
    pub drone_definition: String,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            enabled: default_evolution_enabled(),
            min_sessions: default_min_sessions(),
            run_interval: default_run_interval(),
            time_interval: default_time_interval(),
            repo_run_interval: default_repo_run_interval(),
            drone_definition: default_evolution_definition(),
        }
    }
}
```

Add to `Config` struct:

```rust
pub struct Config {
    pub queen: QueenConfig,
    #[serde(default)]
    pub creep: CreepConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub evolution: EvolutionConfig,
}
```

- [ ] **Step 2: Add evolution dependency to Queen**

In `src/queen/Cargo.toml`, add:
```toml
evolution = { path = "../evolution" }
chrono = { version = "0.4", features = ["serde"] }
```

In `src/queen/BUCK`, add to `QUEEN_DEPS`:
```python
"//src/evolution:evolution",
"//third-party:chrono",
```

- [ ] **Step 3: Add evolution actor to Queen main.rs**

Create `src/queen/src/actors/evolution.rs`:

```rust
// src/queen/src/actors/evolution.rs
use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use evolution::EvolutionChamber;
use evolution::report::AnalysisScope;
use nydus::NydusClient;
use tokio_util::sync::CancellationToken;

use crate::config::EvolutionConfig;

pub async fn run(
    client: NydusClient,
    config: EvolutionConfig,
    token: CancellationToken,
) {
    if !config.enabled {
        tracing::info!("evolution chamber disabled");
        return;
    }

    let chamber = EvolutionChamber::new(client.clone());
    let time_interval = crate::parse_duration(&config.time_interval)
        .unwrap_or(Duration::from_secs(86400));

    let mut last_analysis = Instant::now();
    let mut completed_since_last = 0usize;
    let mut last_analysis_time: DateTime<Utc> = Utc::now();

    let poll_interval = Duration::from_secs(60);

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("evolution actor shutting down");
                return;
            }
            _ = tokio::time::sleep(poll_interval) => {}
        }

        // Count completed runs since last analysis
        let runs = match client.list_runs(Some("completed")).await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "failed to poll completed runs for evolution");
                continue;
            }
        };
        completed_since_last = runs.len(); // simplified: count all completed

        let time_triggered = last_analysis.elapsed() >= time_interval;
        let count_triggered = completed_since_last >= config.run_interval;

        if !time_triggered && !count_triggered {
            continue;
        }

        tracing::info!(
            time_triggered,
            count_triggered,
            completed = completed_since_last,
            "evolution chamber triggered"
        );

        // Run global analysis
        match chamber.analyze(
            AnalysisScope::Global,
            last_analysis_time,
            config.min_sessions,
        ).await {
            Ok(Some(report)) => {
                tracing::info!(
                    runs = report.runs_analyzed,
                    recommendations = report.recommendations.len(),
                    "evolution analysis complete"
                );
                // Submit report as job task
                let report_json = serde_json::to_string_pretty(&report).unwrap_or_default();
                let task = format!(
                    "Analyze the following Evolution Chamber report and create GitHub issues for each high/medium severity recommendation.\n\nLabel issues with `evolution-chamber`.\n\n```json\n{}\n```",
                    report_json
                );
                if let Err(e) = client.submit_job(&config.drone_definition, &task, None).await {
                    tracing::warn!(error = %e, "failed to submit evolution report job");
                }
            }
            Ok(None) => {
                tracing::info!("evolution analysis skipped: insufficient data");
            }
            Err(e) => {
                tracing::warn!(error = %e, "evolution analysis failed");
            }
        }

        last_analysis = Instant::now();
        last_analysis_time = Utc::now();
        completed_since_last = 0;
    }
}
```

Add `pub mod evolution;` to `src/queen/src/actors/mod.rs`.

In `src/queen/src/main.rs`, after the supervisor spawn (around line 121), add:

```rust
// 8.5. Start Evolution actor
let evolution_client = client.clone();
let evolution_config = config.evolution;
let evolution_token = token.clone();
tokio::spawn(async move {
    actors::evolution::run(evolution_client, evolution_config, evolution_token).await;
});
```

- [ ] **Step 4: Check nydus has submit_job**

Verify `NydusClient::submit_job` exists. If not, check what method submits jobs and adjust the call in the evolution actor accordingly.

Run: `grep -n "submit_job\|start_job" src/nydus/src/client.rs`

- [ ] **Step 5: Add a config test**

In `src/queen/src/config.rs` tests, add:

```rust
#[test]
fn test_parse_evolution_config() {
    let f = write_toml(
        r#"
[queen]
name = "test"

[evolution]
enabled = true
min_sessions = 10
run_interval = 20
time_interval = "12h"
"#,
    );
    let config = Config::load(f.path()).unwrap();
    assert!(config.evolution.enabled);
    assert_eq!(config.evolution.min_sessions, 10);
    assert_eq!(config.evolution.run_interval, 20);
    assert_eq!(config.evolution.time_interval, "12h");
    assert_eq!(config.evolution.drone_definition, "evolve-from-analysis");
}

#[test]
fn test_evolution_defaults_disabled() {
    let f = write_toml(
        r#"
[queen]
name = "test"
"#,
    );
    let config = Config::load(f.path()).unwrap();
    assert!(!config.evolution.enabled);
    assert_eq!(config.evolution.min_sessions, 5);
}
```

- [ ] **Step 6: Run tests**

Run: `cd src/queen && cargo test`
Expected: PASS

- [ ] **Step 7: Run buckify**

Run: `./tools/buckify.sh`

- [ ] **Step 8: Commit**

```bash
git add src/queen/ src/evolution/
git commit -m "feat(queen): evolution chamber integration with configurable triggers"
```

---

## Task 8: Seed evolve-from-analysis job definition in Overseer

**Files:**
- Modify: `src/overseer/src/main.rs:92-118`
- Modify: `src/drones/claude/base/src/stages.rs` (add evolution stage CLAUDE.md)

- [ ] **Step 1: Add seed definition**

In `src/overseer/src/main.rs`, add to the `seed_definitions` array:

```rust
(
    "evolve-from-analysis",
    "Create problem specs from Evolution Chamber analysis report",
    serde_json::json!({ "drone_type": "claude-drone", "stage": "evolve" }),
),
```

- [ ] **Step 2: Add evolution stage to drone stages.rs**

In `src/drones/claude/base/src/stages.rs`, add a match arm for the `"evolve"` stage in `generate_claude_md()`. The CLAUDE.md should instruct the drone to:

```rust
"evolve" => Some(format!(r#"# Evolution Chamber Analysis

You are reviewing an Evolution Chamber analysis report. Your task is to create GitHub issues for actionable recommendations.

## Instructions

1. Read the analysis report provided in the task
2. For each recommendation with severity High or Medium:
   - Create a GitHub issue as a problem spec
   - Title: the recommendation title
   - Body: Include the detail, evidence, and your proposed approach
   - Label: `evolution-chamber`
3. Skip Low severity recommendations unless they have compelling evidence
4. Group related recommendations into a single issue when they share a root cause

## Output

Create the issues using `gh issue create`. Report what you created.
"#)),
```

- [ ] **Step 3: Verify Overseer compiles**

Run: `cd src/overseer && cargo check`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add src/overseer/src/main.rs src/drones/claude/base/src/stages.rs
git commit -m "feat(overseer): seed evolve-from-analysis job definition and drone stage"
```

---

## Task 9: End-to-end test with fixture data

**Files:**
- Create: `src/evolution/tests/integration.rs`
- Create: `src/evolution/tests/fixtures/` (test data)

- [ ] **Step 1: Create fixture conversation and session data**

Create `src/evolution/tests/fixtures/conversation.json`:
```json
{
  "total_cost_usd": 1.71,
  "num_turns": 74,
  "duration_ms": 501455,
  "subtype": "success",
  "session_id": "test-session-1",
  "modelUsage": {
    "claude-sonnet-4-6": {
      "inputTokens": 74,
      "outputTokens": 21740,
      "cacheReadInputTokens": 3838424,
      "cacheCreationInputTokens": 62451
    }
  }
}
```

Create `src/evolution/tests/fixtures/session.jsonl` — a minimal session with tool calls:
```jsonl
{"type":"user","message":{"role":"user","content":"implement the feature"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/src/main.rs"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"fn main() {}"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/src/main.rs","old_string":"fn main","new_string":"fn new_main"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"String not found"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/src/main.rs","old_string":"fn main","new_string":"fn new_main"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t3","is_error":true,"content":"String not found"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t4","name":"Edit","input":{"file_path":"/src/main.rs","old_string":"fn main","new_string":"fn new_main"}}]}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t4","content":"Edit applied"}]}}
```

- [ ] **Step 2: Write integration test parsing fixtures end-to-end**

```rust
// src/evolution/tests/integration.rs
use evolution::report::*;
use std::collections::HashMap;

#[test]
fn test_full_pipeline_with_fixtures() {
    let conversation_data = include_bytes!("fixtures/conversation.json");
    let session_data = include_bytes!("fixtures/session.jsonl");

    // Parse
    let conversation = evolution::parse::parse_conversation("run-fixture", conversation_data).unwrap();
    let session = evolution::parse::parse_session("run-fixture", session_data).unwrap();

    assert!(conversation.success);
    assert!(conversation.cost_usd > 1.0);
    assert_eq!(session.tool_calls.len(), 4);
    assert!(session.tool_calls[1].is_error); // Edit #1 failed
    assert!(session.tool_calls[2].is_error); // Edit #2 failed
    assert!(!session.tool_calls[3].is_error); // Edit #3 succeeded

    // Metrics
    let stage_map = HashMap::new();
    let cost = evolution::metrics::build_cost_summary(&[conversation.clone()], &stage_map).unwrap();
    let tools = evolution::metrics::build_tool_patterns(&[session.clone()]).unwrap();
    let context = evolution::metrics::build_context_pressure(&[conversation.clone()], &[session.clone()]);
    let failures = evolution::metrics::build_failure_analysis(&[conversation], &stage_map);

    assert!(cost.total_cost_usd > 1.0);
    assert_eq!(tools.call_counts["Edit"], 3);
    assert_eq!(tools.call_counts["Read"], 1);
    assert!(*tools.error_rates.get("Edit").unwrap() > 0.5);

    // Recommendations
    let recs = evolution::rules::generate_recommendations(&cost, &tools, &context, &failures);
    // Should flag Edit's high error rate
    assert!(recs.iter().any(|r| r.title.contains("Edit")));

    // Report serializes to JSON
    let report = AnalysisReport {
        generated_at: chrono::Utc::now(),
        scope: AnalysisScope::Global,
        period_start: chrono::Utc::now(),
        period_end: chrono::Utc::now(),
        runs_analyzed: 1,
        cost_summary: cost,
        tool_patterns: tools,
        context_pressure: context,
        failure_analysis: failures,
        recommendations: recs,
    };
    let json = serde_json::to_string_pretty(&report).unwrap();
    assert!(json.contains("Edit"));
}
```

- [ ] **Step 3: Make parse and metrics modules public for integration test access**

In `src/evolution/src/lib.rs`, change:
```rust
mod fetch;
mod metrics;
mod parse;
mod rules;
```
to:
```rust
pub mod fetch;
pub mod metrics;
pub mod parse;
pub mod rules;
```

- [ ] **Step 4: Run integration test**

Run: `cd src/evolution && cargo test --test integration`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/evolution/tests/ src/evolution/src/lib.rs
git commit -m "test(evolution): end-to-end integration test with fixture data"
```

---

Plan complete and saved to `docs/plans/2026-04-02-evolution-chamber-v1.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?
