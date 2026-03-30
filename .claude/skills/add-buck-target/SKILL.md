---
name: add-buck-target
description: Use when adding a new cross-compilation target platform to Buck2, e.g. "add armv7 target", "add macos target", "support new platform"
---

# Add Buck2 Target Platform

Adds a new cross-compilation target to the Buck2 build.

## Procedure

1. **Identify the target.** Get from user:
   - Rust target triple (e.g. `aarch64-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`)
   - Short name for Buck2 targets (e.g. `aarch64-linux`, `armv7-linux`)

2. **Map to Buck2 constraint values.** Look up the correct prelude constraints:

| Arch | CPU constraint |
|------|---------------|
| x86_64 | `prelude//cpu/constraints:x86_64` |
| aarch64/arm64 | `prelude//cpu/constraints:arm64` |
| armv7 | `prelude//cpu/constraints:arm32` |

| OS | OS constraint |
|----|--------------|
| linux | `prelude//os/constraints:linux` |
| macos | `prelude//os/constraints:macos` |
| windows | `prelude//os/constraints:windows` |

If unsure about a constraint name, check: `buck-out/v2/external_cells/bundled/prelude/cpu/constraints/BUCK` and `buck-out/v2/external_cells/bundled/prelude/os/constraints/BUCK`

3. **Add platform to `platforms/BUCK`:**

```python
platform(
    name = "{short-name}",
    constraint_values = [
        "prelude//cpu/constraints:{cpu}",
        "prelude//os/constraints:{os}",
    ],
    visibility = ["PUBLIC"],
)
```

4. **Add rust-std `http_archive` to `toolchains/BUCK`:**

Fetch the SHA256:
```
curl -sL "https://static.rust-lang.org/dist/{RUST_NIGHTLY}/rust-std-nightly-{TRIPLE}.tar.xz.sha256"
```

Add the archive (follow existing naming convention `rust-std-{short-name}`):
```python
http_archive(
    name = "rust-std-{short-name}",
    urls = ["https://static.rust-lang.org/dist/{}/rust-std-nightly-{}.tar.xz".format(RUST_NIGHTLY, "{TRIPLE}")],
    sha256 = "{HASH}",
    strip_prefix = "rust-std-nightly-{TRIPLE}/rust-std-{TRIPLE}",
)
```

5. **Optionally add a target-specific variable** at the top of `toolchains/BUCK` following the `RUST_HOST`/`RUST_RPI` pattern.

6. **Verify:** `buck2 targets root//...` to confirm no parse errors.

7. **Update CLAUDE.md** — add the new platform to the Platforms section.

## Usage After Adding

```bash
buck2 build root//src/overseer:cortex --target-platforms platforms//:linux-aarch64
```

## Notes

- The `hermetic_rust_toolchain` in `toolchains/BUCK` currently targets the host triple. Cross-compilation targets only need the `rust-std` download and a platform definition — Buck2's `select()` mechanism in the Rust prelude auto-selects the correct triple based on the target platform's constraints.
- If the target needs a different linker, add a second `hermetic_rust_toolchain` with `use_bundled_linker = True` or a custom `rustc_flags` pointing to the cross-linker.
