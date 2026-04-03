# Manual Evolution Trigger (`kerrigan evolve`)

## Problem

The Evolution Chamber is only triggerable automatically via Queen's evolution actor,
which requires either N completed runs or a time interval to elapse. During fast
iteration and deployment cycles, artifacts accumulate but never reach the automatic
trigger thresholds before the operator wants analysis. There is no way to run evolution
analysis on demand.

Additionally, the recovery logic in the evolution actor has a bug: when no prior
evolution-report artifacts exist, `recover_last_analysis_time` falls back to
`Utc::now()`, making all pre-existing artifacts invisible to the first analysis.

## Solution

Add a `kerrigan evolve` CLI command that runs the evolution analysis pipeline
client-side and fix the Queen actor recovery bug.

## CLI Interface

```
kerrigan evolve [OPTIONS]

Options:
  --since <TIMESTAMP>   Only analyze artifacts after this time (RFC 3339).
                        Default: all time (epoch)
  --min-sessions <N>    Minimum sessions required for analysis (default: 5)
  --submit              Also submit an evolve-from-analysis job with the report
  --json                Output raw JSON instead of formatted report
```

### Default behavior

1. Fetch all conversation and session artifacts from Overseer (since epoch)
2. Run analysis pipeline: parse -> metrics -> rules -> report
3. Print formatted report to terminal
4. Store report as `evolution-report-{timestamp}` artifact in Overseer

### With `--submit`

After storing the report artifact, resolve the `evolve-from-analysis` job definition
and start a run with the report as task input. This matches what the Queen actor does
automatically.

### With `--json`

Print the raw `AnalysisReport` JSON to stdout instead of the formatted display.
Useful for piping to other tools or inspecting the full data structure.

### With `--since`

Narrow the analysis window. Accepts RFC 3339 timestamps (e.g. `2026-04-02T00:00:00Z`).
Without this flag, all artifacts are analyzed regardless of age.

## Report Display

Formatted terminal output with colored severity tags:

```
Evolution Report (2026-04-03T12:00:00Z)
Scope: global | Period: all time -> now | Runs analyzed: 5

Cost Summary
  Total: $X.XX | Trend: Stable
  Top runs: <run-id> $X.XX, ...

Tool Patterns
  Error rates: <tool> XX% (N/M calls)
  Retry sequences: <tool> retried N times on <target>
  Top consumers: <tool> (N calls, XX%)

Context Pressure
  Avg turns: N | Median turns: N
  Compression events: N
  Cache hit ratio: XX%

Failure Analysis
  Overall: XX% (N/M runs)
  By stage: <stage> XX%

Recommendations
  [HIGH] Title — detail (evidence)
  [MED]  Title — detail (evidence)
  [LOW]  Title — detail (evidence)

Report stored: <artifact-id>
```

## Changes

### `src/kerrigan/` — add evolve command

- **Cargo.toml**: add `evolution` and `chrono` dependencies
- **BUCK**: add `//src/evolution:evolution` and `//third-party:chrono` deps
- **src/main.rs**: add `Evolve` variant to `Command` enum, `cmd_evolve` handler
- **src/display.rs**: add `print_evolution_report` for formatted terminal output

`cmd_evolve` implementation:
1. Parse `--since` to `DateTime<Utc>` (default: `DateTime::<Utc>::MIN_UTC`)
2. Create `EvolutionChamber::new(client.clone())`
3. Call `chamber.analyze(AnalysisScope::Global, since, min_sessions)`
4. Handle `Ok(None)` -> "Insufficient data" message with artifact count hint
5. Handle `Ok(Some(report))`:
   - If `--json`: serialize and print
   - Else: call `print_evolution_report(&report)`
   - Store artifact via `client.store_artifact()`
   - If `--submit`: resolve definition, start run, print run ID
6. Handle `Err` -> print error

### `src/queen/src/actors/evolution.rs` — fix recovery bug

Line 193: change `Utc::now()` fallback to `DateTime::<Utc>::MIN_UTC` when no prior
evolution reports exist. Same fix for the error fallback on line 198.

This ensures the first automatic analysis catches all historical artifacts rather than
only those created after Queen boots.

## Dependencies

The `kerrigan` binary gains a dependency on the `evolution` crate (already a workspace
member). The `evolution` crate depends on `nydus` which kerrigan already has. The only
new transitive dep is `chrono` (already in the workspace via other crates).

No new third-party crates required.
