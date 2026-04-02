# Hermetic Toolchain Binaries on PATH — Project Setup

**Date:** 2026-04-02
**Status:** Draft
**Issue:** #18

---

## Problem Statement

The hermetic Rust toolchain (rustc, cargo, clippy-driver, rustfmt) is fetched by Buck2
into `buck-out/` but its binaries aren't exposed on PATH. Developers and any environment
running in this repo can only reach these tools via `buck2 run` wrappers or by having a
separate system rustup installation.

This is a project setup problem: after `./tools/buckstrap.sh`, the developer should have
`cargo`, `rustc`, `rustfmt`, and `clippy-driver` available without needing rustup or any
system Rust installation. The same applies to containers built from the Dockerfile.

---

## Clarifying Questions (answered from codebase)

**Q: Which binaries are needed?**

`cargo`, `rustc`, `rustdoc`, `rustfmt`, and `clippy-driver`. The rustc distribution
archive bundles cargo alongside rustc and rustdoc. Clippy and rustfmt are downloaded as
separate archives. Currently cargo is not fetched at all — Buck2 drives rustc directly
and doesn't need it.

**Q: What archives does the toolchain already fetch?**

- `rustc-x86_64-linux` — rustc, cargo, rustdoc, stdlib
- `rust-std-x86_64-linux` — standard library (used by Buck2 toolchain)
- `clippy-x86_64-linux` — clippy-driver binary
- `rustfmt-x86_64-linux` — rustfmt binary
- Corresponding aarch64 variants for cross-compilation

**Q: Can Buck2 output paths be used in wrapper scripts portably?**

The existing `tools:rustfmt` genrule (tagged `uses_local_filesystem_abspaths`) bakes
absolute Buck2 cache paths into the wrapper script. That works only on the machine where
the build ran and only as long as the cache artifact is present. A portable wrapper must
find its sibling files relative to its own location at runtime.

---

## Approach Comparison

### Approach A — Self-Relocatable Wrapper Directory (recommended)

Add a new `tools:toolchain-bin` genrule that produces a single output directory:

```
toolchain-bin/
  bin/
    cargo          ← self-relocatable wrapper script
    rustc          ← self-relocatable wrapper script
    rustdoc        ← self-relocatable wrapper script
    rustfmt        ← self-relocatable wrapper script
    clippy-driver  ← self-relocatable wrapper script
  rustc-dist/      ← copy of rustc http_archive output
  rustfmt-dist/    ← copy of rustfmt http_archive output
```

Each wrapper resolves its own location at runtime:

```bash
#!/usr/bin/env bash
TOOLCHAIN_ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
export LD_LIBRARY_PATH="$TOOLCHAIN_ROOT/rustc-dist/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$TOOLCHAIN_ROOT/rustc-dist/bin/cargo" "$@"
```

`buckstrap.sh` materialises this target and symlinks the wrappers into `~/.local/bin/`.

**Pros:**
- Fully hermetic: exact same toolchain version as Buck2 builds.
- Self-relocatable: works anywhere the Buck2 cache is populated.
- Fast after first build: subsequent calls return cached path immediately.
- No system rustup/rustc dependency.

**Cons:**
- First materialisation takes ~2–5 s (cold Buck2 cache).
- Symlinks break if `buck2 clean` is run; repaired by re-running `buckstrap.sh`.

---

### Approach B — `buck2 run` Proxy Scripts

Install thin shell scripts that delegate to `buck2 run`:

```bash
#!/usr/bin/env bash
exec buck2 run root//tools:cargo -- "$@"
```

**Pros:** Trivial to implement.

**Cons:**
- Every invocation spawns a full Buck2 process (~100–300 ms overhead).
- Buck2 stdout/stderr bleed into tool output.
- Tools that re-exec themselves (like `cargo clippy`) pay the overhead multiple times.

---

## Recommended Design (Approach A)

### 1. Visibility fix: `clippy-x86_64-linux`

The `clippy-x86_64-linux` http_archive in `toolchains/BUCK` has no `visibility` annotation.
Add `visibility = ["PUBLIC"]` so it can be referenced from `tools/BUCK`.

### 2. New Buck2 genrule: `tools:toolchain-bin`

Add to `tools/BUCK`. The genrule copies the rustc, rustfmt, and clippy archives into a
self-contained output directory, then generates self-relocatable wrapper scripts in `bin/`.

Each wrapper uses `readlink -f "$0"` at runtime to find its own location and resolves
`TOOLCHAIN_ROOT` relative to that — no absolute paths baked in at build time.

### 3. `buckstrap.sh` integration

After the existing cache warm-up step, materialise `toolchain-bin` and symlink its
wrappers into `~/.local/bin/`:

```bash
TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null \
  | awk 'NR==1{print $2}')
if [[ -n "$TOOLCHAIN_BIN" ]]; then
    mkdir -p "$HOME/.local/bin"
    for bin in cargo rustc rustdoc rustfmt clippy-driver; do
        ln -sf "$TOOLCHAIN_BIN/bin/$bin" "$HOME/.local/bin/$bin"
    done
fi
```

Symlinks, not copies — Buck2 cache remains the single source of truth.

### 4. CLAUDE.md update

Document the toolchain availability in the "Build System" section. After running
`buckstrap.sh`, `cargo`, `rustc`, `rustfmt`, and `clippy-driver` are on PATH via
`~/.local/bin/`. No system rustup needed.

---

## Scope & Files Changed

| File | Change |
|------|--------|
| `toolchains/BUCK` | Add `visibility = ["PUBLIC"]` to `clippy-x86_64-linux` |
| `tools/BUCK` | Add `toolchain-bin` genrule |
| `tools/buckstrap.sh` | Symlink wrappers to `~/.local/bin/` |
| `CLAUDE.md` | Document hermetic toolchain on PATH |

No changes to drone code, Overseer, Queen, or any Rust source files. This is purely
project setup infrastructure.

---

## Failure Modes & Mitigations

| Failure | Mitigation |
|---------|-----------|
| `buck2 build` fails (network, missing fixup) | `buckstrap.sh` prints warning, skips symlink step. Developer falls back to system rustup. |
| Buck2 cache evicted (`buck2 clean`) | Symlinks break. Re-run `buckstrap.sh` to repair. |
| PATH ordering conflict with system rustc | `~/.local/bin` position in PATH depends on user's shell config. Document that hermetic toolchain should take precedence. |
| `readlink -f` unavailable (macOS) | Drones and CI run Linux. For macOS local dev, `buckstrap.sh` would need a `greadlink` fallback — file as follow-on if needed. |

---

## Open Questions

1. **`cargo build` vs `buck2 build` divergence:** Making `cargo` directly accessible may
   encourage use of `cargo build` instead of `buck2 build`. The two produce separate build
   artifacts and don't share a cache. CLAUDE.md should document that `cargo check`/`cargo test`
   are the intended use — `cargo build` should go through `buck2 build`.

2. **Clippy via `cargo clippy` vs `buck2 build ...[clippy.txt]`:** Both should produce
   equivalent diagnostics since they use the same clippy-driver binary. Worth verifying.
