#!/bin/bash
# Check that every pinned download on the website and in INSTALL.md
# points at the version in Cargo.toml
#
# The release checklist only named website/index.html, so the /download
# page sat on v0.7.1 while users installed from it (#844). CI runs this
# against main, which only moves at release time; on dev the crate version
# is bumped before the release exists, so the links would 404 there.
#
# AppImage lines are skipped: no release has shipped one since v1.0.0, and
# the tab's fate is tracked in #622 / #631.
#
# Usage:
#   ./scripts/check-website-versions.sh [version]

set -e

VERSION="${1:-$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)}"

FILES=(
    website/index.html
    website/download/index.html
    website/compare/*.html
    docs/INSTALL.md
)

# Pull each pinned version out of each line, so a line that mentions the
# right version once and a stale one elsewhere still gets caught.
STALE=$(grep -nE 'releases/download/v|voxtype[_-][0-9]|version-badge">v|peteonrails/voxtype/v[0-9]' "${FILES[@]}" \
    | grep -v AppImage \
    | grep -oE '^[^:]+:[0-9]+:|(/v|>v|voxtype[_-])[0-9]+\.[0-9]+\.[0-9]+' \
    | awk -v want="$VERSION" '
        /:$/ { loc = $0; next }
        { ver = $0; sub(/^(\/v|>v|voxtype[_-])/, "", ver) }
        ver != want { print loc " v" ver }' \
    | sort -u)

if [[ -n "$STALE" ]]; then
    echo "ERROR: these lines pin a version other than v${VERSION} (Cargo.toml):"
    echo "$STALE"
    echo ""
    echo "Update them per the release checklist in CLAUDE.md."
    exit 1
fi

echo "Website and INSTALL.md download links all pin v${VERSION}."
