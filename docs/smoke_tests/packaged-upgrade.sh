#!/bin/bash
# Packaged upgrade test: $1 = previous version, $2 = next version, $3 = deb|rpm
# Runs inside a container with the packages mounted at /pkgs (read-only).
# See packaged-upgrade.md for what each phase proves.
set -u
PREV="$1"; NEXT="$2"; KIND="$3"
PASS=0; FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

install_pkg() {
  local f="$1"
  if [ "$KIND" = deb ]; then
    apt-get install -y "/pkgs/$f" >/dev/null 2>&1
  else
    dnf install -y "/pkgs/$f" >/dev/null 2>&1
  fi
}

echo "=== Phase 1: install ${PREV} ==="
if [ "$KIND" = deb ]; then apt-get update >/dev/null 2>&1; fi
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
[ "$V" = "voxtype ${NEXT%%-*}" ] && ok "upgraded ($V)" || bad "upgrade version: got '$V'"

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

echo
echo "RESULT: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
