# BuildBuddy Remote Execution for Buck2

## Goal

Enable remote build execution via BuildBuddy so actions run on BuildBuddy's infrastructure with shared caching. This is a step toward running the full dev loop in a container.

## Changes

### 1. `.buckconfig` — RE client configuration

Add a `[buck2_re_client]` section pointing at BuildBuddy's gRPC endpoints. All three services (engine, action cache, CAS) use the same address. Authentication via `x-buildbuddy-api-key` header, read from the `$BUILDBUDDY_API_KEY` environment variable.

```ini
[buck2_re_client]
engine_address = grpcs://remote.buildbuddy.io
action_cache_address = grpcs://remote.buildbuddy.io
cas_address = grpcs://remote.buildbuddy.io
http_headers = x-buildbuddy-api-key:$BUILDBUDDY_API_KEY
```

### 2. `platforms/BUCK` — hybrid execution platform

Replace the current `execution_platform` (local-only, from prelude) with a custom rule that enables both local and remote execution. The rule returns `ExecutionPlatformInfo` with:

- `local_enabled = True` — actions can still run locally when RE is unavailable
- `remote_enabled = True` — actions are eligible for remote execution
- `use_limited_hybrid = True` — prefer remote, fall back to local
- `remote_execution_properties` — `OSFamily: Linux`, `container-image: docker://gcr.io/flame-public/rbe-ubuntu20-04:latest` (BuildBuddy's default Ubuntu image)
- `remote_execution_use_case = "buck2-default"`
- `remote_output_paths = "output_paths"`

The existing `linux-x86_64` and `linux-aarch64` target platforms remain unchanged — they are target platforms, not execution platforms.

Implementation follows the upstream example at `facebook/buck2/examples/remote_execution/buildbuddy/platforms/defs.bzl`: a `defs.bzl` defining the rule, and `BUCK` invoking it.

### 3. API key management

The `$BUILDBUDDY_API_KEY` env var keeps the secret out of the repo. Set it in your shell profile for local dev and in CI environment variables for automated builds.

## What stays the same

- **Toolchains** — hermetic Rust nightly + LLVM, unchanged. They download their own artifacts via `http_archive` which works in any Linux environment.
- **Reindeer / `cargo_env = true`** — unchanged. `http_archive` rules for crate sources will attempt network access in the RE sandbox. Once cached, they won't re-download.
- **`buckify.sh` wrapper** — unchanged.
- **Cross-compilation targets** — unchanged. Buck2 handles cross-compilation through toolchains, not the execution container.
- **Local dev workflow** — unchanged. Hybrid mode means everything works locally without RE configured.

## Risks

**`http_archive` network access in RE sandbox.** Crate sources are downloaded via `http_archive` rules (`cargo_env = true`). If BuildBuddy's sandbox blocks outbound network, these will fail on cache misses. Mitigation: switch to vendored crates (`reindeer vendor`) if this becomes an issue. Expected to be low-impact since results are cached after first build.

## Files to modify

| File | Change |
|------|--------|
| `.buckconfig` | Add `[buck2_re_client]` section |
| `platforms/BUCK` | Replace `execution_platform` with custom rule loading `defs.bzl` |
| `platforms/defs.bzl` | New file: custom execution platform rule with RE properties |
