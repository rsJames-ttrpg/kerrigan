#!/usr/bin/env bash
# Wrapper around `reindeer buckify` that fixes known issues in the generated BUCK.
#
# Reindeer generates buildscript_run() before http_archive() for each crate.
# The buildscript_run prelude macro uses rule_exists() to find the crate archive,
# but rule_exists() only sees rules defined earlier in the file. For crates whose
# build scripts need source files (like libsqlite3-sys), this causes an empty
# manifest_dir. Fix: move each http_archive before its buildscript_run.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUCK_FILE="$REPO_ROOT/third-party/BUCK"

# Run reindeer buckify
buck2 run root//tools:reindeer -- buckify "$@"

# Fix ordering: move each http_archive block before its buildscript_run block.
# The pattern: buildscript_run name is "<pkg>-<ver>-build-script-run" and the
# archive is "<pkg>-<ver>.crate". We use Python for reliable multi-line editing.
python3 -c "
import re, sys

with open('$BUCK_FILE') as f:
    content = f.read()

# Match buildscript_run(...) blocks followed later by their http_archive(...) blocks
# Pattern: buildscript_run block, then some content, then the matching http_archive block
pattern = re.compile(
    r'(buildscript_run\(\s*\n\s*name = \"([^\"]+)-build-script-run\".*?\n\))'
    r'(.*?)'
    r'(http_archive\(\s*\n\s*name = \"\2\.[^\"]*\.crate\".*?\n\))',
    re.DOTALL,
)

def swap(m):
    bsr = m.group(1)
    between = m.group(3)
    archive = m.group(4)
    return archive + between + bsr

content = pattern.sub(swap, content)

with open('$BUCK_FILE', 'w') as f:
    f.write(content)
"

echo "buckify complete (with ordering fix applied)"
