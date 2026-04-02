#!/usr/bin/env bash
# Bootstrap buck2 for this project.
# Installs the pinned buck2 release if not already present,
# sets up pre-commit hooks, and warms the build cache.
set -euo pipefail

BUCK2_RELEASE="2026-01-19"
INSTALL_DIR="/usr/local/bin"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

need_install=true

# Check if buck2 is already installed and matches the pinned release
if command -v buck2 &>/dev/null; then
    installed="$(buck2 --version 2>/dev/null || true)"
    if echo "$installed" | grep -q "$BUCK2_RELEASE"; then
        echo "buck2 $BUCK2_RELEASE already installed"
        need_install=false
    else
        echo "buck2 found but not $BUCK2_RELEASE (got: $installed)"
    fi
fi

if [[ "$need_install" == true ]]; then
    # Detect architecture
    case "$(uname -m)" in
        x86_64)  ARCH="x86_64-unknown-linux-gnu" ;;
        aarch64) ARCH="aarch64-unknown-linux-gnu" ;;
        *)       echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
    esac

    # Install missing dependencies
    missing=()
    command -v zstd &>/dev/null || missing+=(zstd)
    command -v curl &>/dev/null || missing+=(curl)

    if [[ ${#missing[@]} -gt 0 ]]; then
        echo "Installing missing dependencies: ${missing[*]}"
        if command -v apt-get &>/dev/null; then
            sudo apt-get update -qq && sudo apt-get install -y -qq "${missing[@]}"
        elif command -v pacman &>/dev/null; then
            sudo pacman -S --noconfirm "${missing[@]}"
        elif command -v dnf &>/dev/null; then
            sudo dnf install -y "${missing[@]}"
        else
            echo "Could not detect package manager. Please install manually: ${missing[*]}" >&2
            exit 1
        fi
    fi

    echo "Downloading buck2 $BUCK2_RELEASE ($ARCH)..."
    DOWNLOAD_URL="https://github.com/facebook/buck2/releases/download/${BUCK2_RELEASE}/buck2-${ARCH}.zst"

    tmpfile="$(mktemp)"
    trap 'rm -f "$tmpfile"' EXIT

    curl -fSL "$DOWNLOAD_URL" | zstd -d > "$tmpfile"
    chmod +x "$tmpfile"

    echo "Installing buck2 to ${INSTALL_DIR}/buck2 (requires sudo)..."
    sudo install -m 755 "$tmpfile" "${INSTALL_DIR}/buck2"

    echo "buck2 $BUCK2_RELEASE installed successfully"
    buck2 --version
fi

# Set up the project
cd "$REPO_ROOT"

echo "Installing pre-commit hooks..."
buck2 run root//tools:prek -- install
buck2 run root//tools:prek -- install --hook-type pre-push

echo "Warming up buck2 cache..."
buck2 build root//...

# Hermetic toolchain wrappers (x86_64 only — genrule hardcodes x86_64 archives)
if [[ "$(uname -m)" == "x86_64" ]]; then
    echo "Installing hermetic toolchain wrappers to ~/.local/bin/..."
    TOOLCHAIN_BIN=$(buck2 build root//tools:toolchain-bin --show-full-output \
      | awk 'NR==1{print $2}')
    if [[ -d "$TOOLCHAIN_BIN/bin" ]]; then
        mkdir -p "$HOME/.local/bin"
        for bin in cargo rustc rustdoc rustfmt clippy-driver; do
            ln -sf "$TOOLCHAIN_BIN/bin/$bin" "$HOME/.local/bin/$bin"
        done
        echo "  -> symlinked cargo, rustc, rustdoc, rustfmt, clippy-driver to ~/.local/bin/"
        echo "  -> add ~/.local/bin to PATH if not already present"
    else
        echo "  -> WARNING: toolchain-bin build failed; skipping symlink step"
    fi
else
    echo "Skipping hermetic toolchain wrappers (x86_64-only; this is $(uname -m))"
fi

echo "Bootstrap complete"
