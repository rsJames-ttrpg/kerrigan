# Drone Plugin Vendoring

## Problem

The Claude drone's `settings.json` enables 5 official plugins via `enabledPlugins`, but the plugin files don't exist at runtime. The drone binary embeds settings/CLAUDE.md/cli via `include_bytes!` but plugins are not embedded. The partial implementation in `tools/BUCK` is broken — it references `superpowers` in the `claude-plugins-official` archive where it doesn't exist (superpowers is a third-party plugin from `obra/superpowers`). The `install_plugins()` function is hardcoded for just `creep-discovery`.

## Design

Embed all plugins in the drone binary as an uncompressed tarball via `include_bytes!`. At runtime, extract to the drone's isolated `~/.claude/plugins/` and write `installed_plugins.json` for Claude Code discovery.

### Plugin Set

| Plugin | Source Repo | Namespace | Version |
|--------|-------------|-----------|---------|
| pr-review-toolkit | anthropics/claude-plugins-official | claude-plugins-official | vendored |
| code-simplifier | anthropics/claude-plugins-official | claude-plugins-official | vendored |
| claude-code-setup | anthropics/claude-plugins-official | claude-plugins-official | vendored |
| feature-dev | anthropics/claude-plugins-official | claude-plugins-official | vendored |
| superpowers | obra/superpowers | claude-plugins-official | vendored |
| creep-discovery | first-party (src/drones/claude/plugins/) | kerrigan | 0.1.0 |

### Build System

**Two `http_archive` targets** in `tools/BUCK`:
- `claude-plugins-official` — existing, from `anthropics/claude-plugins-official`, pinned to commit SHA
- `superpowers-plugin` — new, from `obra/superpowers`, pinned to commit SHA

**One `genrule`** (`drone-plugins-tar`) assembles all plugins into a tarball with the directory structure:
```
cache/
  claude-plugins-official/
    pr-review-toolkit/vendored/
      .claude-plugin/plugin.json
      agents/
      commands/
      ...
    code-simplifier/vendored/...
    claude-code-setup/vendored/...
    feature-dev/vendored/...
    superpowers/vendored/
      .claude-plugin/plugin.json
      skills/
      commands/
      agents/
      ...
  kerrigan/
    creep-discovery/0.1.0/
      .claude-plugin/plugin.json
      skills/
      ...
```

The genrule uses `cp -rL` to dereference `http_archive` symlinks. The tarball root is `cache/` so extracting to `{home}/.claude/plugins/` yields the correct path hierarchy.

The tarball is added to the drone's `mapped_srcs` as `src/config/drone-plugins.tar` for `include_bytes!` access.

### Runtime Extraction

`install_plugins()` in `environment.rs`:
1. Creates `{home}/.claude/plugins/`
2. Extracts the embedded tarball using the `tar` crate (in `spawn_blocking` since tar I/O is sync)
3. Writes `installed_plugins.json` with format:

```json
{
  "version": 2,
  "plugins": {
    "pr-review-toolkit@claude-plugins-official": [{
      "scope": "user",
      "installPath": "/tmp/drone-abc/.claude/plugins/cache/claude-plugins-official/pr-review-toolkit/vendored",
      "version": "vendored",
      "installedAt": "2026-01-01T00:00:00.000Z",
      "lastUpdated": "2026-01-01T00:00:00.000Z"
    }],
    ...
  }
}
```

Timestamps are static — Claude Code checks path existence, not freshness.

### creep-discovery Fix

Add missing `.claude-plugin/plugin.json` to `src/drones/claude/plugins/creep-discovery/` so Claude Code recognizes it as a valid plugin.

### settings.json Update

Add `"creep-discovery@kerrigan": true` to `enabledPlugins`.

## Files Changed

| File | Change |
|------|--------|
| `tools/BUCK` | Add `superpowers-plugin` http_archive, fix `drone-plugins-tar` genrule |
| `src/drones/claude/base/BUCK` | Add `//tools:drone-plugins-tar` to `mapped_srcs`, add `tar` dep |
| `src/drones/claude/base/Cargo.toml` | Add `tar` crate |
| `src/drones/claude/base/src/environment.rs` | Rewrite `install_plugins()` — extract tarball + write manifest |
| `src/drones/claude/base/src/config/settings.json` | Add `creep-discovery@kerrigan` |
| `src/drones/claude/plugins/creep-discovery/.claude-plugin/plugin.json` | New file |
| `src/drones/claude/plugins/BUCK` | No change needed (glob captures new file) |

## Verification

1. `buck2 build root//tools:drone-plugins-tar` — tarball builds, contains all 6 plugins with `.claude-plugin/plugin.json`
2. `buck2 build root//src/drones/claude/base:claude-drone` — binary compiles with embedded tarball
3. `buck2 test root//src/drones/claude/base:claude-drone-test` — existing tests pass
4. Run a drone job end-to-end — verify plugins appear in Claude Code's loaded plugin list
