#!/bin/bash
set -euo pipefail

BUCK_FILE="$(cd "$(dirname "$0")" && pwd)/BUCK"
BUCKET="https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"

# Get current pinned version from BUCK file
OLD_VERSION=$(grep 'CLAUDE_CLI_VERSION = ' "$BUCK_FILE" | sed 's/.*"\(.*\)".*/\1/')

# Fetch latest version
NEW_VERSION=$(curl -sfL "$BUCKET/latest")
if [ -z "$NEW_VERSION" ]; then
    echo "ERROR: failed to fetch latest version" >&2
    exit 1
fi

# Fetch manifest and extract linux-x64 checksum
NEW_SHA256=$(curl -sfL "$BUCKET/$NEW_VERSION/manifest.json" \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['platforms']['linux-x64']['checksum'])")
if [ -z "$NEW_SHA256" ]; then
    echo "ERROR: failed to fetch checksum from manifest" >&2
    exit 1
fi

if [ "$OLD_VERSION" = "$NEW_VERSION" ]; then
    echo "Claude CLI already at $OLD_VERSION"
    exit 0
fi

# Update BUCK file
sed -i "s/CLAUDE_CLI_VERSION = \".*\"/CLAUDE_CLI_VERSION = \"$NEW_VERSION\"/" "$BUCK_FILE"
sed -i "s/CLAUDE_CLI_SHA256 = \".*\"/CLAUDE_CLI_SHA256 = \"$NEW_SHA256\"/" "$BUCK_FILE"

echo "Claude CLI: $OLD_VERSION → $NEW_VERSION"
echo "SHA256: $NEW_SHA256"
echo "Updated $BUCK_FILE"
