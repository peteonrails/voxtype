#!/usr/bin/env bash
# Standalone QML checks, with no live daemon, shell, or user-config writes.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mkdir -p "$root/target"
work=$(mktemp -d "$root/target/q.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/r" "$work/c" "$work/t"
chmod 700 "$work/r"
cp -R "$root/quickshell/voxtype-shared" "$work/voxtype-shared"
cp "$root/tests/qml/tst_shell_style.qml" "$work/shell.qml"
# The test's radius inputs are deterministic; don't connect to the user's
# compositor or pick up a style file inherited from a running OSD.
if ! env -u HYPRLAND_INSTANCE_SIGNATURE -u VOXTYPE_OSD_STYLE_FILE -u WAYLAND_DISPLAY -u DISPLAY \
    QT_QPA_PLATFORM=offscreen QT_QPA_PLATFORMTHEME= QT_IM_MODULE= \
    QT_QUICK_BACKEND=software QML_DISABLE_DISK_CACHE=1 \
    XDG_RUNTIME_DIR="$work/r" XDG_CACHE_HOME="$work/c" TMPDIR="$work/t" \
    timeout 20s qs -p "$work" --no-color > "$work/result.log" 2>&1; then
    tail -80 "$work/result.log"
    exit 1
fi
if ! grep -q 'SHELL_STYLE_PASS' "$work/result.log" || grep -qE 'SHELL_STYLE_FAIL|Failed to load configuration|ReferenceError|TypeError|Unable to assign' "$work/result.log"; then
    tail -80 "$work/result.log"
    exit 1
fi
grep 'SHELL_STYLE_PASS' "$work/result.log"
