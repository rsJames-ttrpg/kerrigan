# BuildBuddy Remote Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable remote build execution via BuildBuddy so Buck2 actions run on BuildBuddy's infrastructure with shared caching.

**Architecture:** Add RE client config to `.buckconfig`, replace the local-only execution platform with a hybrid local+remote platform using a custom Starlark rule modeled on the upstream BuildBuddy example.

**Tech Stack:** Buck2, Starlark, BuildBuddy RE API (gRPC)

---

### Task 1: Add RE client config to `.buckconfig`

**Files:**
- Modify: `.buckconfig`

- [ ] **Step 1: Add the `[buck2_re_client]` section to `.buckconfig`**

Add the following section at the end of `.buckconfig`:

```ini
[buck2_re_client]
engine_address = grpcs://remote.buildbuddy.io
action_cache_address = grpcs://remote.buildbuddy.io
cas_address = grpcs://remote.buildbuddy.io
http_headers = x-buildbuddy-api-key:$BUILDBUDDY_API_KEY
```

This tells Buck2's RE client where to find BuildBuddy's services. All three endpoints (execution engine, action cache, content-addressable storage) point at the same BuildBuddy address. The API key is read from the `$BUILDBUDDY_API_KEY` environment variable — it is never committed to the repo.

- [ ] **Step 2: Verify Buck2 still parses the config**

Run: `buck2 targets root//src/overseer:overseer`

Expected: the target resolves without config parse errors. (The RE client config is inert until an execution platform enables `remote_enabled = True`.)

- [ ] **Step 3: Commit**

```bash
git add .buckconfig
git commit -m "feat: add BuildBuddy RE client config to .buckconfig"
```

---

### Task 2: Create hybrid execution platform with RE support

**Files:**
- Create: `platforms/defs.bzl`
- Modify: `platforms/BUCK`

- [ ] **Step 1: Create `platforms/defs.bzl` with the execution platform rule**

Create `platforms/defs.bzl` with the following content. This is modeled on the upstream Buck2 BuildBuddy example (`facebook/buck2/examples/remote_execution/buildbuddy/platforms/defs.bzl`):

```python
def _buildbuddy_platforms(ctx):
    configuration = ConfigurationInfo(
        constraints = {},
        values = {},
    )

    image = "docker://gcr.io/flame-public/rbe-ubuntu20-04:latest"
    platform = ExecutionPlatformInfo(
        label = ctx.label.raw_target(),
        configuration = configuration,
        executor_config = CommandExecutorConfig(
            local_enabled = True,
            remote_enabled = True,
            use_limited_hybrid = True,
            remote_execution_properties = {
                "OSFamily": "Linux",
                "container-image": image,
            },
            remote_execution_use_case = "buck2-default",
            remote_output_paths = "output_paths",
        ),
    )

    return [DefaultInfo(), ExecutionPlatformRegistrationInfo(platforms = [platform])]

buildbuddy_platforms = rule(attrs = {}, impl = _buildbuddy_platforms)
```

Key details:
- `local_enabled = True` — actions can still run locally (e.g. when `$BUILDBUDDY_API_KEY` is not set)
- `remote_enabled = True` — actions are eligible for remote execution
- `use_limited_hybrid = True` — prefers remote, falls back to local
- `container-image` — BuildBuddy's default Ubuntu 20.04 image
- `remote_output_paths = "output_paths"` — tells the RE API to use the `output_paths` field (Buck2's preferred mode)

- [ ] **Step 2: Update `platforms/BUCK` to use the new rule**

Replace the current `execution_platform` invocation with the new `buildbuddy_platforms` rule. Keep the existing `linux-x86_64` and `linux-aarch64` target platforms unchanged.

Replace the full contents of `platforms/BUCK` with:

```python
load(":defs.bzl", "buildbuddy_platforms")

buildbuddy_platforms(
    name = "default",
    visibility = ["PUBLIC"],
)

platform(
    name = "linux-x86_64",
    constraint_values = [
        "prelude//cpu/constraints:x86_64",
        "prelude//os/constraints:linux",
    ],
    visibility = ["PUBLIC"],
)

platform(
    name = "linux-aarch64",
    constraint_values = [
        "prelude//cpu/constraints:arm64",
        "prelude//os/constraints:linux",
    ],
    visibility = ["PUBLIC"],
)
```

Note: the `.buckconfig` already has `execution_platforms = platforms//:default` and `target_platform_detector_spec` pointing at `platforms//:default`, so no changes needed there.

- [ ] **Step 3: Verify Buck2 can resolve the new execution platform**

Run: `buck2 audit providers platforms//:default`

Expected: output shows `ExecutionPlatformInfo` with `executor_config` containing both `Local` and `Remote` sections, and `remote_execution_properties` including `OSFamily` and `container-image`.

- [ ] **Step 4: Verify the project still builds locally**

Run: `buck2 build root//src/overseer:overseer`

Expected: `BUILD SUCCEEDED`. Since `local_enabled = True`, the build should work exactly as before even without a BuildBuddy API key set.

- [ ] **Step 5: Commit**

```bash
git add platforms/defs.bzl platforms/BUCK
git commit -m "feat: hybrid local+remote execution platform for BuildBuddy"
```

---

### Task 3: Smoke test remote execution

**Files:** None (manual verification)

- [ ] **Step 1: Set the BuildBuddy API key**

```bash
export BUILDBUDDY_API_KEY="<your-api-key>"
```

- [ ] **Step 2: Clean and rebuild to force remote execution**

```bash
buck2 clean
buck2 build root//src/overseer:overseer
```

Expected: `BUILD SUCCEEDED`. Check the BuildBuddy dashboard at `https://app.buildbuddy.io` to verify that actions appeared.

- [ ] **Step 3: Run tests remotely**

```bash
buck2 test root//...
```

Expected: tests pass. Check BuildBuddy dashboard for test execution entries.

- [ ] **Step 4: If `http_archive` fails in RE sandbox**

If you see errors about network access being denied for `http_archive` rules (crate downloads), the fix is to run one local build first to populate the cache, then subsequent remote builds will use cached results. If this is a persistent problem, the fallback is switching to vendored crates via `reindeer vendor`.

- [ ] **Step 5: Verify local-only still works without API key**

```bash
unset BUILDBUDDY_API_KEY
buck2 clean
buck2 build root//src/overseer:overseer
```

Expected: `BUILD SUCCEEDED` using local execution only.
