# Hermetic Toolchain Binaries on PATH in Remote Environments

**Date:** 2026-04-02
**Status:** Draft
**Issue:** #18

---

## Problem Statement

Kerrigan drones (Claude Code agents) run in isolated environments with no system Rust
installed. The hermetic Rust toolchain — rustc, cargo, clippy-driver, and rustfmt — is
only reachable through Buck2 sub-targets (e.g. `buck2 run root//tools:rustfmt`). When a
drone agent runs `cargo check`, `cargo test`, `rustfmt --check`, or `cargo clippy`,
those commands fail with "command not found".

This makes interactive development in drone sessions painful: the CLAUDE.md explicitly
advertises `cargo check` / `cargo test` as the fast-feedback loop, but that loop is
broken in every drone-spawned environment.

---

## Clarifying Questions (answered from codebase)

**Q: Which binaries are needed?**

`cargo`, `rustc`, `rustfmt`, and `clippy-driver`. The rustc distribution archive
(`rustc-nightly-x86_64-unknown-linux-gnu.tar.xz`) bundles cargo alongside rustc and
rustdoc. Clippy is downloaded separately and already gets a self-relocatable wrapper in
`toolchains/rust_dist.bzl`. rustfmt has a wrapper genrule in `tools/BUCK` that sets
`LD_LIBRARY_PATH`, but it is only accessible via `buck2 run root//tools:rustfmt`.

**Q: Is `buck2` available inside a drone session?**

Yes. `buckstrap.sh` installs it to `/usr/local/bin/buck2`, the Dockerfile copies it into
the container image, and `drone.rs` inherits the Queen's PATH (which includes
`/usr/local/bin`). The `.drone-env` mechanism also lets the drone set additional
environment variables that are forwarded to the Claude CLI subprocess.

**Q: Does the drone run `buckstrap.sh` inside the cloned repo?**

No. The current setup phase (`environment.rs`) only clones the repo and writes config
files. It does not run any build commands. The Claude CLI session then runs inside
`env.workspace` (the cloned repo root) with the full inherited PATH.

**Q: Can Buck2 output paths be used in wrapper scripts portably?**

The existing `tools:rustfmt` genrule (tagged `uses_local_filesystem_abspaths`) bakes
absolute Buck2 cache paths into the wrapper script. That works only on the machine where
the build ran and only as long as the cache artifact is present. A cross-machine or
cross-session wrapper must find its sibling files relative to its own location at
runtime.

**Q: What are the Raspberry Pi resource constraints?**

The target hardware is an RPi with AI HAT 2. Bundling the full rustc distribution
(~150 MB uncompressed) into the drone binary would be prohibitive. The solution must
avoid copying large toolchain artifacts at every drone setup; it should materialise them
once via Buck2 and reuse the cached output.

---

## Approach Comparison

### Approach A — Self-Relocatable Wrapper Directory (recommended)

Add a new `tools:toolchain-bin` genrule that produces a single output directory:

```
toolchain-bin/
  bin/
    cargo          ← self-relocatable wrapper script
    rustc          ← self-relocatable wrapper script
    rustfmt        ← self-relocatable wrapper script
    clippy-driver  ← self-relocatable wrapper script (variant of existing sysroot wrapper)
  rustc-dist/      ← copy/symlink of rustc http_archive output
  rustfmt-dist/    ← copy/symlink of rustfmt http_archive output
```

Each wrapper resolves its own location:

```bash
#!/usr/bin/env bash
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
export LD_LIBRARY_PATH="$SCRIPT_DIR/rustc-dist/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$SCRIPT_DIR/rustc-dist/bin/cargo" "$@"
```

In the drone setup phase, after clone, add one step:

```rust
// setup() in drone.rs
let bin_dir = materialize_toolchain_bin(&env.workspace).await?;
env_vars.push(("PATH".into(), format!("{}:{}", bin_dir, std::env::var("PATH").unwrap_or_default())));
```

`materialize_toolchain_bin` runs:
```
buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null
```
and parses the output path. Buck2 materialises the artifact on first call and returns the
cached path on all subsequent calls (sub-second with warm cache).

**Pros:**
- Fully hermetic: exact same toolchain version as Buck2 builds.
- Self-relocatable: works on any machine where the Buck2 cache is populated.
- Fast after first build: subsequent calls return the cached path immediately.
- No changes to drone binary size.
- Reuses the existing `uses_local_filesystem_abspaths`-free pattern established by the
  sysroot clippy-driver wrapper.

**Cons:**
- Adds a `buck2 build` call to the drone setup critical path (~2–5 s cold, <1 s warm).
- The drone setup Rust code gains a dependency on Buck2 being on PATH.
- Absolute paths embedded in wrapper scripts are avoided, but Buck2 must be present to
  materialise the directory initially.

---

### Approach B — `buck2 run` Proxy Scripts

Install thin shell scripts to `$HOME/.local/bin/` during drone setup:

```bash
#!/usr/bin/env bash
exec buck2 run root//tools:cargo -- "$@"
```

No new genrule required. `tools:cargo` would be a new `genrule` that wraps the hermetic
binary identically to `tools:rustfmt`.

**Pros:**
- Trivial to implement — just write a few files.
- No wrapper-path concerns: Buck2 resolves everything at invocation time.
- Adding a new tool is one line of shell.

**Cons:**
- Every `cargo check` or `cargo test` invocation spawns a full Buck2 process (~100–300 ms
  cold-start overhead, repeatable per invocation).
- Buck2 stdout/stderr bleed into tool output unless carefully redirected.
- `cargo test` and interactive tools that re-exec themselves (`cargo clippy`, `cargo fmt`)
  would each pay the Buck2 overhead — bad for test suites with many crates.
- The proxy pattern is fragile if Buck2 is not on PATH inside the tool subprocess.

---

### Approach C — Bundled Toolchain Tarball in Drone Binary

At drone compile time, a genrule assembles a minimal toolchain tarball (cargo, rustc,
rustfmt, clippy-driver, and the required `.so` files). The tarball is embedded in the
drone binary via `include_bytes!`. During `setup()`, the drone extracts it to
`$HOME/.local/bin/`.

**Pros:**
- Fully self-contained: zero runtime dependency on Buck2 or network.
- Drone setup is deterministic and offline-capable.
- No Buck2 invocation in the hot path.

**Cons:**
- Rustc distribution is ~150 MB uncompressed; even stripped and compressed, the drone
  binary would grow by tens of megabytes — problematic for RPi storage and memory.
- Every toolchain bump requires a full rebuild of the drone binary (already true, but the
  artifact size makes iteration slower).
- Duplication: the toolchain already lives in Buck2's content-addressed cache; embedding
  it a second time wastes disk.
- Cross-compilation (aarch64) would require embedding a second architecture's binaries
  or a build-time platform select.

---

## Recommended Design (Approach A)

### 1. New Buck2 genrule: `tools:toolchain-bin`

Add to `tools/BUCK`:

```python
genrule(
    name = "toolchain-bin",
    srcs = [
        "toolchains//:rustc-x86_64-linux",
        "toolchains//:rustfmt-x86_64-linux",
        "toolchains//:clippy-x86_64-linux",
    ],
    out = "toolchain-bin",
    type = "directory",
    bash = """
        set -euo pipefail
        RUSTC_DIST="$PWD/$(location toolchains//:rustc-x86_64-linux)"
        RUSTFMT_DIST="$PWD/$(location toolchains//:rustfmt-x86_64-linux)"
        CLIPPY_DIST="$PWD/$(location toolchains//:clippy-x86_64-linux)"
        OUT="$OUT"

        mkdir -p "$OUT/rustc-dist" "$OUT/rustfmt-dist" "$OUT/bin"

        # Copy relevant subtrees (dereference symlinks so output is self-contained)
        cp -rfL "$RUSTC_DIST"/. "$OUT/rustc-dist/"
        cp -rfL "$RUSTFMT_DIST"/. "$OUT/rustfmt-dist/"
        # clippy-driver binary only (the sysroot is already in rustc-dist)
        cp -L "$CLIPPY_DIST/bin/clippy-driver" "$OUT/bin/clippy-driver-bin"

        # Self-relocatable wrapper: all wrappers resolve TOOLCHAIN_ROOT relative to $0
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

        write_wrapper cargo    "rustc-dist/bin/cargo"
        write_wrapper rustc    "rustc-dist/bin/rustc"
        write_wrapper rustdoc  "rustc-dist/bin/rustdoc"
        write_wrapper rustfmt  "rustfmt-dist/bin/rustfmt"

        # clippy-driver needs the same LD_LIBRARY_PATH treatment
        cat > "$OUT/bin/clippy-driver" <<WRAPPER
#!/usr/bin/env bash
TOOLCHAIN_ROOT="\$(cd "\$(dirname "\$(readlink -f "\$0")")/.." && pwd)"
export LD_LIBRARY_PATH="\$TOOLCHAIN_ROOT/rustc-dist/lib\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
exec "\$TOOLCHAIN_ROOT/bin/clippy-driver-bin" "\$@"
WRAPPER
        chmod +x "$OUT/bin/clippy-driver"
    """,
    visibility = ["PUBLIC"],
)
```

The key insight: each wrapper script uses `readlink -f "$0"` at **runtime** to find its
own location and resolves `TOOLCHAIN_ROOT` relative to that. This is identical to the
approach already used for the clippy-driver wrapper inside `rust_dist.bzl` (lines 36–37).
No absolute paths are baked in at build time.

### 2. Drone setup integration (`src/drones/claude/base/`)

Add a new function `environment::materialize_toolchain_bin(workspace: &Path) -> Result<PathBuf>`:

```rust
/// Run `buck2 build root//tools:toolchain-bin --show-full-output` inside the cloned
/// workspace and return the path to the materialised `toolchain-bin/bin` directory.
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
        .context("failed to run buck2 build for toolchain-bin")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("buck2 build root//tools:toolchain-bin failed: {stderr}");
    }

    // Output format: "root//tools:toolchain-bin <abs-path>\n"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let abs_path = stdout
        .lines()
        .find_map(|line| line.split_whitespace().nth(1))
        .context("no output path in buck2 --show-full-output")?;

    Ok(PathBuf::from(abs_path).join("bin"))
}
```

In `drone.rs` `setup()`, after `write_task` and before returning `env`:

```rust
// Materialise hermetic toolchain and prepend its bin/ dir to PATH.
match environment::materialize_toolchain_bin(&env.workspace).await {
    Ok(bin_dir) => {
        let existing_path = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
        env_vars.push(("PATH".into(), format!("{}:{existing_path}", bin_dir.display())));
        tracing::info!(bin_dir = %bin_dir.display(), "hermetic toolchain on PATH");
    }
    Err(e) => {
        // Non-fatal: the drone session may still succeed if the task doesn't need cargo.
        tracing::warn!(error = %e, "failed to materialise hermetic toolchain; cargo/rustc may be unavailable");
    }
}
```

`PATH` is then written to `.drone-env` alongside `BUCK2_RE_HTTP_HEADERS` and forwarded
to the Claude CLI subprocess via the existing `extra_env` mechanism.

### 3. `buckstrap.sh` integration (local dev)

Extend `tools/buckstrap.sh` to install the same wrappers into the developer's
`~/.local/bin/` after warming the cache:

```bash
echo "Installing hermetic toolchain wrappers to ~/.local/bin/..."
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null \
  | awk 'NR==1{print $2}')
mkdir -p "$HOME/.local/bin"
for bin in cargo rustc rustdoc rustfmt clippy-driver; do
    ln -sf "$TOOLCHAIN_BIN/bin/$bin" "$HOME/.local/bin/$bin"
done
echo "  -> add ~/.local/bin to PATH if not already present"
```

This is symlinks, not copies, so there is no duplication — the Buck2 cache remains the
single source of truth. Symlinks break if the cache is cleaned but are trivially repaired
by re-running `buckstrap.sh` (which warms the cache anyway).

### 4. CLAUDE.md update (workspace)

Update the "Build System" section of `workspace/CLAUDE.md` to document the guaranteed
availability of these tools:

> **In drone environments**, `cargo`, `rustc`, `rustfmt`, and `clippy-driver` are placed
> on PATH automatically during setup. Use them directly; no `buck2 run` prefix needed.

---

## Scope & Files Changed

| File | Change |
|------|--------|
| `tools/BUCK` | Add `toolchain-bin` genrule |
| `src/drones/claude/base/src/environment.rs` | Add `materialize_toolchain_bin()` |
| `src/drones/claude/base/src/drone.rs` | Call it in `setup()`, push `PATH` to env_vars |
| `tools/buckstrap.sh` | Symlink wrappers to `~/.local/bin/` |
| `workspace/CLAUDE.md` | Document PATH guarantee for drone sessions |

No changes to `toolchains/`, `prelude/`, Overseer, Queen, or any other component.

---

## Failure Modes & Mitigations

| Failure | Mitigation |
|---------|-----------|
| `buck2 build` fails in setup (network, missing fixup) | Non-fatal warning; session continues without hermetic PATH. Drone CLAUDE.md should note `cargo` may be unavailable. |
| Buck2 cache evicted mid-session | Wrapper scripts resolve paths at invocation time; if `$TOOLCHAIN_ROOT/rustc-dist/bin/cargo` is gone, the error is clear ("No such file"). Re-running `buck2 build root//tools:toolchain-bin` would restore it. |
| PATH ordering conflict with system rustc | Hermetic path is prepended, taking precedence over any system Rust. This is intentional. |
| aarch64 drone (RPi target) | The genrule as written targets `x86_64-unknown-linux-gnu`. Cross-compilation is out of scope for this issue; drone binaries run on x86_64 hosts, cross-compile targets are built by Buck2 at build time, not at drone runtime. |
| `readlink -f` unavailable (macOS `readlink` lacks `-f`) | Drones run on Linux only. `buckstrap.sh` symlinks for local dev would need a macOS variant if macOS dev is added. Filed as a follow-on concern. |

---

## Open Questions

1. **Cargo `.cargo/` home:** `cargo` writes its registry and build cache to
   `$CARGO_HOME` (default `~/.cargo`). With `HOME=/tmp/drone-{id}`, this is
   `/tmp/drone-{id}/.cargo`, which is cleaned up by `teardown()`. Should we configure
   `CARGO_HOME` to point to a persistent cache directory on the host to avoid
   re-downloading crates on every drone session? (Out of scope for this issue, but worth
   tracking.)

2. **`cargo build` vs `buck2 build` divergence:** Making `cargo` directly accessible may
   encourage drones to use `cargo build` instead of `buck2 build`. The two produce
   separate build artifacts and don't share a cache. Consider whether the CLAUDE.md
   should explicitly restrict cargo to `cargo check`/`cargo test` and direct `cargo build`
   to `buck2 build`.

3. **Clippy via `cargo clippy` vs `buck2 build ...[clippy.txt]`:** `cargo clippy`
   invokes `clippy-driver` directly on Cargo metadata; `buck2 build ...[clippy.txt]`
   uses the hermetic toolchain via Buck2's build graph. Both should produce equivalent
   diagnostics now that clippy-driver is the same binary. Verify this assumption and
   document the preferred invocation in the drone CLAUDE.md.
