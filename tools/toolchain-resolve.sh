#!/usr/bin/env bash
# Resolve TOOLCHAIN_ROOT from the calling wrapper's real path.
# Wrappers source this file, so BASH_SOURCE[1] is the wrapper itself.
_real="$(readlink -f "${BASH_SOURCE[1]}")"
TOOLCHAIN_ROOT="$(dirname "$(dirname "$_real")")"
export LD_LIBRARY_PATH="$TOOLCHAIN_ROOT/rustc-dist/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
