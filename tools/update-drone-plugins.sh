#!/bin/bash
set -euo pipefail

# Update pinned plugin commits in tools/BUCK.
# Checks anthropics/claude-plugins-official and obra/superpowers for new commits
# on their default branches, then updates the commit SHA and archive SHA256.

BUCK_FILE="$(cd "$(dirname "$0")" && pwd)/BUCK"

update_plugin() {
    local name="$1" repo="$2" var_commit="$3" var_sha256="$4"

    local old_commit
    old_commit=$(grep "${var_commit} = " "$BUCK_FILE" | sed 's/.*"\(.*\)".*/\1/')

    local new_commit
    new_commit=$(gh api "repos/${repo}/commits/HEAD" --jq '.sha')
    if [ -z "$new_commit" ]; then
        echo "ERROR: failed to fetch latest commit for ${repo}" >&2
        return 1
    fi

    if [ "$old_commit" = "$new_commit" ]; then
        echo "${name}: already at ${old_commit:0:12}"
        return 0
    fi

    local new_sha256
    new_sha256=$(curl -sfL "https://github.com/${repo}/archive/${new_commit}.tar.gz" | sha256sum | awk '{print $1}')
    if [ -z "$new_sha256" ]; then
        echo "ERROR: failed to compute SHA256 for ${repo}" >&2
        return 1
    fi

    sed -i "s/${var_commit} = \".*\"/${var_commit} = \"${new_commit}\"/" "$BUCK_FILE"
    sed -i "s/${var_sha256} = \".*\"/${var_sha256} = \"${new_sha256}\"/" "$BUCK_FILE"

    echo "${name}: ${old_commit:0:12} → ${new_commit:0:12}"
}

update_plugin "claude-plugins-official" "anthropics/claude-plugins-official" "PLUGINS_COMMIT" "PLUGINS_SHA256"
update_plugin "superpowers" "obra/superpowers" "SUPERPOWERS_COMMIT" "SUPERPOWERS_SHA256"
