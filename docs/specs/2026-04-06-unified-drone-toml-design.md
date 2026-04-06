# Unified DroneToml in drone-sdk

## Problem

`drone.toml` is the per-repo config that tells drones how to behave for a project. Currently there are two separate implementations:

1. **SDK's `DroneToml`** (`src/drone-sdk/src/drone_toml.rs`) — has `git` (with identity), `setup`, `prompts`. Used by the claude drone.
2. **Native drone's `DroneConfig`** (`src/drones/native/src/config.rs`) — has `provider`, `runtime`, `cache`, `git` (without identity), `tools`, `mcp`, `environment`, `orchestrator`, `health_checks`. Not using the SDK at all.

The `GitSection` is duplicated between them, with the SDK version being the superset (has `identity`). The native drone's version is missing `identity`, `setup`, and `prompts`.

## Design

One `DroneToml` struct in the SDK. All sections. Both drones use it. Each drone uses the fields it cares about.

### DroneToml struct (SDK)

```rust
pub struct DroneToml {
    pub provider: Option<ProviderSection>,  // optional — claude drone has its own provider
    pub runtime: RuntimeSection,
    pub cache: CacheSection,
    pub git: GitSection,          // existing SDK version (superset, has identity)
    pub setup: SetupSection,      // existing SDK
    pub prompts: PromptsSection,  // existing SDK
    pub tools: ToolsSection,
    pub mcp: HashMap<String, McpSection>,
    pub environment: EnvironmentSection,
    pub orchestrator: OrchestratorSection,
    pub health_checks: Vec<CustomHealthCheck>,
}
```

All sections `#[serde(default)]` so a minimal/empty drone.toml still works. `provider` is `Option` since the claude drone doesn't need it (it gets its provider from the Claude CLI), but a repo could still specify a model preference.

### What moves from native drone to SDK

These structs move from `src/drones/native/src/config.rs` to `src/drone-sdk/src/drone_toml.rs`:

- `ProviderSection` — but made optional on `DroneToml` (not all drones need it)
- `RuntimeSection` — max_tokens, max_iterations, temperature, timeout, compaction
- `CacheSection` — cache dir, size limits, repo/tool cache toggles
- `ToolsSection` + `ExternalToolSection` — sandbox, allowed/denied, external tool defs
- `McpSection` — MCP server connections
- `EnvironmentSection` — extra PATH, env vars
- `OrchestratorSection` — test command, fixup iterations, parallelism
- `CustomHealthCheck` — health check definitions

All default functions (`default_max_tokens`, `default_branch`, etc.) move too.

### What stays in native drone

- `ResolvedConfig` and the resolution logic (`resolve.rs`) — this merges drone.toml with job spec overrides and stage defaults. It's native-drone-specific because different drones resolve config differently.
- `to_provider_config()` — moves to the SDK since `ProviderSection` moves there. Native drone's `resolve.rs` calls it from the SDK.

### What changes in native drone

- Delete `src/drones/native/src/config.rs` entirely
- `src/drones/native/src/drone.rs` — change `use crate::config::DroneConfig` to `use drone_sdk::drone_toml::DroneToml`
- `src/drones/native/src/resolve.rs` — change `DroneConfig` references to `DroneToml`, import sections from SDK

### What changes in claude drone

- Already uses `DroneToml` from SDK — no structural changes
- Can now access `provider`, `tools`, `orchestrator`, etc. if needed in the future

### ProviderSection dependency

The native drone's `ProviderSection` has `to_provider_config()` which returns `runtime::api::ProviderConfig`. This creates a dependency from drone-sdk on the `runtime` crate.

Options:
1. **Keep `to_provider_config()` in native drone** — SDK defines the TOML struct, native drone has the conversion. SDK doesn't depend on `runtime`.
2. **Add `runtime` as a drone-sdk dependency** — cleaner API but tighter coupling.

Option 1 is better. The SDK defines the data shape; each drone decides how to interpret it.

### Validation

The SDK's existing `validate()` method expands to cover new sections. Current validation (identity field checks) stays. New validations as needed (e.g., `cache.max_size_mb > 0`).

### drone.toml format

Unchanged. A full drone.toml looks the same as the native drone's current format, plus the SDK's `setup`, `prompts`, and `git.identity` sections. Existing drone.toml files from either drone continue to parse correctly.

## Verification

1. `cd src/drone-sdk && cargo test` — all existing + new tests pass
2. `cd src/drones/native && cargo test` — all existing tests pass (resolve tests still work with new import paths)
3. `cd src/drones/claude/base && cargo test` — existing tests pass
4. `buck2 build root//...` — full build succeeds
5. Existing `drone.toml` in the repo root still parses correctly
