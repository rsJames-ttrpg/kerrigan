---
name: add-buck-dep
description: Use when adding a Rust crate dependency to a Buck2 target, e.g. "add serde", "add tokio dependency", "need a new crate"
---

# Add Rust Crate Dependency

Uses hybrid Cargo workspace + reindeer. Crate Cargo.tomls are the source of truth.

## Procedure

1. **Add the dep via cargo:**
   ```bash
   cd src/cortex && cargo add serde --features derive
   ```

2. **Regenerate third-party/BUCK:**
   ```bash
   buck2 run root//tools:reindeer -- buckify
   ```

3. **If reindeer warns about a build script**, create `third-party/fixups/<crate>/fixups.toml`:
   - Most crates: `[buildscript]\nrun = true`
   - Crates needing Cargo env vars: add `cargo_env = true`
   - Build scripts that can be skipped: `[buildscript]\nrun = false`
   - Then re-run step 2

4. **Add dep to BUCK target:**
   ```python
   deps = ["//third-party:crate-name"]
   ```

5. **Verify:** `buck2 build root//src/cortex:cortex`

## Known Fixups

| Crate | fixups.toml |
|-------|------------|
| serde | `cargo_env = true` + `[buildscript]\nrun = true` |
| serde_core | `cargo_env = true` + `[buildscript]\nrun = true` |
| serde_derive | `cargo_env = ["CARGO_PKG_VERSION_PATCH"]` |
| proc-macro2 | `[buildscript]\nrun = true` |
| quote | `[buildscript]\nrun = false` |

Check https://github.com/facebook/buck2/tree/main/shim/third-party/rust/fixups for more.
