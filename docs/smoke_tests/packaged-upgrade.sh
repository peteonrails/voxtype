#!/bin/bash
# Packaged upgrade test: $1 = previous version, $2 = next version, $3 = deb|rpm
# Runs inside a container with the packages mounted at /pkgs (read-only).
# See packaged-upgrade.md for what each phase proves.
set -u
if [ "$#" -ne 3 ]; then
  echo "usage: packaged-upgrade.sh <previous version> <next version> deb|rpm" >&2
  exit 2
fi
PREV="$1"; NEXT="$2"; KIND="$3"
PASS=0; FAIL=0; FINISHED=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

# The verdict is printed on every way out, and only a run that reached the
# last phase with at least one check and no failure exits 0. This script has
# passed while testing nothing. Piped into `docker run ... bash -s` with
# stdin attached, apt-get read the rest of the script as its own input, so
# bash stopped after phase 1 and exited 0. (Piped in without `-i` the
# container gets no stdin and bash runs nothing at all, which no script can
# catch: packaged-upgrade.md mounts the script instead.)
finish() {
  echo
  echo "RESULT: $PASS passed, $FAIL failed"
  if [ "$FINISHED" -ne 1 ]; then
    echo "ERROR: the script stopped before its last phase" >&2
    exit 1
  fi
  if [ $((PASS + FAIL)) -eq 0 ]; then
    echo "ERROR: no checks ran" >&2
    exit 1
  fi
  [ "$FAIL" -eq 0 ] || exit 1
}
trap finish EXIT

# The package managers get no stdin, so they cannot consume this script when
# it is read from stdin.
install_pkg() {
  local f="$1"
  if [ "$KIND" = deb ]; then
    apt-get install -y "/pkgs/$f" >/dev/null 2>&1 </dev/null
  else
    dnf install -y "/pkgs/$f" >/dev/null 2>&1 </dev/null
  fi
}

echo "=== Phase 1: install ${PREV} ==="
if [ "$KIND" = deb ]; then apt-get update >/dev/null 2>&1 </dev/null; fi
install_pkg "$(ls /pkgs | grep "$PREV" | grep "$KIND$" | head -1)"
V=$(voxtype --version 2>/dev/null)
[ "$V" = "voxtype $PREV" ] && ok "$PREV installed" || bad "version: got '$V', want 'voxtype $PREV'"

echo "=== Phase 2: real config loads on ${PREV} ==="
mkdir -p ~/.config/voxtype
cat > ~/.config/voxtype/config.toml << 'EOF'
engine = "whisper"

[whisper]
model = "base.en"

[text]
spoken_punctuation = true

[text.replacements]
"upgrade marker" = "UPGRADE-MARKER-SURVIVED"

[osd]
enabled = true
opacity = 0.9
EOF
voxtype info engines >/dev/null 2>&1 && ok "config loads (info engines)" || bad "config load"
A=$(md5sum ~/.config/voxtype/config.toml | cut -d' ' -f1)

echo "=== Phase 3: upgrade to ${NEXT} ==="
install_pkg "$(ls /pkgs | grep "$NEXT" | grep "$KIND$" | head -1)"
V=$(voxtype --version 2>/dev/null)
# The full version: a release candidate reports its -rcN suffix.
[ "$V" = "voxtype $NEXT" ] && ok "upgraded ($V)" || bad "upgrade version: got '$V', want 'voxtype $NEXT'"

echo "=== Phase 4: config survived byte-for-byte, loads on new binary ==="
B=$(md5sum ~/.config/voxtype/config.toml | cut -d' ' -f1)
[ "$A" = "$B" ] && ok "config untouched by upgrade" || bad "config CHANGED by upgrade"
voxtype info engines >/dev/null 2>&1 && ok "old config loads on new binary" || bad "old config load on new binary"
grep -q "UPGRADE-MARKER-SURVIVED" ~/.config/voxtype/config.toml && ok "replacement rule intact" || bad "replacement rule lost"
voxtype config get osd.opacity 2>/dev/null | grep -q 0.9 && ok "osd settings intact" || bad "osd settings lost"

echo "=== Phase 5: new release surface present ==="
voxtype info styles >/dev/null 2>&1 && ok "info styles exists" || bad "info styles"
voxtype info variants >/dev/null 2>&1 && ok "info variants exists" || bad "info variants"
ls /usr/lib/voxtype/ | grep -q baseline && ok "baseline binary shipped" || bad "baseline binary missing"
[ -d /usr/share/voxtype/osd ] && ok "OSD styles shipped" || bad "/usr/share/voxtype/osd missing"
[ -d /usr/share/voxtype/osd-recipes ] && ok "OSD recipes shipped" || bad "osd-recipes missing"
[ -d /usr/share/voxtype/quickshell ] && ok "quickshell tree shipped" || bad "quickshell tree missing"
voxtype info styles 2>/dev/null | grep -q "aegis-hud" && ok "aegis-hud discoverable" || bad "aegis-hud not discovered"

echo "=== Phase 6: Omarchy Settings plugin shipped in the versioned layout ==="
PLUGIN=/usr/share/voxtype/omarchy-plugin/io.voxtype.settings
[ -f "$PLUGIN/manifest.json" ] && ok "plugin manifest shipped" || bad "$PLUGIN/manifest.json missing"
[ -f "$PLUGIN/${NEXT}/Panel.qml" ] && ok "Panel.qml under ${NEXT}/" || bad "$PLUGIN/${NEXT}/Panel.qml missing"
ls "$PLUGIN/${NEXT}/components/"*.qml >/dev/null 2>&1 && ok "plugin components shipped" || bad "plugin components missing"
grep -qF "\"version\": \"${NEXT}\"" "$PLUGIN/manifest.json" 2>/dev/null \
  && ok "plugin manifest version is ${NEXT}" || bad "plugin manifest version is not ${NEXT}"
grep -qF "\"panel\": \"${NEXT}/Panel.qml\"" "$PLUGIN/manifest.json" 2>/dev/null \
  && ok "panel entry point is ${NEXT}/Panel.qml" || bad "panel entry point is not ${NEXT}/Panel.qml"

FINISHED=1
