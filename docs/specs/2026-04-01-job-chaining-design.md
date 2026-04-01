# Job Chaining

**Date:** 2026-04-01
**Roadmap item:** #7 — Job chaining (last item to dogfooding)

## Context

The dev loop has four stages (spec → plan → implement → review) with job definitions for each (#6). Currently each stage must be triggered manually. This spec adds automatic pipeline advancement: when a stage completes, Overseer creates the next stage's run, respecting human approval gates.

## Pipeline Definition

Hardcoded in Overseer. One pipeline, four stages:

| Stage | Next | Gate before next? |
|-------|------|-------------------|
| spec | plan | yes |
| plan | implement | yes |
| implement | review | no |
| review | (end) | — |

**Gated transitions** (spec→plan, plan→implement): Operator must merge the PR and run `kerrigan approve` before the next stage starts. The operator is trusted — if they approve, they've reviewed and merged.

**Non-gated transitions** (implement→review): Overseer automatically creates the next run when the current one completes.

## Run Hierarchy

Runs are linked via `parent_id` (existing field on job_runs):

```
spec run (root, parent_id = None)
 └→ plan run (parent_id = spec_run_id)
     └→ implement run (parent_id = plan_run_id)
         └→ review run (parent_id = implement_run_id)
```

## Pipeline Advancement Logic

In `JobService`, when `update_job_run` sets status to `completed`:

1. Fetch the completed run's definition
2. Read `config.stage` — if absent, not a pipeline run, do nothing
3. Look up next stage in the hardcoded pipeline
4. If next stage is gated → do nothing (wait for `kerrigan approve`)
5. If next stage is not gated → create the next run automatically

### Creating the next run

Overseer creates a new job run with:
- `definition_id`: the next stage's definition (looked up by name, e.g., `plan-from-spec`)
- `parent_id`: the completed run's ID
- `triggered_by`: `"pipeline"` (distinguishes from operator-triggered runs)
- `config_overrides`: context forwarded from the completed run

### Context forwarding

**Gated transitions** (PR was merged, next stage clones main):
- `repo_url`: copied from parent's config
- `secrets`: copied from parent's config
- No `branch` — clones main (the merged PR is there)

**Non-gated transitions** (implement→review, PR not yet merged):
- `repo_url`: copied from parent's config
- `secrets`: copied from parent's config
- `branch`: from parent's `result.git_refs.branch`
- `pr_url`: from parent's `result.git_refs.pr_url`

## `kerrigan approve` Changes

Currently calls `update_run(id, "running", ...)`. New behavior:

Calls a new endpoint `POST /api/jobs/runs/{id}/advance` which:
1. Verifies the run status is `completed`
2. Reads the run's definition to determine the current stage
3. Looks up the next stage
4. Creates the next run with parent_id and forwarded context
5. Assigns to an available hatchery (Overseer queries hatcheries for one with capacity)
6. Returns the newly created run

The `kerrigan approve` command calls this endpoint and prints the new run ID.

## New Overseer Endpoint

`POST /api/jobs/runs/{id}/advance`

Response: the newly created job run (JSON).

Errors:
- 404: run not found
- 400: run not completed, or not a pipeline stage, or no next stage (review is terminal)

## Hatchery Assignment

The advance endpoint needs to assign the new run to a hatchery. Overseer already has `HatcheryService` with `list` and `assign_job`. The advance logic:
1. List online hatcheries
2. Pick first with `active_drones < max_concurrency`
3. Assign the new run
4. If no hatchery available, the run stays pending — Queen's poller will find and assign it when capacity opens up (existing behavior)

## `kerrigan status` Enhancement

`kerrigan status <run-id>` should show the pipeline chain — walk `parent_id` up and children down to show the full pipeline:

```
Pipeline:
  ✓ spec (abc123) — completed
  ✓ plan (def456) — completed
  → implement (ghi789) — running
    review — pending
```

This requires a new query: find runs where `parent_id = this_run_id` (children). Walk both directions.

## Notification

When a gated stage completes, the operator needs to know. Queen already has a notification trait with `QueenEvent::DroneCompleted`. The notifier should indicate that approval is needed. No new notification infrastructure — just enhance the log message to say "awaiting approval, run `kerrigan approve <id>`."

## Out of Scope

- Pipeline definitions as data (future — currently hardcoded)
- Multiple pipelines
- Partial pipeline runs (e.g., skip spec, start at plan)
- Automatic PR merge on approval
- Evolution Chamber integration
