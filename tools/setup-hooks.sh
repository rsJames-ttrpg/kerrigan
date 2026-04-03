#!/usr/bin/env bash
set -euo pipefail

# Skip if hooks already installed (prek writes .git/hooks/pre-commit)
if [ -f .git/hooks/pre-commit ] && grep -q "prek" .git/hooks/pre-commit 2>/dev/null; then
    exit 0
fi

# Need buck2 on PATH
if ! command -v buck2 &>/dev/null; then
    echo "buck2 not found, skipping hook setup" >&2
    exit 0
fi

echo "Installing pre-commit hooks..."
buck2 run root//tools:prek -- install 2>/dev/null || true
buck2 run root//tools:prek -- install --hook-type pre-push 2>/dev/null || true
