---
name: update-rust-toolchain
description: Use when updating the pinned Rust nightly version in the hermetic Buck2 toolchain, or when user says "update rust", "bump toolchain", "new nightly"
---

# Update Rust Toolchain

Updates the hermetic Rust nightly pinned in `toolchains/BUCK` to a new date.

## Procedure

1. **Determine target date.** If user provides a date, use it. Otherwise use yesterday's date (nightly builds publish overnight). Format: `YYYY-MM-DD`.

2. **Fetch new SHA256 hashes.** For every `http_archive` in `toolchains/BUCK`, fetch the `.sha256` file from `static.rust-lang.org`:

```
curl -sL "https://static.rust-lang.org/dist/{DATE}/{COMPONENT}-nightly-{TARGET}.tar.xz.sha256"
```

Components to fetch (derive from existing `http_archive` targets):
- `rustc` for each host triple
- `rust-std` for each target triple (host AND cross-compilation targets)
- `clippy` for each host triple

3. **Verify all hashes returned successfully.** If any curl returns empty or an error page, the nightly may not exist for that date. Try the previous day.

4. **Update `toolchains/BUCK`:**
   - Change `RUST_NIGHTLY = "{DATE}"`
   - Update each `sha256 = "..."` field with the new hash
   - Update the comment with the new version info

5. **Get the new version string:**
```
curl -sL "https://static.rust-lang.org/dist/{DATE}/channel-rust-nightly.toml" | grep -m1 "^version ="
```
Update the comment line: `# -- Rust nightly {DATE} ({VERSION}) --`

6. **Verify:** Run `buck2 build root//src/cortex:cortex` to confirm the new toolchain works.

7. **Report** the old and new versions to the user.

## Quick Reference

| File | What to change |
|------|---------------|
| `toolchains/BUCK` line `RUST_NIGHTLY = ...` | Date string |
| `toolchains/BUCK` each `sha256 = ...` | Hash from `.sha256` URL |
| `toolchains/BUCK` comment on line 6 | Version + date |

## Common Issues

- **Nightly not published yet:** Try previous day. Nightlies occasionally skip days.
- **Hash mismatch after update:** You fetched the hash for the wrong component/target combo. Double-check URL matches the `urls` field.
- **Build fails after update:** Breaking nightly change. Roll back the date and hashes, report to user.
