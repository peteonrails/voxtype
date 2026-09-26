#!/bin/sh
# Run a cargo command with the desktop session out of reach.
#
# What this changes, and why each part matters:
#
#   xvfb-run -a        A private X server on an unused display. Anything that
#                      needs DISPLAY gets a virtual one; the real one is never
#                      touched.
#   XDG_RUNTIME_DIR    A throwaway directory, so runtime-dir state (the daemon
#                      lock, the sentinel files, the OSD socket) lands in a temp
#                      dir instead of the live one.
#   DBUS_SESSION_BUS_ADDRESS=disabled:  Unsetting this is not enough: libdbus
#                      then falls back to autolaunch and starts a bus of its
#                      own. The `disabled:` transport has no handler, so every
#                      connection attempt fails immediately instead.
#   WAYLAND_DISPLAY unset  No compositor, so wl-copy/wl-paste cannot reach the
#                      real clipboard even if something tried.
#   espanso           Watches the session clipboard and pastes replacements; it
#                      has nothing to watch here.
#
# Usage: scripts/test-isolated.sh [cargo args...]
#        scripts/test-isolated.sh test --lib -- --test-threads=1 harness::
set -eu

runtime=$(mktemp -d "${TMPDIR:-/tmp}/voxtype-test-runtime.XXXXXX")
cleanup() { rm -rf "$runtime"; }
trap cleanup EXIT INT TERM

exec xvfb-run -a env \
    -u WAYLAND_DISPLAY \
    -u HYPRLAND_INSTANCE_SIGNATURE \
    -u SWAYSOCK \
    -u XDG_SESSION_TYPE \
    DBUS_SESSION_BUS_ADDRESS=disabled: \
    DBUS_SYSTEM_BUS_ADDRESS=disabled: \
    XDG_RUNTIME_DIR="$runtime" \
    cargo "$@"
