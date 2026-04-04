# Native Drone: Overview

**Date:** 2026-04-04
**Status:** Draft

## Problem

The current Claude drone (`src/drones/claude/base/`) is a thin wrapper around the official Claude CLI binary. It embeds the CLI at compile time, spawns it as a subprocess with `--dangerously-skip-permissions`, and scrapes stdout/stderr for results. This creates several problems:

1. **Black box agent loop** — no control over tool execution, context management, or conversation flow. Workflow structure (brainstorming, planning, TDD) is injected via skill markdown files and relies on the LLM following them.
2. **Brittle observability** — health checks rely on stderr liveness. No structured event stream. Queen can't distinguish "thinking" from "stuck."
3. **No context management** — can't control compaction, can't checkpoint, can't scope tool access per stage. Long implement runs hit context limits with no recovery.
4. **Git workflow by prayer** — branching, commit messages, and PR creation depend on the LLM running the right commands in the right order. The `ensure_pr()` safety net exists because this regularly fails.
5. **Vendor lock-in** — embedded CLI binary ties deployment to Anthropic's release cadence. Can't use local models (Ollama) or alternative providers.
6. **Distribution burden** — vendoring a ~100MB binary into every drone build is wasteful and fragile.

## Solution

Replace the CLI wrapper with a native Rust agent runtime that talks directly to LLM APIs. Two new crates:

- **`src/runtime/`** — generic agent engine library. Multi-provider API client, tool registry with built-in + MCP + external binary tools, conversation loop with checkpoint-based compaction, typed event stream. No knowledge of drones, Queen, or Overseer.
- **`src/drones/native/`** — kerrigan drone binary. Implements `DroneRunner` from drone-sdk, adds pipeline state machine (spec/plan/implement/review/evolve/freeform), task orchestrator with sub-agent coordination, enforced git workflow, and `drone.toml` configuration.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Queen (process manager)              │
│  Spawns drone binary, communicates via JSON-line stdio  │
└──────────────┬──────────────────────────┬───────────────┘
               │ stdin (Job, AuthResponse) │ stdout (Event, Result)
               ▼                          ▲
┌─────────────────────────────────────────────────────────┐
│                 Native Drone (src/drones/native/)       │
│                                                         │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │  Pipeline    │  │ Orchestrator │  │ Git Workflow  │  │
│  │  State       │  │ (sub-agents, │  │ (enforced     │  │
│  │  Machine     │  │  task queue) │  │  branching,   │  │
│  │             │  │              │  │  commits, PR)  │  │
│  └──────┬──────┘  └──────┬───────┘  └───────┬────────┘  │
│         │                │                   │          │
│  ┌──────▼───────────────▼───────────────────▼─────────┐ │
│  │              Runtime (src/runtime/)                │ │
│  │                                                    │ │
│  │  ┌───────────┐ ┌────────────┐ ┌──────────────────┐ │ │
│  │  │ API Client│ │ Tool       │ │ Conversation     │ │ │
│  │  │ Anthropic │ │ Registry   │ │ Loop + Compaction│ │ │
│  │  │ OpenAI    │ │ Built-in   │ │ + Checkpoints    │ │ │
│  │  │ Ollama    │ │ MCP        │ │                  │ │ │
│  │  │           │ │ External   │ │                  │ │ │
│  │  └───────────┘ └────────────┘ └──────────────────┘ │ │
│  │                                                    │ │
│  │  ┌──────────────────────────────────────────────┐  │ │
│  │  │ Event Stream → EventSink trait               │  │ │
│  │  └──────────────────────────────────────────────┘  │ │
│  └────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

## Design Principles

1. **Runtime is generic** — no kerrigan-specific knowledge. Reusable for the kerrigan CLI, testing harnesses, or other drone types.
2. **Drone is opinionated** — pipeline structure, git policy, prompt patterns. This is where kerrigan's development methodology lives.
3. **Prompts are code** — stage instructions generated in Rust from superpowers-derived patterns, not injected markdown files. Auto-generated tool guides stay in sync with the registry.
4. **Tools are typed** — git operations, test execution, file ops all have structured inputs and outputs. The LLM never runs raw git commands.
5. **Markdown-first output** — tool results default to markdown for context efficiency. JSON/raw available when needed.
6. **Checkpoint, don't pray** — context compaction creates Overseer artifacts with full state snapshots. Fresh context references prior work by artifact ID.
7. **Single reporting channel** — drone reports status/results to Queen via stdio only. Queen handles fan-out to Overseer. The drone may have direct MCP connections (e.g., Overseer) for tool use during execution, but all lifecycle events flow through Queen.
8. **Freeform fallback** — unknown job types get a capable agent with full tool access. Structured stages are progressive specialization, not a requirement.

## Sub-Specs

| Spec | Covers |
|------|--------|
| [01-runtime-api-client](01-runtime-api-client.md) | Multi-provider API client, streaming, auth |
| [02-runtime-tool-system](02-runtime-tool-system.md) | Tool trait, registry, built-in tools, MCP, external binaries |
| [03-runtime-conversation-loop](03-runtime-conversation-loop.md) | Agent loop, session model, compaction, checkpoints, events |
| [04-drone-pipeline](04-drone-pipeline.md) | Stage state machine, orchestrator, sub-agents, git workflow |
| [05-drone-config-and-prompts](05-drone-config-and-prompts.md) | drone.toml, config hierarchy, prompt construction |
| [06-drone-queen-integration](06-drone-queen-integration.md) | Event bridge, extended protocol, health/liveness |

## Migration Path

The new drone lives at `src/drones/native/` alongside the existing `src/drones/claude/base/`. Both implement `DroneRunner`. Queen's supervisor spawns whichever binary the job definition specifies. This allows:

1. Build and test the runtime crate independently
2. Build the native drone, test with freeform jobs against Ollama
3. Port stages one at a time (start with implement — highest pain)
4. Run both drones in parallel during transition
5. Retire the Claude CLI drone when all stages are ported
