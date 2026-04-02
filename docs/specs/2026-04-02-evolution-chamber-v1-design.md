# Evolution Chamber v1 Design

## Problem

Drone sessions generate rich telemetry (conversation summaries, full session JSONL) but nothing analyzes it. We can't see which stages cost the most, where drones retry excessively, which tool patterns waste context, or what systemic improvements would have the highest impact. Without feedback, the dev loop can't improve itself.

## Solution

A Rust crate that performs heuristic analysis on aggregated drone session data, producing structured reports that feed into the dev loop as problem specs for a Claude drone to action.

## Non-Goals

- Evolution Chamber is **not** a drone. No Claude Code, no LLM calls.
- No real-time analysis. Batch processing of completed runs.
- No UI. Output is a JSON report consumed programmatically.

## Architecture

```
Overseer (artifacts API)
    |
    | GET /api/artifacts?artifact_type=session&since=...
    | GET /api/artifacts?artifact_type=conversation&since=...
    |
    v
Evolution Chamber (Rust crate, invoked by Queen)
    |
    | 1. Fetch artifacts via nydus
    | 2. Decompress + parse session JSONL
    | 3. Aggregate with Polars
    | 4. Apply heuristic rules
    | 5. Produce structured report
    |
    v
Queen
    |
    | Submits report as task to Claude drone
    | via Overseer submit_job
    |
    v
Claude Drone
    |
    | Reads analysis report
    | Proposes: tooling, skills, shared services,
    |   context optimizations, workflow improvements
    | Creates GitHub issues as problem specs
    |
    v
Dev Loop (problem specs feed back in)
```

## Crate: `src/evolution/`

New workspace member. Dependencies: `nydus`, `polars`, `serde`, `serde_json`, `flate2`, `anyhow`, `chrono`.

### Public API

```rust
pub enum AnalysisScope {
    /// All sessions across all repos.
    Global,
    /// Sessions for a specific repository.
    Repo(String), // repo_url
}

pub struct EvolutionChamber {
    client: NydusClient,
}

impl EvolutionChamber {
    pub fn new(client: NydusClient) -> Self;

    /// Run analysis on sessions since the given timestamp.
    /// Returns None if insufficient data (< min_sessions).
    pub async fn analyze(
        &self,
        scope: AnalysisScope,
        since: DateTime<Utc>,
        min_sessions: usize,
    ) -> Result<Option<AnalysisReport>>;
}
```

### Data Pipeline

1. **Fetch** — Pull `conversation` and `session` artifacts from Overseer via nydus, filtered by `since` timestamp.
2. **Decompress** — Gunzip artifact blobs.
3. **Parse conversations** — Extract per-run summary: cost, tokens, turns, duration, model, exit code, stage (from job run config).
4. **Parse sessions** — Extract per-run detail: tool call counts by name, tool errors, retry sequences, message sizes, context compression events.
5. **Build DataFrames** — Load into Polars for aggregation.
6. **Analyze** — Apply heuristic rules (see below).
7. **Report** — Produce `AnalysisReport`.

### Session JSONL Parsing

Each line is a JSON object with a `type` field. Relevant types:

| Type | Extract |
|------|---------|
| `assistant` | Tool calls (name, input size), text responses (token proxy via length) |
| `user` | Message size, frequency |
| `system` | Context compression events, system prompt changes |

Tool calls are in `message.content[]` where `type == "tool_use"`. Tool results follow as `tool_result` blocks.

### Heuristic Rules

**Cost Analysis:**
- Total cost per stage type (spec, plan, implement, review)
- Cost per run, sorted descending
- Cost trend over time (increasing = concern)

**Tool Patterns:**
- Tool call frequency distribution (which tools dominate)
- Retry detection: same tool + same first argument within N turns
- Tool error rate: tool_result with `is_error: true`
- High-cost tools: tools that precede large context growth

**Context Pressure:**
- Turns-to-completion distribution
- Runs that hit context compression (system messages with compression markers)
- Cache read vs creation ratio (from conversation summary)

**Failure Analysis:**
- Exit code distribution
- Runs where drone failed but PR was expected
- Common error patterns in failed runs

### Report Schema

```rust
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

pub struct CostSummary {
    pub total_cost_usd: f64,
    pub cost_by_stage: HashMap<String, f64>,
    pub cost_trend: Trend, // Increasing, Stable, Decreasing
    pub highest_cost_runs: Vec<RunCostEntry>,
}

pub struct ToolPatterns {
    pub call_counts: HashMap<String, u64>,
    pub retry_sequences: Vec<RetrySequence>,
    pub error_rates: HashMap<String, f64>,
    pub top_context_consumers: Vec<String>,
}

pub struct ContextPressure {
    pub avg_turns: f64,
    pub median_turns: f64,
    pub compression_events: usize,
    pub avg_cache_hit_ratio: f64,
}

pub struct FailureAnalysis {
    pub failure_rate: f64,
    pub failure_by_stage: HashMap<String, f64>,
    pub common_errors: Vec<ErrorPattern>,
}

pub struct Recommendation {
    pub category: RecommendationCategory,
    pub severity: Severity, // High, Medium, Low
    pub title: String,
    pub detail: String,
    pub evidence: String, // specific data points
}

pub enum RecommendationCategory {
    Tooling,       // new tools, tool improvements
    Skills,        // new skills, skill configuration
    SharedService, // new MCP servers, shared infrastructure
    Context,       // context reduction, caching improvements
    Workflow,      // pipeline stage changes, prompt improvements
}
```

### Example Recommendations

**Global (platform-wide):**
- "Tool `Bash` called 45 times across 10 runs for git operations. Consider a dedicated git MCP tool."
- "Retry rate for `Edit` is 23% — mostly 'string not found'. Drones may need better file reading before editing."
- "Review stage costs $0.12 avg vs implement at $1.70. Review prompt may be over-constrained."

**Repo-scoped:**
- "3 of 5 implement runs on kerrigan hit context compression. Review stage prompt size or split large tasks."
- "8 runs on kerrigan spent 30% of tool calls on Buck2 build commands. Consider a build-status MCP tool or pre-build hook."
- "kerrigan implement runs average 74 turns vs 40 global. CLAUDE.md may need clearer task decomposition guidance."

## Queen Integration

### Analysis Scope

Evolution Chamber supports two scopes:

- **Repo-scoped** — analyzes sessions for a single repository (filtered by `repo_url` in job run config). Produces improvements specific to that project: custom skills, repo-specific CLAUDE.md tuning, project-aware tooling.
- **Global** — analyzes all sessions across all repos. Produces generic improvements: platform-wide tooling, shared MCP servers, drone prompt optimization, stage configuration.

Both scopes can run independently. Repo-scoped analysis fires per-repo after enough runs accumulate. Global analysis fires on the overall cadence.

### Trigger Policy

Configured in `hatchery.toml`:

```toml
[evolution]
enabled = true
min_sessions = 5           # minimum completed runs before analysis
run_interval = 10          # analyze every N completed runs (global)
time_interval = "24h"      # or analyze on this schedule, whichever comes first
repo_run_interval = 5      # analyze per-repo every N completed runs for that repo
drone_definition = "evolve-from-analysis"  # job definition for the Claude drone
```

### Flow

1. Queen tracks completed run count since last analysis.
2. When threshold hit (count or time), Queen invokes `EvolutionChamber::analyze()`.
3. If report is produced (enough data, findings exist), Queen submits a job:
   - Definition: `evolve-from-analysis` (seeded on Overseer startup)
   - Task: the serialized `AnalysisReport` as JSON
   - Config: `{ "stage": "spec" }` — drone generates problem specs
4. Claude drone reads the report, creates GitHub issues for actionable items.
5. Queen resets counters.

### Job Definition: `evolve-from-analysis`

Seeded on Overseer startup alongside existing definitions. Stage-specific CLAUDE.md instructs the drone to:
- Read the analysis report from the task
- For each high/medium severity recommendation, create a GitHub issue as a problem spec
- Issue title: recommendation title
- Issue body: detail + evidence + proposed approach
- Label: `evolution-chamber`

## Crate Layout

```
src/evolution/
  Cargo.toml
  BUCK
  src/
    lib.rs          # EvolutionChamber, analyze()
    fetch.rs        # artifact fetching + decompression
    parse.rs        # conversation + session JSONL parsing
    metrics.rs      # Polars aggregation, DataFrame construction
    rules.rs        # heuristic rules, recommendation generation
    report.rs       # AnalysisReport types
```

## Dependencies

- `nydus` — Overseer HTTP client (fetch artifacts, submit jobs)
- `polars` — DataFrame analysis (aggregation, statistics)
- `flate2` — gzip decompression
- `serde` + `serde_json` — parsing + report serialization
- `chrono` — timestamps
- `anyhow` — error handling

## Testing

- Unit tests with fixture JSONL data (synthetic sessions)
- Heuristic rule tests with known patterns (e.g., feed it a session with 10 Edit retries, verify recommendation)
- Integration test with in-memory Overseer (existing test pattern)

## Future (v2)

- Evolution Chamber submits implementation jobs directly (not just specs)
- Trend analysis across multiple analysis runs
- Per-repo / per-drone-type breakdown
- Dashboard integration (metrics endpoint)
