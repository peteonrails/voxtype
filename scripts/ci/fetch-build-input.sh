#!/usr/bin/env bash
# Download one build input and keep it only if it is the pinned bytes.
#
# The release Dockerfiles use this for anything they fetch that is not a
# distribution package or a Cargo dependency, so a build never continues with
# an archive it cannot identify. The URL is whatever the Dockerfile was told
# to use (the project's mirror by default, or a copy the builder points
# VOXTYPE_BUILD_INPUTS_URL at); the sha256 is fixed in the Dockerfile and is
# the same for every source. testing/test-build-input-pins.py covers it.
set -euo pipefail

url=${1:?usage: fetch-build-input.sh URL SHA256 DESTINATION}
expected=${2:?usage: fetch-build-input.sh URL SHA256 DESTINATION}
destination=${3:?usage: fetch-build-input.sh URL SHA256 DESTINATION}

if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
    echo "FAIL: build input $url has no sha256 pin (got '$expected')." >&2
    exit 1
fi

mkdir -p "$(dirname -- "$destination")"
partial=$(mktemp "${destination}.XXXXXX")
trap 'rm -f -- "$partial"' EXIT

if ! curl --fail --silent --show-error --location --retry 3 \
        --connect-timeout 15 --max-time 1800 -o "$partial" "$url"; then
    echo "FAIL: could not download build input $url" >&2
    echo "  If the mirror is unreachable from here, serve your own copy and pass" >&2
    echo "  --build-arg VOXTYPE_BUILD_INPUTS_URL=<base url>. The sha256 pin still applies." >&2
    exit 1
fi

actual=$(sha256sum "$partial" | cut -d' ' -f1)
if [[ "$actual" != "$expected" ]]; then
    echo "FAIL: build input $url is not the pinned file." >&2
    echo "  expected sha256 $expected" >&2
    echo "  received sha256 $actual ($(stat -c %s "$partial") bytes)" >&2
    exit 1
fi

mv -f -- "$partial" "$destination"
echo "Verified build input $(basename -- "$destination"): sha256 $expected"
