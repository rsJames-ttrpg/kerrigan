#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "=== building kerrigan binaries with buck2 ==="
buck2 build \
  root//src/overseer:overseer \
  root//src/queen:queen \
  root//src/creep:creep \
  root//src/drones/claude/base:claude-drone

# Buck2 output paths are content-addressed. Use buck2 build --show-full-output
# to get the actual paths, then copy to a staging dir for Docker.
STAGE="$REPO_ROOT/deploy/dev/.stage"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/drones"

echo "=== staging binaries ==="
for target_bin in \
  "root//src/overseer:overseer bin/overseer" \
  "root//src/queen:queen bin/queen" \
  "root//src/creep:creep bin/creep" \
  "root//src/drones/claude/base:claude-drone drones/claude-drone"; do

  target="${target_bin% *}"
  dest="${target_bin#* }"
  src=$(buck2 build --show-full-output "$target" 2>/dev/null | awk '{print $2}')
  cp "$src" "$STAGE/$dest"
  echo "  $target -> $STAGE/$dest"
done

echo "=== building docker image ==="
docker build -t kerrigan -f "$REPO_ROOT/Dockerfile" "$REPO_ROOT"

echo "=== cleaning staging dir ==="
rm -rf "$STAGE"

echo "=== done ==="
echo "Run with: docker run -it --rm -p 3100:3100 -v kerrigan-data:/data kerrigan"
