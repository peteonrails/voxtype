---
name: aur-publish
description: Publish voxtype to AUR. Updates PKGBUILDs, fills checksums from the CI-signed SHA256SUMS.txt, and pushes to AUR. Covers the stable packages (voxtype, voxtype-bin) and the rc channel (voxtype-bin-rc). Use after a GitHub release is published.
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Edit
  - Glob
---

# AUR Publish

Update and publish voxtype packages to the Arch User Repository.

## Packages

| Package | Type | Tracks | Location |
|---------|------|--------|----------|
| `voxtype` | Source build | Latest stable tag | `packaging/arch/` |
| `voxtype-bin` | Pre-built binaries | Latest stable release | `packaging/arch-bin/` |
| `voxtype-bin-rc` | Pre-built binaries | Latest pre-release tag | `packaging/arch-bin-rc/` |

All three are nested git repos inside the main checkout's `packaging/`
directory (gitignored there; each pushes to `ssh://aur.archlinux.org/<name>.git`).

## Prerequisites

- GitHub release already published with the full asset set and
  `SHA256SUMS.txt` uploaded (CI signs it with `9CCF7915B750CAE8B095ED1AA3FC9F33FD209279`)
- AUR maintainer GPG key configured: `E79F5BAF8CD51A806AA27DBB7DA2709247D75BC6`

## Checksums: the rules (learned the hard way at 1.1.0)

1. **The CI-signed `SHA256SUMS.txt` from the release is the only source of
   truth for binary sums.** Never trust a locally downloaded file's hash on
   its own - cross-check every non-SKIP sum in the PKGBUILD against that
   file before pushing.
2. **Do not let `updpkgsums` rewrite this PKGBUILD.** It strips the
   per-entry comments (which are load-bearing) and has left orphan lines
   that break sourcing. Use it at most to download the sources, then write
   sums yourself.
3. **`source_x86_64` and `source_aarch64` share local filenames**
   (`voxtype-$pkgver-osd` names both arches' downloads). Hashing local
   files therefore poisons the aarch64 array with x86_64 sums. Take the
   aarch64 sums from `SHA256SUMS.txt` by remote filename, never from disk.
4. `.asc` entries are `SKIP` (signatures are verified via `validpgpkeys`).
   The source tarball is not in SHA256SUMS.txt: hash the downloaded
   tarball, then `gpg --verify` its `.asc` to prove the hash is of the
   authentic file.

## Workflow for voxtype-bin

```bash
VERSION=1.1.0
cd packaging/arch-bin

# 1. Bump version
sed -i "s/^pkgver=.*/pkgver=${VERSION}/; s/^pkgrel=.*/pkgrel=1/" PKGBUILD

# 2. Fetch the authoritative sums
curl -sL "https://github.com/peteonrails/voxtype/releases/download/v${VERSION}/SHA256SUMS.txt" -o /tmp/sums.txt

# 3. Update every sums array from /tmp/sums.txt by REMOTE filename,
#    preserving the comments (script it; do not use updpkgsums to write).
#    Raw-file sources (config.toml, service, completions, LICENSE, README,
#    desktop entry, launcher) are fetched at the tag and hashed locally.

# 4. Cross-check: every non-SKIP binary sum in the PKGBUILD must appear in
#    /tmp/sums.txt. Zero misses or stop.

# 5. Update the post_upgrade highlights in voxtype-bin.install for the new
#    version. The banner header reads pacman's $1 - never hardcode a
#    version in that file (a hardcoded 1.0.0 announced 1.1.0's features).

# 6. Regenerate, verify, push
makepkg --printsrcinfo > .SRCINFO
bash -n PKGBUILD && bash -n voxtype-bin.install
git add PKGBUILD .SRCINFO voxtype-bin.install
git commit -S -m "Update to ${VERSION}"
git push
```

## Workflow for voxtype (source package)

Same shape, two sources: the GitHub archive tarball (hash it, then
`gpg --verify` the release's `.asc` against it) and its signature (`SKIP`).
The tag must carry a `Cargo.lock` matching `Cargo.toml`'s version or
`cargo fetch --locked` fails for every builder (the v0.4.6 incident).

## Workflow for voxtype-bin-rc

Tracks pre-release tags. AUR forbids hyphens in `pkgver`, so `v1.2.0-rc1`
becomes `pkgver=1.2.0.rc1` and the PKGBUILD's `_upstream_ver` computes the
real tag back (`${pkgver/.rc/-rc}`). Everything else follows the
voxtype-bin workflow against the pre-release's SHA256SUMS.txt. Note: as of
1.1.0 this package has never been published; its first push creates it.

## Important Rules

**Always bump `pkgver`, never just `pkgrel`, when binaries change.** The
download URLs include `pkgver` only; a pkgrel-only bump leaves the URL
unchanged and AUR helpers serve cached files against new checksums.

**`pkgrel` bumps are for packaging-only changes** (install script, deps).

**Never re-upload different binaries to an existing GitHub release.** Cut a
new version instead.

## Checklist

- [ ] GitHub release exists, all assets uploaded, SHA256SUMS.txt present
- [ ] Binaries validated (/validate-binaries; baseline needs the behavioral
      floor test, not just --version)
- [ ] `pkgver` bumped, `pkgrel` reset to 1
- [ ] Every binary sum cross-checked against the CI-signed SHA256SUMS.txt
- [ ] aarch64 sums taken from SHA256SUMS.txt, not from local files
- [ ] Tarball hash GPG-verified against its .asc
- [ ] post_upgrade highlights updated (no hardcoded versions)
- [ ] `.SRCINFO` regenerated; `bash -n` passes on PKGBUILD and .install
- [ ] Committed signed, pushed to AUR
