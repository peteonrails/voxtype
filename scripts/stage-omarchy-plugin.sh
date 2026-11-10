#!/bin/bash
# Stage the Omarchy Settings plugin into its packaged, versioned layout.
#
# Usage: stage-omarchy-plugin.sh <source-dir> <dest-root>
#
#   <source-dir>  the repo's omarchy-plugin/ directory
#   <dest-root>   where io.voxtype.settings/ is created, normally
#                 $pkgroot/usr/share/voxtype/omarchy-plugin
#
# Produces:
#
#   <dest-root>/io.voxtype.settings/
#     manifest.json              entryPoints.panel = "<version>/Panel.qml"
#     LICENSE
#     <version>/Panel.qml
#     <version>/components/*.qml
#
# <version> is the manifest's version string, including any -rcN suffix.
# The QML lives under a versioned directory so every upgrade gives Omarchy a
# new entry URL: Omarchy re-adds a panel when its sourceUrl changes, and a new
# URL cannot hit a stale entry in Qt's component cache. The repo keeps its flat
# layout so `omarchy plugin add` from git and the QML tests keep working; only
# packaging produces this one.
#
# dev/, tests/, install/, README.md and dotfiles are not shipped.
#
# Every package (deb, rpm, AUR) calls this one script so the file list cannot
# drift between them. Bash, coreutils and sed only, so no packaging path gains
# a build dependency.
set -euo pipefail

PLUGIN_ID="io.voxtype.settings"

die() {
    echo "stage-omarchy-plugin: $*" >&2
    exit 1
}

[[ $# -eq 2 ]] || die "usage: $0 <source-dir> <dest-root>"
SRC="${1%/}"
DEST_ROOT="${2%/}"

[[ -f "$SRC/manifest.json" ]] || die "$SRC/manifest.json not found"
[[ -f "$SRC/Panel.qml" ]] || die "$SRC/Panel.qml not found"
[[ -d "$SRC/components" ]] || die "$SRC/components/ not found"
[[ -f "$SRC/LICENSE" ]] || die "$SRC/LICENSE not found"

# The version doubles as a directory name inside an entry point, which
# Omarchy rejects if it contains "..", so accept only a plain release or
# prerelease version.
VERSION_LINES=$(sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$SRC/manifest.json")
[[ -n "$VERSION_LINES" ]] || die "no \"version\" in $SRC/manifest.json"
[[ "$VERSION_LINES" != *$'\n'* ]] || die "more than one \"version\" in $SRC/manifest.json"
VERSION="$VERSION_LINES"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9]+(\.[A-Za-z0-9]+)*)?$ ]] \
    || die "manifest version '$VERSION' is not a release version"

ENTRY_PATTERN='"panel"[[:space:]]*:[[:space:]]*"Panel.qml"'
ENTRY_COUNT=$(grep -c "$ENTRY_PATTERN" "$SRC/manifest.json" || true)
[[ "$ENTRY_COUNT" -eq 1 ]] \
    || die "expected exactly one \"panel\": \"Panel.qml\" entry point in $SRC/manifest.json"

PLUGIN_DIR="$DEST_ROOT/$PLUGIN_ID"
# Refuse rather than merge: a leftover version directory beside the new one
# would ship two copies of the panel.
[[ ! -e "$PLUGIN_DIR" ]] || die "$PLUGIN_DIR already exists"

shopt -s nullglob
COMPONENTS=("$SRC"/components/*.qml)
shopt -u nullglob
[[ ${#COMPONENTS[@]} -gt 0 ]] || die "no components in $SRC/components/"

install -d -m 755 "$DEST_ROOT" "$PLUGIN_DIR" "$PLUGIN_DIR/$VERSION" "$PLUGIN_DIR/$VERSION/components"
install -m 644 "$SRC/LICENSE" "$PLUGIN_DIR/LICENSE"
install -m 644 "$SRC/Panel.qml" "$PLUGIN_DIR/$VERSION/Panel.qml"
install -m 644 "${COMPONENTS[@]}" "$PLUGIN_DIR/$VERSION/components/"

sed "s|$ENTRY_PATTERN|\"panel\": \"$VERSION/Panel.qml\"|" "$SRC/manifest.json" > "$PLUGIN_DIR/manifest.json"
chmod 644 "$PLUGIN_DIR/manifest.json"
grep -q "\"panel\": \"$VERSION/Panel.qml\"" "$PLUGIN_DIR/manifest.json" \
    || die "failed to rewrite the panel entry point"

echo "Staged $PLUGIN_ID $VERSION into $PLUGIN_DIR"
