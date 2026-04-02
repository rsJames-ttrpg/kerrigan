# Hermetic Toolchain PATH Exposure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the hermetic `cargo`, `rustc`, `rustfmt`, and `clippy-driver` binaries on PATH inside drone sessions so that `cargo check`, `cargo test`, `rustfmt --check`, and `cargo clippy` work without a `buck2 run` prefix.

**Architecture:** Four independent changes: (1) new `tools:toolchain-bin` Buck2 genrule producing a self-relocatable wrapper directory, (2) `materialize_toolchain_bin()` helper in `environment.rs` that materialises the directory via Buck2 and returns the `bin/` path, (3) call site in `drone.rs` `setup()` that prepends the `bin/` path to `PATH` in `.drone-env`, (4) symlink step in `buckstrap.sh` for local dev.

**Spec:** `docs/specs/2026-04-02-hermetic-toolchain-path-design.md`

---

## File Structure

### Modified files

| File | Change |
|------|--------|
| `toolchains/BUCK` | Add `visibility = ["PUBLIC"]` to `clippy-x86_64-linux` target |
| `tools/BUCK` | Add `toolchain-bin` genrule |
| `src/drones/claude/base/src/environment.rs` | Add `materialize_toolchain_bin()` function |
| `src/drones/claude/base/src/drone.rs` | Call `materialize_toolchain_bin()` in `setup()`, push PATH to env_vars |
| `tools/buckstrap.sh` | Symlink wrappers to `~/.local/bin/` after warming cache |
| `CLAUDE.md` | Document PATH guarantee for drone sessions in "Build System" section |

### No new files

All changes are modifications to existing files.

---

## Task 1: Expose `clippy-x86_64-linux` with PUBLIC visibility

**Files:**
- Modify: `toolchains/BUCK`

The `toolchain-bin` genrule in `tools/BUCK` (Task 2) references
`toolchains//:clippy-x86_64-linux`. That target currently has no `visibility`
annotation (defaults to private within the `toolchains/` cell). This task makes
it accessible from `tools/BUCK`.

- [ ] **Step 1: Add visibility to `clippy-x86_64-linux`**

In `toolchains/BUCK`, find the `clippy-x86_64-linux` http_archive (lines 38–42):

```python
http_archive(
    name = "clippy-x86_64-linux",
    urls = ["https://static.rust-lang.org/dist/{}/clippy-nightly-{}.tar.xz".format(RUST_NIGHTLY, RUST_HOST)],
    sha256 = "d79368518f92ed0a06610f7aa89e38b382fe1324baf381464dc87c1816ed6a09",
    strip_prefix = "clippy-nightly-{}/clippy-preview".format(RUST_HOST),
)
```

Add `visibility = ["PUBLIC"]`:

```python
http_archive(
    name = "clippy-x86_64-linux",
    urls = ["https://static.rust-lang.org/dist/{}/clippy-nightly-{}.tar.xz".format(RUST_NIGHTLY, RUST_HOST)],
    sha256 = "d79368518f92ed0a06610f7aa89e38b382fe1324baf381464dc87c1816ed6a09",
    strip_prefix = "clippy-nightly-{}/clippy-preview".format(RUST_HOST),
    visibility = ["PUBLIC"],
)
```

- [ ] **Step 2: Verify the target resolves**

Run: `buck2 targets toolchains//:clippy-x86_64-linux`
Expected: prints `toolchains//:clippy-x86_64-linux`

- [ ] **Step 3: Commit**

```bash
git add toolchains/BUCK
git commit -m "feat(toolchains): expose clippy-x86_64-linux with PUBLIC visibility"
```

---

## Task 2: Add `toolchain-bin` genrule to `tools/BUCK`

**Files:**
- Modify: `tools/BUCK`

This genrule produces a directory containing self-relocatable wrapper scripts for
`cargo`, `rustc`, `rustdoc`, `rustfmt`, and `clippy-driver`. Each wrapper resolves
its sibling `rustc-dist/` or `rustfmt-dist/` at runtime relative to `$0`, so the
output is relocatable without baking in absolute paths.

The genrule copies the full contents of the three archives into the output directory
(dereference symlinks with `cp -rfL` to ensure the output is self-contained in Buck2's
content-addressed cache), then writes the wrapper scripts.

**Note on `uses_local_filesystem_abspaths`:** the existing `rustfmt` genrule carries
this label because it bakes absolute paths into the wrapper at build time. The new
`toolchain-bin` genrule does NOT do this — paths are resolved at wrapper invocation
time via `readlink -f` — so the label is intentionally omitted.

- [ ] **Step 1: Append the `toolchain-bin` genrule to `tools/BUCK`**

Add at the end of `tools/BUCK`:

```python
# -- hermetic toolchain wrappers -----------------------------------------------
# Produces a self-relocatable bin/ directory with cargo, rustc, rustdoc, rustfmt,
# and clippy-driver. Path resolved at runtime via readlink -f, not baked in.
# Usage (drone setup): buck2 build root//tools:toolchain-bin --show-full-output

genrule(
    name = "toolchain-bin",
    srcs = [
        "toolchains//:rustc-x86_64-linux",
        "toolchains//:rustfmt-x86_64-linux",
        "toolchains//:clippy-x86_64-linux",
    ],
    out = "toolchain-bin",
    bash = """
        set -euo pipefail
        RUSTC_DIST="$PWD/$(location toolchains//:rustc-x86_64-linux)"
        RUSTFMT_DIST="$PWD/$(location toolchains//:rustfmt-x86_64-linux)"
        CLIPPY_DIST="$PWD/$(location toolchains//:clippy-x86_64-linux)"

        mkdir -p "$OUT/rustc-dist" "$OUT/rustfmt-dist" "$OUT/bin"

        # Copy relevant subtrees, dereferencing symlinks so output is self-contained
        cp -rfL "$RUSTC_DIST"/. "$OUT/rustc-dist/"
        cp -rfL "$RUSTFMT_DIST"/. "$OUT/rustfmt-dist/"
        # Only the clippy-driver binary; its sysroot lives in rustc-dist
        cp -L "$CLIPPY_DIST/bin/clippy-driver" "$OUT/bin/clippy-driver-bin"

        # Helper: write a self-relocatable wrapper for a binary inside rustc-dist or rustfmt-dist.
        # TOOLCHAIN_ROOT resolves to $OUT at runtime (one level above bin/).
        write_wrapper() {
            local name="$1" exec_path="$2"
            cat > "$OUT/bin/$name" <<WRAPPER
#!/usr/bin/env bash
TOOLCHAIN_ROOT="\$(cd "\$(dirname "\$(readlink -f "\$0")")/.." && pwd)"
export LD_LIBRARY_PATH="\$TOOLCHAIN_ROOT/rustc-dist/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\$TOOLCHAIN_ROOT/$exec_path" "\$@"
WRAPPER
            chmod +x "$OUT/bin/$name"
        }

        write_wrapper cargo   "rustc-dist/bin/cargo"
        write_wrapper rustc   "rustc-dist/bin/rustc"
        write_wrapper rustdoc "rustc-dist/bin/rustdoc"
        write_wrapper rustfmt "rustfmt-dist/bin/rustfmt"

        # clippy-driver: same LD_LIBRARY_PATH treatment, but exec the copied binary
        cat > "$OUT/bin/clippy-driver" <<'WRAPPER'
#!/usr/bin/env bash
TOOLCHAIN_ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
export LD_LIBRARY_PATH="$TOOLCHAIN_ROOT/rustc-dist/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$TOOLCHAIN_ROOT/bin/clippy-driver-bin" "$@"
WRAPPER
        chmod +x "$OUT/bin/clippy-driver"
    """,
    visibility = ["PUBLIC"],
)
```

**Implementation note on wrapper quoting:** The `write_wrapper` function uses an
unquoted HEREDOC (`<<WRAPPER`) so that `$exec_path` is interpolated (producing the
correct subpath like `rustc-dist/bin/cargo`), while `\$` sequences produce literal
`$` in the installed script. The standalone `clippy-driver` block uses `<<'WRAPPER'`
(quoted HEREDOC) since there is no variable to interpolate.

- [ ] **Step 2: Build and smoke-test the target**

```bash
buck2 build root//tools:toolchain-bin
```

Expected: succeeds without error.

```bash
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null | awk 'NR==1{print $2}')
ls "$TOOLCHAIN_BIN/bin/"
```

Expected: lists `cargo`, `rustc`, `rustdoc`, `rustfmt`, `clippy-driver`,
`clippy-driver-bin`.

```bash
"$TOOLCHAIN_BIN/bin/cargo" --version
"$TOOLCHAIN_BIN/bin/rustfmt" --version
"$TOOLCHAIN_BIN/bin/clippy-driver" --version
```

Expected: each prints a version string without errors.

- [ ] **Step 3: Commit**

```bash
git add tools/BUCK
git commit -m "feat(tools): add toolchain-bin genrule with self-relocatable wrappers"
```

---

## Task 3: Add `materialize_toolchain_bin()` to `environment.rs`

**Files:**
- Modify: `src/drones/claude/base/src/environment.rs`

This async function runs `buck2 build root//tools:toolchain-bin --show-full-output`
inside the cloned workspace and parses the output path from stdout. It returns the
path to the `bin/` subdirectory inside the materialised directory. Buck2 materialises
the artifact on first call and returns the cached path on all subsequent calls
(sub-second with warm cache).

The function is intentionally **non-fatal at the call site** — if Buck2 is unavailable
or the build fails, the drone session continues without the hermetic PATH.

- [ ] **Step 1: Add `materialize_toolchain_bin` to `environment.rs`**

In `src/drones/claude/base/src/environment.rs`, add after the `clone_repo` function
(after line 209), before the `cleanup` function:

```rust
/// Materialise the hermetic toolchain wrapper directory via Buck2 and return
/// the path to its `bin/` subdirectory.
///
/// Runs `buck2 build root//tools:toolchain-bin --show-full-output` inside
/// `workspace`. Buck2 materialises the artifact on first call and returns the
/// cached path on all subsequent calls (sub-second with a warm cache).
///
/// Returns `Err` if Buck2 is unavailable, the build fails, or the output line
/// cannot be parsed. The caller should treat this as non-fatal.
pub async fn materialize_toolchain_bin(workspace: &Path) -> Result<PathBuf> {
    let output = Command::new("buck2")
        .args([
            "build",
            "root//tools:toolchain-bin",
            "--show-full-output",
        ])
        .current_dir(workspace)
        .output()
        .await
        .context("failed to spawn `buck2 build root//tools:toolchain-bin`")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`buck2 build root//tools:toolchain-bin` failed (exit {:?}): {stderr}",
            output.status.code()
        );
    }

    // stdout line format: "root//tools:toolchain-bin <abs-path>"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let abs_path = stdout
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let label = parts.next()?;
            let path = parts.next()?;
            if label == "root//tools:toolchain-bin" {
                Some(path.to_string())
            } else {
                None
            }
        })
        .context("could not find `root//tools:toolchain-bin` output path in buck2 stdout")?;

    Ok(PathBuf::from(abs_path).join("bin"))
}
```

- [ ] **Step 2: Verify the crate compiles**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles. (Failure on the embedded `claude-cli` binary is a pre-existing
issue unrelated to this change.)

- [ ] **Step 3: Commit**

```bash
git add src/drones/claude/base/src/environment.rs
git commit -m "feat(drone/env): add materialize_toolchain_bin() to expose hermetic tools on PATH"
```

---

## Task 4: Call `materialize_toolchain_bin` from `drone.rs` `setup()`

**Files:**
- Modify: `src/drones/claude/base/src/drone.rs`

In `setup()`, after clone and task file setup, call `materialize_toolchain_bin`.
On success, prepend its `bin/` path to `PATH` and push it into `env_vars`.
On failure, log a warning and continue — the session may still succeed if the
task doesn't exercise `cargo`/`rustc` directly.

`env_vars` is written to `.drone-env` at the end of `setup()`, which
`execute()` reads back and injects into the Claude CLI subprocess via the existing
`extra_env` mechanism.

**Current `setup()` structure (lines 19–67) for reference:**
1. `create_home` (line 20)
2. Configure GitHub auth from secrets (lines 23–27)
3. `clone_repo` (lines 29–35)
4. `write_task` (line 36)
5. Generate stage-specific CLAUDE.md (lines 39–45)
6. Configure MCP URL (lines 48–51)
7. Collect `env_vars` (lines 53–64) — `BUCK2_RE_HTTP_HEADERS` from secrets
8. Write `env_vars` if non-empty (lines 63–65)
9. `return Ok(env)` (line 67)

The new toolchain step is inserted into step 7 (env_vars collection), after the
BuildBuddy key block. The `write_env_vars` guard must also be relaxed so PATH
is written even when no BB key is present.

- [ ] **Step 1: Insert toolchain materialisation into `setup()`**

In `src/drones/claude/base/src/drone.rs`, replace the env_vars collection and
write block (lines 53–65):

```rust
        // Collect environment variables for the drone session
        let mut env_vars = Vec::new();
        if let Some(secrets) = job.config.get("secrets") {
            if let Some(bb_key) = secrets.get("buildbuddy_api_key").and_then(|v| v.as_str()) {
                env_vars.push((
                    "BUCK2_RE_HTTP_HEADERS".to_string(),
                    format!("x-buildbuddy-api-key:{bb_key}"),
                ));
            }
        }
        if !env_vars.is_empty() {
            environment::write_env_vars(&env.home, &env_vars).await?;
        }
```

with:

```rust
        // Collect environment variables for the drone session
        let mut env_vars = Vec::new();
        if let Some(secrets) = job.config.get("secrets") {
            if let Some(bb_key) = secrets.get("buildbuddy_api_key").and_then(|v| v.as_str()) {
                env_vars.push((
                    "BUCK2_RE_HTTP_HEADERS".to_string(),
                    format!("x-buildbuddy-api-key:{bb_key}"),
                ));
            }
        }

        // Materialise hermetic toolchain and prepend its bin/ to PATH.
        // Non-fatal: session continues if Buck2 or the build is unavailable.
        match environment::materialize_toolchain_bin(&env.workspace).await {
            Ok(bin_dir) => {
                let existing_path =
                    std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
                env_vars.push((
                    "PATH".to_string(),
                    format!("{}:{existing_path}", bin_dir.display()),
                ));
                tracing::info!(bin_dir = %bin_dir.display(), "hermetic toolchain on PATH");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to materialise hermetic toolchain; cargo/rustc may be unavailable"
                );
            }
        }

        if !env_vars.is_empty() {
            environment::write_env_vars(&env.home, &env_vars).await?;
        }
```

- [ ] **Step 2: Verify the crate compiles**

Run: `cd src/drones/claude/base && cargo check`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add src/drones/claude/base/src/drone.rs
git commit -m "feat(drone): prepend hermetic toolchain bin/ to PATH during setup"
```

---

## Task 5: Extend `buckstrap.sh` with toolchain symlinks

**Files:**
- Modify: `tools/buckstrap.sh`

After the existing `buck2 build root//...` warm-up step, materialise `toolchain-bin`
and symlink its wrappers into `~/.local/bin/`. This provides the same `cargo`/`rustc`
experience for local developers without copying any binaries.

Symlinks (not copies) preserve the single-source-of-truth: the Buck2 content-addressed
cache. If the cache is cleaned, symlinks break, but `buckstrap.sh` repairs them on
re-run.

- [ ] **Step 1: Add toolchain symlink step to `buckstrap.sh`**

In `tools/buckstrap.sh`, after line 75 (`buck2 build root//...`) and before line 77
(`echo "Bootstrap complete"`), insert:

```bash
echo "Installing hermetic toolchain wrappers to ~/.local/bin/..."
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null \
  | awk 'NR==1{print $2}')
if [[ -n "$TOOLCHAIN_BIN" ]]; then
    mkdir -p "$HOME/.local/bin"
    for bin in cargo rustc rustdoc rustfmt clippy-driver; do
        ln -sf "$TOOLCHAIN_BIN/bin/$bin" "$HOME/.local/bin/$bin"
    done
    echo "  -> symlinked cargo, rustc, rustdoc, rustfmt, clippy-driver to ~/.local/bin/"
    echo "  -> add ~/.local/bin to PATH if not already present"
else
    echo "  -> WARNING: toolchain-bin build failed; skipping symlink step"
fi
```

- [ ] **Step 2: Verify the script is valid bash**

Run: `bash -n tools/buckstrap.sh`
Expected: exits 0 (no syntax errors)

- [ ] **Step 3: Commit**

```bash
git add tools/buckstrap.sh
git commit -m "feat(buckstrap): symlink hermetic toolchain wrappers to ~/.local/bin/"
```

---

## Task 6: Update `CLAUDE.md` to document the PATH guarantee

**Files:**
- Modify: `CLAUDE.md`

Add a callout to the "Build System" section documenting that `cargo`, `rustc`,
`rustfmt`, and `clippy-driver` are automatically on PATH in drone environments.
This informs drone agents (who read the project CLAUDE.md as part of their context)
that they can use these tools directly.

Also add guidance to prefer `cargo check`/`cargo test` over `cargo build` (which
would bypass Buck2's build graph) and to prefer `buck2 build ...[clippy.txt]` for
CI-equivalent clippy runs.

- [ ] **Step 1: Add drone-environment note to the "Build System" section**

In `CLAUDE.md`, find the "Build System" section header. After the opening paragraph
(after the `buck2 build root//...` / `buck2 run root//...` bullet list and before the
"### Remote Execution (BuildBuddy)" subsection), insert:

```markdown
**In drone environments**, `cargo`, `rustc`, `rustfmt`, and `clippy-driver` are
placed on `PATH` automatically during drone setup (via `buck2 build root//tools:toolchain-bin`).
Use them directly — no `buck2 run` prefix needed.

- `cargo check` / `cargo test` — fast feedback loop (no `buck2 run` prefix needed)
- `cargo build` — avoid in drone sessions; use `buck2 build` instead to keep Buck2's
  build graph consistent and leverage the shared remote cache
- `cargo clippy` — acceptable for quick interactive checks; use
  `buck2 build 'root//src/<crate>:<crate>[clippy.txt]'` for CI-equivalent results
- `rustfmt --check` / `rustfmt` — use directly (same binary as `buck2 run root//tools:rustfmt`)
```

- [ ] **Step 2: Verify the CLAUDE.md is valid Markdown**

Run: `grep -n "In drone environments" CLAUDE.md`
Expected: prints the line you just added.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document hermetic toolchain PATH guarantee for drone environments"
```

---

## Task 7: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Cargo check all affected crates**

Run: `cargo check -p drone-sdk`
Expected: compiles.

Note: `claude-drone` requires an embedded `claude-cli` binary fetched by Buck2.
If `cargo check` on that crate fails with a missing file error, that is a
pre-existing issue. Verify the change compiles by inspecting the modified functions
for type errors manually.

- [ ] **Step 2: Run tests on affected crates**

Run: `cd src/drones/claude/base && cargo test --lib`
Expected: all existing tests pass (the new function has no tests yet — it invokes
a real `buck2` binary, which is an integration concern).

- [ ] **Step 3: Buck2 build and test the `toolchain-bin` target**

```bash
buck2 build root//tools:toolchain-bin
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null | awk 'NR==1{print $2}')
"$TOOLCHAIN_BIN/bin/cargo" --version
"$TOOLCHAIN_BIN/bin/rustfmt" --version
"$TOOLCHAIN_BIN/bin/clippy-driver" --version
```

Expected: all print version strings.

- [ ] **Step 4: Verify wrappers are self-relocatable**

```bash
# Copy to a temp location and test there — simulates a moved Buck2 cache path
TEMP_DIR=$(mktemp -d)
cp -r "$TOOLCHAIN_BIN/.." "$TEMP_DIR/toolchain-bin"
"$TEMP_DIR/toolchain-bin/bin/cargo" --version
"$TEMP_DIR/toolchain-bin/bin/rustfmt" --version
rm -rf "$TEMP_DIR"
```

Expected: both print version strings (self-relocation works).

- [ ] **Step 5: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass.

---

## Open questions (not blocking, track separately)

1. **Persistent `CARGO_HOME` across drone sessions** — with `HOME=/tmp/drone-{id}`,
   Cargo writes its registry to `/tmp/drone-{id}/.cargo`, which is deleted by
   `teardown()`. Consider pointing `CARGO_HOME` at a shared directory (e.g.
   `/var/cache/kerrigan/cargo`) to avoid re-downloading crates on every session.
   Tracked separately from this issue.

2. **`cargo clippy` vs `buck2 build ...[clippy.txt]`** — verify that `cargo clippy`
   (which drives `clippy-driver` via Cargo metadata) produces equivalent diagnostics
   to `buck2 build ...[clippy.txt]`. Document the preferred invocation in
   `src/drones/claude/base/src/config/CLAUDE.md` and the stage-specific CLAUDE.md
   templates in `src/drones/claude/base/src/stages.rs`.

3. **macOS local dev** — `readlink -f` is not available on macOS (BSD `readlink` lacks
   `-f`). The wrappers work on Linux (drone target). If macOS dev support is added,
   `buckstrap.sh` will need a `greadlink` fallback (via coreutils).
