#!/usr/bin/env bash
set -euo pipefail

DEST="${1:-${HOME}/.local/bin}"
mkdir -p "$DEST"

SCRIPT_DIR="$(dirname "$0")"
BINARY="$SCRIPT_DIR/src/kerrigan/kerrigan"
if [ ! -f "$BINARY" ]; then
    # Fallback: try sibling path (depends on Buck2 resource layout)
    BINARY="$SCRIPT_DIR/kerrigan"
fi
if [ ! -f "$BINARY" ]; then
    echo "error: kerrigan binary not found" >&2
    exit 1
fi

cp "$BINARY" "$DEST/kerrigan"
chmod +x "$DEST/kerrigan"
echo "Installed kerrigan to $DEST/kerrigan"
