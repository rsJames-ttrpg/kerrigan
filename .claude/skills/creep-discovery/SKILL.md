---
name: creep-discovery
description: "Use when exploring or navigating a codebase — find files by pattern, check file metadata, understand workspace structure. Faster than glob/grep for indexed workspaces."
---

# Creep File Discovery

Use `creep-cli` to search the pre-indexed file tree. The drone attempts to register the workspace with Creep during setup (after cloning). If Creep is not running or `creep-cli` is unavailable, registration is silently skipped — fall back to Glob/Grep in that case.

## When to Use

- Finding files by glob pattern across the workspace
- Checking whether a file exists and its type before reading it
- Getting a quick overview of what files exist in a directory pattern
- Comparing content hashes to detect changes
- Finding function/struct/trait definitions by name (faster and more precise than Grep)
- Getting a structural overview of a file (all functions, structs, traits defined in it)
- Understanding what's implemented on a type (search for impl blocks)

## When NOT to Use

- Searching file *contents* (use Grep for that)
- Reading file contents (use Read for that)
- Finding call sites or references (use Grep — symbol search finds definitions only)
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

### Search for symbols by name

```bash
creep-cli symbols "process"                         # find symbols containing "process"
creep-cli symbols "Config" --kind struct             # find structs named "Config"
creep-cli symbols --file /path/to/file.rs            # list all symbols in a file
creep-cli symbols --file /path/to/file.rs --json     # JSON output for parsing
creep-cli symbols "handler" --workspace /path/repo   # search within workspace
```

Symbol kinds: function, struct, enum, trait, impl, const, static, type_alias, module, macro.

Output includes: name, kind, file path, line number, enclosing scope (parent), and signature (for functions).

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
