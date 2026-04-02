# Hermetic Toolchain PATH Exposure — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the hermetic `cargo`, `rustc`, `rustfmt`, and `clippy-driver` binaries on PATH so they're available after running `./tools/buckstrap.sh` — no system rustup needed.

**Architecture:** Three independent changes: (1) visibility fix for `clippy-x86_64-linux` in `toolchains/BUCK`, (2) new `tools:toolchain-bin` Buck2 genrule producing a self-relocatable wrapper directory, (3) `buckstrap.sh` materialises the target and symlinks wrappers to `~/.local/bin/`, plus CLAUDE.md documentation.

**Spec:** `docs/specs/2026-04-02-hermetic-toolchain-path-design.md`

---

## File Structure

### Modified files

| File | Change |
|------|--------|
| `toolchains/BUCK` | Add `visibility = ["PUBLIC"]` to `clippy-x86_64-linux` target |
| `tools/BUCK` | Add `toolchain-bin` genrule |
| `tools/buckstrap.sh` | Symlink wrappers to `~/.local/bin/` after warming cache |
| `CLAUDE.md` | Document hermetic toolchain availability in "Build System" section |

### No new files

All changes are modifications to existing files. No drone code changes.

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
# Usage: buck2 build root//tools:toolchain-bin --show-full-output

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

## Task 3: Extend `buckstrap.sh` with toolchain symlinks

**Files:**
- Modify: `tools/buckstrap.sh`

After the existing `buck2 build root//...` warm-up step, materialise `toolchain-bin`
and symlink its wrappers into `~/.local/bin/`. This provides `cargo`/`rustc`/`rustfmt`/
`clippy-driver` on PATH for local developers without copying any binaries.

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

## Task 4: Update `CLAUDE.md` to document toolchain availability

**Files:**
- Modify: `CLAUDE.md`

Add a note to the "Build System" section documenting that after running `buckstrap.sh`,
`cargo`, `rustc`, `rustfmt`, and `clippy-driver` are available on PATH via hermetic
toolchain wrappers in `~/.local/bin/`.

Also add guidance to prefer `cargo check`/`cargo test` over `cargo build` (which would
bypass Buck2's build graph) and to prefer `buck2 build ...[clippy.txt]` for CI-equivalent
clippy runs.

- [ ] **Step 1: Add toolchain note to the "Build System" section**

In `CLAUDE.md`, find the "Build System" section header. After the opening paragraph
(after the `buck2 build root//...` / `buck2 run root//...` bullet list and before the
"### Remote Execution (BuildBuddy)" subsection), insert:

```markdown
**Hermetic dev tools:** After running `./tools/buckstrap.sh`, `cargo`, `rustc`, `rustdoc`,
`rustfmt`, and `clippy-driver` are available on PATH via hermetic wrappers symlinked to
`~/.local/bin/`. These use the exact same toolchain as Buck2 builds — no system rustup needed.

- `cargo check` / `cargo test` — fast feedback loop, use freely
- `cargo build` — avoid; use `buck2 build` instead to keep the build graph consistent
  and leverage the shared remote cache
- `cargo clippy` — acceptable for quick checks; use
  `buck2 build 'root//src/<crate>:<crate>[clippy.txt]'` for CI-equivalent results
- `rustfmt --check` / `rustfmt` — use directly (same binary as `buck2 run root//tools:rustfmt`)
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: document hermetic toolchain on PATH after buckstrap"
```

---

## Task 5: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Buck2 build and test the `toolchain-bin` target**

```bash
buck2 build root//tools:toolchain-bin
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null | awk 'NR==1{print $2}')
"$TOOLCHAIN_BIN/bin/cargo" --version
"$TOOLCHAIN_BIN/bin/rustfmt" --version
"$TOOLCHAIN_BIN/bin/clippy-driver" --version
```

Expected: all print version strings.

- [ ] **Step 2: Verify wrappers are self-relocatable**

```bash
# Copy to a temp location and test there — simulates a moved Buck2 cache path
TEMP_DIR=$(mktemp -d)
cp -r "$TOOLCHAIN_BIN/.." "$TEMP_DIR/toolchain-bin"
"$TEMP_DIR/toolchain-bin/bin/cargo" --version
"$TEMP_DIR/toolchain-bin/bin/rustfmt" --version
rm -rf "$TEMP_DIR"
```

Expected: both print version strings (self-relocation works).

- [ ] **Step 3: Verify buckstrap.sh syntax**

Run: `bash -n tools/buckstrap.sh`
Expected: exits 0 (no syntax errors)

- [ ] **Step 4: Run pre-commit hooks**

Run: `buck2 run root//tools:prek -- run --all-files`
Expected: all hooks pass.

---

## Open questions (not blocking, track separately)

1. **`cargo clippy` vs `buck2 build ...[clippy.txt]`** — verify that `cargo clippy`
   (which drives `clippy-driver` via Cargo metadata) produces equivalent diagnostics
   to `buck2 build ...[clippy.txt]`. Document the preferred invocation.

2. **macOS local dev** — `readlink -f` is not available on macOS (BSD `readlink` lacks
   `-f`). The wrappers work on Linux. If macOS dev support is added, `buckstrap.sh`
   will need a `greadlink` fallback (via coreutils).

3. **Drone integration** — once this project-level tooling is in place, drones can
   consume it by running `buckstrap.sh` during setup or by calling
   `buck2 build root//tools:toolchain-bin --show-full-output` and prepending the
   result to PATH. That's a separate change to drone code.
