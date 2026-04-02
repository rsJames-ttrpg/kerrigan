---
name: creep-discovery
description: "Use when exploring or navigating a codebase — find files by pattern, check file metadata, understand workspace structure. Faster than glob/grep for indexed workspaces."
---

# Creep File Discovery

Use `creep-cli` to search the pre-indexed file tree. The workspace was registered automatically on drone startup — files are already indexed with content hashes and type detection.

## When to Use

- Finding files by glob pattern across the workspace
- Checking whether a file exists and its type before reading it
- Getting a quick overview of what files exist in a directory pattern
- Comparing content hashes to detect changes

## When NOT to Use

- Searching file *contents* (use Grep for that)
- Reading file contents (use Read for that)
- If `creep-cli` fails with a connection error, fall back to Glob/Grep — Creep may not be running

## Commands

All commands support `--json` for machine-parseable output. Always use `--json` when you need to parse the result.

### Search files by pattern

```bash
creep-cli search "*.rs"                              # all Rust files
creep-cli search "src/**/*.rs" --json                # Rust files under src/, JSON output
creep-cli search "*_test.rs" --type rust             # test files, filtered by type
creep-cli search "*.toml" --workspace /path/to/repo  # filter by workspace
```

### Get metadata for a specific file

```bash
creep-cli metadata /absolute/path/to/file.rs
```

Output: path, size, modified timestamp, file type, BLAKE3 content hash.

### Register / unregister workspaces

Workspaces are registered automatically by the drone. You should not need these unless debugging.

```bash
creep-cli register /path/to/workspace
creep-cli unregister /path/to/workspace
```

## Tips

- Patterns are glob patterns, not regex. Use `*` for single-level, `**` for recursive.
- File types detected: rust, python, typescript, javascript, go, c, cpp, java, toml, yaml, json, markdown, and more.
- Content hashes are BLAKE3 — fast and collision-resistant. Use them to check if a file has changed between operations.
