"""Reusable install macros for binaries and Claude Code plugins."""

def install_binary(name, binary, binary_name = None, dest = None, visibility = ["PUBLIC"]):
    """Generate an install target that copies a binary to ~/.local/bin (or custom dest).

    Usage in BUCK files:
        load("//rules:install.bzl", "install_binary")
        install_binary(name = "install", binary = ":my_binary")

    Then: buck2 run //path/to:install
    """
    binary_name = binary_name or binary.split(":")[-1]
    dest_expr = dest or "${HOME}/.local/bin"

    native.genrule(
        name = name + "__script",
        out = "install.sh",
        bash = """cat > $OUT << 'INSTALL_EOF'
#!/usr/bin/env bash
set -euo pipefail
DEST="${1:-DEST_PLACEHOLDER}"
mkdir -p "$DEST"
BIN=`find -L "$BUCK_PROJECT_ROOT" -name "NAME_PLACEHOLDER" -type f | head -1`
[ -z "$BIN" ] && echo "error: NAME_PLACEHOLDER not found" >&2 && exit 1
cp "$BIN" "$DEST/NAME_PLACEHOLDER"
chmod +x "$DEST/NAME_PLACEHOLDER"
echo "Installed NAME_PLACEHOLDER to $DEST/NAME_PLACEHOLDER"
INSTALL_EOF
chmod +x $OUT""".replace("NAME_PLACEHOLDER", binary_name).replace("DEST_PLACEHOLDER", dest_expr),
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
