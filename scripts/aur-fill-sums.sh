#!/bin/bash
# Fill the voxtype-bin PKGBUILD's checksums for a release, then regenerate
# .SRCINFO with every variant listed.
#
# Usage: scripts/aur-fill-sums.sh <version> <pkgbuild-dir>
#
#   <version>       the release tag without the v, e.g. 1.2.1 or 1.2.1-rc1.
#                   The PKGBUILD's pkgver must already name it (1.2.1.rc1
#                   for a release candidate).
#   <pkgbuild-dir>  a checkout of the voxtype-bin AUR repo
#
# Binary checksums come from the release's SHA256SUMS.txt, accepted only with
# a good signature from the CI release key. The arch-independent sources
# (config, unit, completions, license, README, desktop entry, launcher) are
# not in SHA256SUMS.txt; their checksums are taken from the signed source
# archive, and each raw.githubusercontent.com download must match its copy
# in that archive.
#
# updpkgsums cannot do this: the PKGBUILD builds its per-arch source arrays
# from the _vt_assets_<arch> tables for the variants this machine selects,
# so makepkg -g only sees part of the release.
set -euo pipefail

CI_KEY="9CCF7915B750CAE8B095ED1AA3FC9F33FD209279"
MAINTAINER_KEY="E79F5BAF8CD51A806AA27DBB7DA2709247D75BC6"
REPO_URL="https://github.com/peteonrails/voxtype"

die() {
    echo "aur-fill-sums: $*" >&2
    exit 1
}

[[ $# -eq 2 ]] || die "usage: $0 <version> <pkgbuild-dir>"
VERSION=${1#v}
PKG_DIR=$(realpath "$2")
PKGBUILD="$PKG_DIR/PKGBUILD"
[[ -f $PKGBUILD ]] || die "no PKGBUILD in $PKG_DIR"
for tool in curl gpg sha256sum tar makepkg; do
    command -v "$tool" >/dev/null || die "$tool not found"
done

# Read the PKGBUILD's own view of the version and its sources, with every
# variant selected and no host state.
pkgbuild_eval() {
    env -i PATH="$PATH" CARCH=x86_64 VOXTYPE_VARIANTS=all _vt_root=/nonexistent \
        bash -c 'source "$1" >/dev/null 2>&1 || exit 1; eval "$2"' _ "$PKGBUILD" "$1"
}
PKG_VT_VER=$(pkgbuild_eval 'echo "$_vt_ver"')
[[ $PKG_VT_VER == "$VERSION" ]] ||
    die "the PKGBUILD is for $PKG_VT_VER, not $VERSION; set pkgver first (an rc uses the dot form, 1.2.1.rc1)"

WORK=$(mktemp -d "${XDG_CACHE_HOME:-$HOME/.cache}/voxtype-aur-fill-sums.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

fetch() {
    curl -fsSL --retry 3 -o "$2" "$1" || die "download failed: $1"
}

# Check that $2 is a good detached signature of $1 by one of the keys in
# the remaining arguments (primary or signing-subkey fingerprint).
verify_sig() {
    local file=$1 sig=$2 status
    shift 2
    status=$(gpg --batch --status-fd 1 --verify "$sig" "$file" 2>/dev/null) ||
        die "bad or unverifiable signature on ${file##*/} (is $CI_KEY in your keyring?)"
    local key
    for key in "$@"; do
        if grep -q "^\[GNUPG:\] VALIDSIG .*$key" <<<"$status"; then
            return 0
        fi
    done
    die "${file##*/} is signed, but not by an expected key ($*)"
}

RELEASE="$REPO_URL/releases/download/v$VERSION"
echo "==> SHA256SUMS.txt for v$VERSION"
fetch "$RELEASE/SHA256SUMS.txt" "$WORK/SHA256SUMS.txt"
fetch "$RELEASE/SHA256SUMS.txt.asc" "$WORK/SHA256SUMS.txt.asc"
verify_sig "$WORK/SHA256SUMS.txt" "$WORK/SHA256SUMS.txt.asc" "$CI_KEY"
echo "    good signature from the CI release key $CI_KEY"

# Binary tables: rewrite the checksum field of every 'variant|asset|sum' line
# inside _vt_assets_<arch>=( ... ).
fill_table() {
    local arch=$1
    awk -v arch="$arch" -v ver="$VERSION" -v sums="$WORK/SHA256SUMS.txt" '
        BEGIN {
            while ((getline line < sums) > 0) {
                split(line, f, /[ \t]+\*?/)
                sum[f[2]] = f[1]
            }
            missing = 0
        }
        $0 ~ "^_vt_assets_" arch "=\\(" { inside = 1; print; next }
        inside && /^\)/ { inside = 0; print; next }
        inside && match($0, /^[[:space:]]*\x27[^|\x27]+\|[^|\x27]+\|[^\x27]*\x27/) {
            indent = $0; sub(/\x27.*/, "", indent)
            body = $0; sub(/^[[:space:]]*\x27/, "", body); sub(/\x27.*/, "", body)
            rest = substr($0, RLENGTH + 1)
            n = split(body, p, "|")
            file = "voxtype-" ver "-linux-" arch "-" p[2]
            if (!(file in sum)) {
                print "aur-fill-sums: " file " is not in SHA256SUMS.txt" > "/dev/stderr"
                missing = 1
                print
                next
            }
            printf "%s\x27%s|%s|%s\x27%s\n", indent, p[1], p[2], sum[file], rest
            next
        }
        { print }
        END { exit missing }
    ' "$PKGBUILD" >"$WORK/PKGBUILD.new" || die "$arch: assets missing from SHA256SUMS.txt"
    cp "$WORK/PKGBUILD.new" "$PKGBUILD"
}
echo "==> binary checksums"
for arch in $(pkgbuild_eval 'echo "${arch[*]}"'); do
    fill_table "$arch"
    count=$(pkgbuild_eval "declare -n t=_vt_assets_$arch; echo \${#t[@]}")
    echo "    $arch: $count assets"
done

# Arch-independent sources, in the order of the PKGBUILD's source array.
echo "==> source archive"
mapfile -t SOURCES < <(pkgbuild_eval 'printf "%s\n" "${source[@]}"')
fetch "$REPO_URL/archive/refs/tags/v$VERSION.tar.gz" "$WORK/source.tar.gz"
fetch "$RELEASE/voxtype-$VERSION.tar.gz.asc" "$WORK/source.tar.gz.asc"
verify_sig "$WORK/source.tar.gz" "$WORK/source.tar.gz.asc" "$CI_KEY" "$MAINTAINER_KEY"
echo "    good signature on voxtype-$VERSION.tar.gz"

NEW_SUMS=()
for entry in "${SOURCES[@]}"; do
    name=${entry%%::*}
    url=${entry#*::}
    if [[ $name == *.asc ]]; then
        NEW_SUMS+=(SKIP)
        continue
    fi
    case "$url" in
        "$REPO_URL/archive/refs/tags/v$VERSION.tar.gz")
            NEW_SUMS+=("$(sha256sum "$WORK/source.tar.gz" | cut -d' ' -f1)")
            ;;
        "https://raw.githubusercontent.com/peteonrails/voxtype/v$VERSION/"*)
            path=${url#https://raw.githubusercontent.com/peteonrails/voxtype/v$VERSION/}
            fetch "$url" "$WORK/raw"
            tar -xzOf "$WORK/source.tar.gz" "voxtype-$VERSION/$path" >"$WORK/archived" 2>/dev/null ||
                die "$path is not in the source archive"
            cmp -s "$WORK/raw" "$WORK/archived" ||
                die "$url differs from voxtype-$VERSION/$path in the signed source archive"
            NEW_SUMS+=("$(sha256sum "$WORK/archived" | cut -d' ' -f1)")
            ;;
        *)
            die "no checksum source for $name ($url)"
            ;;
    esac
    echo "    $name"
done

# Rewrite the entries of sha256sums=( ... ) in order, keeping their comments.
SUMS_FILE="$WORK/sums"
printf '%s\n' "${NEW_SUMS[@]}" >"$SUMS_FILE"
awk -v sums="$SUMS_FILE" '
    BEGIN { while ((getline s < sums) > 0) want[++n] = s; i = 0 }
    /^sha256sums=\(/ { inside = 1; print; next }
    inside && /^\)/ { inside = 0; print; next }
    inside && /^[[:space:]]*\x27/ {
        i++
        line = $0
        sub(/\x27[^\x27]*\x27/, "\x27" want[i] "\x27", line)
        print line
        next
    }
    { print }
    END { if (i != n) { print "aur-fill-sums: sha256sums has " i " entries, source has " n > "/dev/stderr"; exit 1 } }
' "$PKGBUILD" >"$WORK/PKGBUILD.new" || die "could not rewrite sha256sums"
cp "$WORK/PKGBUILD.new" "$PKGBUILD"

echo "==> .SRCINFO (VOXTYPE_VARIANTS=all)"
(cd "$PKG_DIR" && VOXTYPE_VARIANTS=all makepkg --printsrcinfo >.SRCINFO)

if git -C "$PKG_DIR" rev-parse --git-dir >/dev/null 2>&1; then
    git -C "$PKG_DIR" --no-pager diff --stat -- PKGBUILD .SRCINFO
fi
echo "Done. Run scripts/test-aur-bin.sh --sums <SHA256SUMS.txt> $PKG_DIR before committing."
