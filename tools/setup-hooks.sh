#!/usr/bin/env bash
set -euo pipefail

# Need buck2 on PATH
if ! command -v buck2 &>/dev/null; then
    echo "buck2 not found, skipping setup" >&2
    exit 0
fi

# --- Pre-commit hooks (prek) ---
if [ -f .git/hooks/pre-commit ] && grep -q "prek" .git/hooks/pre-commit 2>/dev/null; then
    echo "pre-commit hooks already installed"
else
    echo "Installing pre-commit hooks..."
    buck2 run root//tools:prek -- install 2>/dev/null || true
    buck2 run root//tools:prek -- install --hook-type pre-push 2>/dev/null || true
fi

# --- Hermetic toolchain wrappers ---
# Provision cargo, rustc, rustdoc, rustfmt, clippy-driver from the Buck2 toolchain.
# With BUCK2_RE_HTTP_HEADERS set, these pull from the remote cache.
if command -v cargo &>/dev/null; then
    echo "hermetic toolchain already on PATH"
else
    echo "Setting up hermetic toolchain wrappers..."
    TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output 2>/dev/null \
      | awk 'NR==1{print $2}')
    if [[ -n "${TOOLCHAIN_BIN:-}" && -d "$TOOLCHAIN_BIN/bin" ]]; then
        mkdir -p "$HOME/.local/bin"
        for bin in cargo rustc rustdoc rustfmt clippy-driver; do
            ln -sf "$TOOLCHAIN_BIN/bin/$bin" "$HOME/.local/bin/$bin"
        done
        if [[ -f "$TOOLCHAIN_BIN/rustc-dist/libexec/rust-analyzer-proc-macro-srv" ]]; then
            ln -sf "$TOOLCHAIN_BIN/rustc-dist/libexec/rust-analyzer-proc-macro-srv" \
                "$HOME/.local/bin/rust-analyzer-proc-macro-srv"
        fi
        export PATH="$HOME/.local/bin:$PATH"
        echo "  -> toolchain wrappers installed to $HOME/.local/bin/"
    else
        echo "  -> WARNING: toolchain-bin build failed; toolchain not available" >&2
    fi
fi
