#!/usr/bin/env bash
set -euo pipefail

DEST="${HOME}/.claude/plugins/creep-discovery"
mkdir -p "$DEST/skills/creep-discovery"

SCRIPT_DIR="$(dirname "$0")"

# Find the creep-discovery filegroup output
SRC="$SCRIPT_DIR/creep-discovery"
if [ ! -d "$SRC" ]; then
    # Buck2 resource layout may nest differently
    SRC="$SCRIPT_DIR/src/drones/claude/plugins/creep-discovery"
fi
if [ ! -d "$SRC" ]; then
    echo "error: creep-discovery plugin files not found" >&2
    exit 1
fi

cp "$SRC/package.json" "$DEST/package.json"
cp "$SRC/skills/creep-discovery/SKILL.md" "$DEST/skills/creep-discovery/SKILL.md"

echo "Installed creep-discovery plugin to $DEST"
