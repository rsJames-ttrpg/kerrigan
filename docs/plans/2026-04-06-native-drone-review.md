# Native Drone Comprehensive Review

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Review the entire native drone implementation against all design specs, producing a structured review artifact.

**Architecture:** Spec-by-spec compliance review across `src/drones/native/`, `src/runtime/`, and related crates. Each task reviews one spec's requirements against the implementation, filing issues for gaps.

**Tech Stack:** gh CLI, Overseer MCP (store_artifact)

---

### Task 1: Review Spec 00 — Overview & Architecture

**Files:**
- Read: `docs/specs/native-drone/00-overview.md`
- Read: `src/drones/native/src/main.rs`
- Read: `src/drones/native/src/drone.rs`
- Read: `src/drones/native/BUCK`
- Read: `src/drones/native/Cargo.toml`
- Read: `src/runtime/src/lib.rs`

- [ ] **Step 1: Read the overview spec**

Read `docs/specs/native-drone/00-overview.md` in full. Note every stated requirement and architectural decision.

- [ ] **Step 2: Verify crate structure matches spec**

The spec defines two crates: `src/runtime/` (generic agent engine) and `src/drones/native/` (kerrigan drone). Verify both exist, have correct BUCK targets, and the dependency direction is correct (native depends on runtime, not vice versa).

- [ ] **Step 3: Verify the six problems are addressed**

The spec lists six problems with the CLI wrapper approach. For each, verify the native drone addresses it:
1. Black box agent loop → native conversation loop with tool control
2. Brittle observability → structured event stream
3. No context management → checkpoint-based compaction
4. Git workflow by prayer → enforced git workflow policies
5. Vendor lock-in → multi-provider API client
6. Distribution burden → no embedded CLI binary

- [ ] **Step 4: Document findings**

Write a markdown section summarizing spec 00 compliance. Note any gaps, partial implementations, or deviations.

- [ ] **Step 5: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 00 overview compliance check"
```

---

### Task 2: Review Spec 01 — Runtime API Client

**Files:**
- Read: `docs/specs/native-drone/01-runtime-api-client.md`
- Read: `src/runtime/src/api/` (all files)
- Read: `src/runtime/src/api/client.rs`
- Read: `src/runtime/src/api/types.rs`

- [ ] **Step 1: Read spec 01**

Read `docs/specs/native-drone/01-runtime-api-client.md` in full. Note every requirement: provider abstraction, streaming, token counting, retry logic, rate limiting.

- [ ] **Step 2: Verify provider abstraction**

The spec requires a `Provider` trait with implementations for Anthropic, OpenAI-compatible, and Ollama. Check `src/runtime/src/api/` for:
- Trait definition with `send_message` / `stream_message`
- At least Anthropic provider implementation
- Provider selection from config

- [ ] **Step 3: Verify streaming and token tracking**

Check for streaming response support and token usage tracking in API responses.

- [ ] **Step 4: Verify retry and rate limiting**

Check for exponential backoff retry logic and rate limit handling (429 responses).

- [ ] **Step 5: Document findings**

Append spec 01 findings to the review document.

- [ ] **Step 6: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 01 API client compliance check"
```

---

### Task 3: Review Spec 02 — Runtime Tool System

**Files:**
- Read: `docs/specs/native-drone/02-runtime-tool-system.md`
- Read: `src/runtime/src/tools/` (all files)

- [ ] **Step 1: Read spec 02**

Read `docs/specs/native-drone/02-runtime-tool-system.md` in full. Note requirements: tool registry, built-in tools (Bash, Read, Write, Edit, Glob, Grep), MCP tool proxy, external binary tools, sandboxing.

- [ ] **Step 2: Verify tool registry**

Check for a `ToolRegistry` or equivalent that supports:
- Registration of built-in tools
- MCP server tool proxying
- External binary tools
- Tool allow/deny lists

- [ ] **Step 3: Verify built-in tool implementations**

Check that Bash, Read, Write, Edit, Glob, Grep are implemented as built-in tools with proper sandboxing (path restrictions, command filtering).

- [ ] **Step 4: Verify MCP integration**

Check for MCP client that can connect to servers and proxy tool calls.

- [ ] **Step 5: Document findings**

Append spec 02 findings to the review document.

- [ ] **Step 6: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 02 tool system compliance check"
```

---

### Task 4: Review Spec 03 — Runtime Conversation Loop

**Files:**
- Read: `docs/specs/native-drone/03-runtime-conversation-loop.md`
- Read: `src/runtime/src/loop.rs` or `src/runtime/src/conversation/` (find the conversation loop)

- [ ] **Step 1: Read spec 03**

Read `docs/specs/native-drone/03-runtime-conversation-loop.md` in full. Note requirements: turn-based loop, tool dispatch, checkpoint creation, compaction strategy, event emission, max iteration limits.

- [ ] **Step 2: Verify conversation loop**

Find the conversation loop implementation. Check for:
- Turn-based loop calling API then dispatching tool uses
- Stop condition handling (end_turn, max iterations, timeout)
- Event emission on each turn

- [ ] **Step 3: Verify checkpoint and compaction**

Check for checkpoint creation and context compaction when approaching token limits.

- [ ] **Step 4: Document findings**

Append spec 03 findings to the review document.

- [ ] **Step 5: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 03 conversation loop compliance check"
```

---

### Task 5: Review Spec 04 — Drone Pipeline

**Files:**
- Read: `docs/specs/native-drone/04-drone-pipeline.md`
- Read: `src/drones/native/src/pipeline.rs`
- Read: `src/drones/native/src/exit_conditions.rs`
- Read: `src/drones/native/src/git_workflow.rs`
- Read: `src/drones/native/src/health.rs`
- Read: `src/drones/native/src/orchestrator/` (all files)

- [ ] **Step 1: Read spec 04**

Read `docs/specs/native-drone/04-drone-pipeline.md` in full. Note requirements: stage state machine, stage-specific configs, environment health checks, orchestrator, git workflow enforcement, exit conditions.

- [ ] **Step 2: Verify stage state machine**

Check `pipeline.rs` for all 6 stages (Spec, Plan, Implement, Review, Evolve, Freeform) with correct default configs per stage.

- [ ] **Step 3: Verify health checks**

Check `health.rs` for stage-specific health checks that run before execution.

- [ ] **Step 4: Verify orchestrator**

Check `src/drones/native/src/orchestrator/` for plan parsing, DAG scheduling, parallel execution. Cross-reference with `docs/specs/2026-04-05-orchestrator-integration.md`.

- [ ] **Step 5: Verify git workflow enforcement**

Check `git_workflow.rs` for branch naming, force push denial, protected paths, operation allow-lists.

- [ ] **Step 6: Verify exit conditions**

Check `exit_conditions.rs` for FileCreated, TestsPassing, PrCreated, ArtifactStored, Custom conditions.

- [ ] **Step 7: Document findings**

Append spec 04 findings to the review document.

- [ ] **Step 8: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 04 pipeline compliance check"
```

---

### Task 6: Review Spec 05 — Config and Prompts

**Files:**
- Read: `docs/specs/native-drone/05-drone-config-and-prompts.md`
- Read: `src/drones/native/src/config.rs`
- Read: `src/drones/native/src/resolve.rs`
- Read: `src/drones/native/src/prompt.rs`
- Read: `src/drones/native/src/cache.rs`

- [ ] **Step 1: Read spec 05**

Read `docs/specs/native-drone/05-drone-config-and-prompts.md` in full. Note requirements: config hierarchy, drone.toml structure, cache strategy, prompt construction with priority sections and token budgeting.

- [ ] **Step 2: Verify config hierarchy**

Check `config.rs` and `resolve.rs` for the merge chain: defaults → drone.toml → job config → stage defaults. Verify override precedence is correct.

- [ ] **Step 3: Verify cache strategy**

Check `cache.rs` for bare git repo caching, worktree management, and LRU-bounded tool cache.

- [ ] **Step 4: Verify prompt construction**

Check `prompt.rs` for priority-based sections, token budgeting, and stage-specific prompt generation.

- [ ] **Step 5: Document findings**

Append spec 05 findings to the review document.

- [ ] **Step 6: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 05 config and prompts compliance check"
```

---

### Task 7: Review Spec 06 — Queen Integration

**Files:**
- Read: `docs/specs/native-drone/06-drone-queen-integration.md`
- Read: `src/drones/native/src/event_bridge.rs`
- Read: `src/drones/native/src/drone.rs`
- Read: `src/drone-sdk/src/protocol.rs`

- [ ] **Step 1: Read spec 06**

Read `docs/specs/native-drone/06-drone-queen-integration.md` in full. Note requirements: event bridge mapping, extended DroneEvent types, wire format, NativeDrone lifecycle.

- [ ] **Step 2: Verify event bridge**

Check `event_bridge.rs` for mapping of all RuntimeEvent variants to DroneMessage types. Verify all DroneEvent variants from the spec are implemented.

- [ ] **Step 3: Verify DroneRunner implementation**

Check `drone.rs` for setup/execute/teardown lifecycle matching spec requirements. Verify job config parsing, secret handling, git credential setup.

- [ ] **Step 4: Verify protocol extensions**

Check `src/drone-sdk/src/protocol.rs` for the extended DroneEvent enum with all variants from the spec.

- [ ] **Step 5: Document findings**

Append spec 06 findings to the review document.

- [ ] **Step 6: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: spec 06 queen integration compliance check"
```

---

### Task 8: Cross-cutting Concerns and Test Coverage

**Files:**
- Read: all test files in `src/drones/native/src/` (look for `#[cfg(test)]` modules)
- Read: all test files in `src/runtime/src/`

- [ ] **Step 1: Inventory test coverage**

Run `cargo test --no-run` in `src/drones/native/` and `src/runtime/` to list all test binaries. Then run `cargo test -- --list` to enumerate all test names. Count tests per module.

- [ ] **Step 2: Identify coverage gaps**

For each module, compare test names against the module's public API. Flag any public functions or important code paths without tests.

- [ ] **Step 3: Check error handling patterns**

Grep for `unwrap()`, `expect()`, and bare `?` in non-test code. Flag any that could panic in production.

- [ ] **Step 4: Check for TODO/FIXME/HACK markers**

```bash
grep -rn 'TODO\|FIXME\|HACK\|XXX\|UNIMPLEMENTED' src/drones/native/src/ src/runtime/src/
```

- [ ] **Step 5: Document findings and create final summary**

Complete the review document with:
- Overall spec compliance score (per spec: full / partial / missing)
- Critical gaps that block production use
- Recommended follow-up work (as GitHub issues)

- [ ] **Step 6: Store review artifact**

Use Overseer MCP `store_artifact` to store the completed review document.

- [ ] **Step 7: Commit**

```bash
git add docs/reviews/2026-04-06-native-drone-review.md
git commit -m "review: complete native drone spec compliance review"
```

- [ ] **Step 8: Create PR**

```bash
gh pr create --title "review: native drone spec compliance" \
  --body "Comprehensive review of native drone implementation against all 7 specs (00-06) plus orchestrator integration spec. Produced by drone review run."
```
