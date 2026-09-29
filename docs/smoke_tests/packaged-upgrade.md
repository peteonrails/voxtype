# Packaged Upgrade (deb and rpm)

Prove an in-place package upgrade from the previous stable release
preserves the user's config byte-for-byte, that the old config loads on the
new binary, and that the new release's file surface actually arrives. Run
before every stable tag, against the release-candidate packages.

Run inside Docker so nothing touches the host. Both containers follow the
same script; only the package manager differs.

```bash
PREV=1.0.1        # previous stable
NEXT=1.1.0-rc5    # candidate under test
WORK=$(mktemp -d)
cd "$WORK"
gh release download "v${PREV}" --repo peteonrails/voxtype -p "voxtype_${PREV}-1_amd64.deb" -p "voxtype-${PREV}-1.x86_64.rpm"
gh release download "v${NEXT}" --repo peteonrails/voxtype -p "voxtype_${NEXT}-1_amd64.deb" -p "voxtype-${NEXT}-1.x86_64.rpm"

# deb path
sudo docker run --rm -v "$WORK":/pkgs:ro ubuntu:24.04 bash -s "$PREV" "$NEXT" deb < docs/smoke_tests/packaged-upgrade.sh
# rpm path
sudo docker run --rm -v "$WORK":/pkgs:ro fedora:40 bash -s "$PREV" "$NEXT" rpm < docs/smoke_tests/packaged-upgrade.sh
```

What the script asserts, in order:

1. The previous version installs and reports its version.
2. A real config (with a replacement rule and `[osd]` settings) loads:
   `voxtype info engines` exits 0.
3. The in-place upgrade succeeds and `--version` reports the new version.
4. **The user config is byte-identical after the upgrade** (md5 compare -
   the packages must never touch it).
5. The old config loads on the new binary (the backwards-compat gate).
6. The new release's surface is present: `info styles` and `info variants`
   exist, the baseline binary is in `/usr/lib/voxtype`, style packages are
   at `/usr/share/voxtype/osd` with `aegis-hud` discoverable, recipes and
   the quickshell tree shipped.

Harness lessons from the 1.1.0 run, so they are not relearned:

- Assert through schema-valid commands only. `config get` on a map key
  (`text.replacements`) or a nonexistent key (`audio.sample_rate`) exits 2
  by design; a harness using them reports false failures. Check the
  replacement rule by grepping the config file.
- Fedora's base image has no `diff`; compare with `md5sum`.
- Install with the package manager (`apt-get install ./pkg.deb`,
  `dnf install ./pkg.rpm`) so dependencies resolve; `rpm --nodeps` leaves
  libasound missing and every binary failing for the wrong reason.
