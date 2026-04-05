"""Reusable install macros for binaries and Claude Code plugins."""

def install_binary(name, binary, binary_name = None, dest = None, visibility = ["PUBLIC"]):
    """Generate an install target that copies a binary to ~/.local/bin (or custom dest).

    Usage in BUCK files:
        load("//rules:install.bzl", "install_binary")
        install_binary(name = "install", binary = ":my_binary")
        install_binary(name = "install", binary = ":my-cli", binary_name = "my")

    Then: buck2 run //path/to:install
    """
    target_name = binary.split(":")[-1]
    binary_name = binary_name or target_name
    dest_expr = dest or "${HOME}/.local/bin"

    native.genrule(
        name = name + "__script",
        out = "install.sh",
        bash = """cat > $OUT << 'INSTALL_EOF'
#!/usr/bin/env bash
set -euo pipefail
DEST="${1:-DEST_PLACEHOLDER}"
mkdir -p "$DEST"
BIN=`find -L "$BUCK_PROJECT_ROOT" -name "TARGET_PLACEHOLDER" -type f | head -1`
[ -z "$BIN" ] && echo "error: TARGET_PLACEHOLDER not found" >&2 && exit 1
cp "$BIN" "$DEST/NAME_PLACEHOLDER"
chmod +x "$DEST/NAME_PLACEHOLDER"
echo "Installed NAME_PLACEHOLDER to $DEST/NAME_PLACEHOLDER"
INSTALL_EOF
chmod +x $OUT""".replace("TARGET_PLACEHOLDER", target_name).replace("NAME_PLACEHOLDER", binary_name).replace("DEST_PLACEHOLDER", dest_expr),
    )

    native.sh_binary(
        name = name,
        main = ":" + name + "__script",
        resources = [binary],
        visibility = visibility,
    )

def install_plugin(name, plugin, plugin_name = None, dest = None, visibility = ["PUBLIC"]):
    """Generate an install target that copies a Claude Code plugin to ~/.claude/plugins/.

    Usage in BUCK files:
        load("//rules:install.bzl", "install_plugin")
        install_plugin(name = "install", plugin = ":creep-discovery")

    Then: buck2 run //path/to:install
    """
    plugin_name = plugin_name or plugin.split(":")[-1]
    dest_expr = dest or "${HOME}/.claude/plugins/" + plugin_name

    native.genrule(
        name = name + "__script",
        out = "install.sh",
        bash = """cat > $OUT << 'INSTALL_EOF'
#!/usr/bin/env bash
set -euo pipefail
DEST="DEST_PLACEHOLDER"
PKG=`find -L "$BUCK_PROJECT_ROOT" -name "package.json" -path "*/NAME_PLACEHOLDER/package.json" | head -1`
[ -z "$PKG" ] && echo "error: NAME_PLACEHOLDER plugin not found in resources" >&2 && exit 1
SRC=`dirname "$PKG"`
rm -rf "$DEST"
mkdir -p "$DEST"
cp -rL "$SRC/." "$DEST/"

# Register in installed_plugins.json
REGISTRY="${HOME}/.claude/plugins/installed_plugins.json"
if [ -f "$REGISTRY" ] && command -v python3 >/dev/null 2>&1; then
    python3 -c "
import json, sys
from datetime import datetime, timezone
with open('$REGISTRY') as f:
    data = json.load(f)
now = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.000Z')
key = 'NAME_PLACEHOLDER@local'
ver = '0.1.0'
pkg = '$DEST/package.json'
try:
    with open(pkg) as pf:
        ver = json.load(pf).get('version', ver)
except: pass
data.setdefault('plugins', {})[key] = [{'scope': 'user', 'installPath': '$DEST', 'version': ver, 'installedAt': now, 'lastUpdated': now}]
with open('$REGISTRY', 'w') as f:
    json.dump(data, f, indent=2)
print('Registered NAME_PLACEHOLDER in installed_plugins.json')
"
else
    echo "warning: could not register plugin (missing python3 or installed_plugins.json)"
fi
echo "Installed NAME_PLACEHOLDER plugin to $DEST"
INSTALL_EOF
chmod +x $OUT""".replace("NAME_PLACEHOLDER", plugin_name).replace("DEST_PLACEHOLDER", dest_expr),
    )

    native.sh_binary(
        name = name,
        main = ":" + name + "__script",
        resources = [plugin],
        visibility = visibility,
    )
