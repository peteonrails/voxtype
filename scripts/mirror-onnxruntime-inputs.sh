#!/usr/bin/env bash
# mirror-onnxruntime-inputs.sh - build the two ONNX Runtime inputs the Linux
# release Dockerfiles fetch, so those builds do not depend on
# github.com/microsoft/onnxruntime being reachable (it returned 404 for hours
# on 2026-11-18 and took the onnx-cuda-13 and onnx-migraphx jobs with it).
#
#   source  ONNX Runtime v1.24.2 source, which Dockerfile.onnx-migraphx
#           compiles with the MIGraphX provider (no prebuilt exists).
#   cuda13  The three libraries Dockerfile.onnx-cuda-13 takes from
#           Microsoft's 1.24.4 CUDA 13 archive and ships next to the binary.
#
# Each bundle is a deterministic .tar.gz (sorted names, fixed mtime, owner 0,
# gzip -n), pinned here by sha256. The same pin is in the Dockerfile that
# fetches it through scripts/ci/fetch-build-input.sh, and
# testing/test-build-input-pins.py fails CI when the two disagree.
#
# Layout, under $STAGE and on the mirror:
#   build-inputs/onnxruntime/v1.24.2-source/
#   build-inputs/onnxruntime/v1.24.4-linux-x64-gpu_cuda13/
# Each directory holds the bundle, LICENSE, ThirdPartyNotices.txt,
# ATTRIBUTION.txt and a manifest.json listing every file with its size and
# sha256, like the model directories scripts/mirror-flat-models-to-r2.sh
# writes. build-inputs/ is separate from the model prefixes: the runtime
# never reads it.
#
# Where the bytes come from:
#   source  `git fetch --depth 1` of the pinned tag from $ORT_SOURCE_REPO
#           (default: Microsoft's repository; a local clone works too). The
#           commit must be the pinned commit, and the unpacked bundle must
#           hash back to that commit's git tree. Submodules are left out:
#           see ATTRIBUTION.txt in the staged directory.
#   cuda13  Microsoft's archive when it downloads; otherwise the copies
#           Voxtype published in its own signed release $VOXTYPE_RELEASE
#           (needs gh, gpg and the CI release key). Either way each library
#           must match its sha256 pin below. LICENSE and
#           ThirdPartyNotices.txt come from Microsoft's onnxruntime 1.24.4
#           wheel on PyPI, pinned by sha256.
#
# Usage: mirror-onnxruntime-inputs.sh [stage|upload|verify|all] [source|cuda13 ...]
#   stage   build the bundle and check it against the pinned sha256
#   upload  rclone copy --immutable to r2:voxtype-models (never overwrites)
#   verify  re-hash every file and the manifest via the public URL
#   all     stage, upload and verify (default)
#
# Prerequisites: git, GNU tar, gzip, curl, sha256sum, python3; for upload an
# rclone remote named `r2` on the Cloudflare account holding `voxtype-models`.
#
# Staging uses $STAGE (default ~/.cache/voxtype-mirror/build-inputs). The
# bundles are a few hundred MB, so keep it on disk, never on a tmpfs /tmp.
# gzip output can differ between gzip releases. If a rebuilt bundle misses
# its pin, compare the uncompressed sha256 the script prints with the
# manifest's: when those match, only the compression differs and the
# mirrored copy is still the one to use.
set -euo pipefail
umask 022
export LC_ALL=C TZ=UTC

STAGE="${STAGE:-${XDG_CACHE_HOME:-$HOME/.cache}/voxtype-mirror/build-inputs}"
REMOTE="r2:voxtype-models"
PUBLIC="${PUBLIC:-https://models.voxtype.io}"
PREFIX="build-inputs/onnxruntime"

SOURCE_VERSION="1.24.2"
SOURCE_REPO="${ORT_SOURCE_REPO:-https://github.com/microsoft/onnxruntime.git}"
SOURCE_COMMIT="058787ceead760166e3c50a0a4cba8a833a6f53f"
SOURCE_TREE="e42d56bb417bdecf5e1f4f27a75ce01709acb302"
SOURCE_DIR="v${SOURCE_VERSION}-source"
SOURCE_FILE="onnxruntime-${SOURCE_VERSION}-source.tar.gz"
SOURCE_SHA256="41f2cd14842a288887aeef5942d44eb4ce18f031db30bd1207eed120aa5edb69"
# path|commit of each submodule in the pinned tree. None is shipped.
SOURCE_SUBMODULES=(
"cmake/external/emsdk|c0bb220cb6e6f4e0fabb6f6db9efd53390ef5e56"
"cmake/external/libprotobuf-mutator|7a2ed51a6b682a83e345ff49fc4cfd7ca47550db"
"cmake/external/onnx|d3f6b795aedb48eaecc881bf5e8f5dd6efbe25b3"
)

CUDA13_VERSION="1.24.4"
CUDA13_NAME="onnxruntime-linux-x64-gpu_cuda13-${CUDA13_VERSION}"
CUDA13_URL="https://github.com/microsoft/onnxruntime/releases/download/v${CUDA13_VERSION}/${CUDA13_NAME}.tgz"
CUDA13_DIR="v${CUDA13_VERSION}-linux-x64-gpu_cuda13"
CUDA13_FILE="${CUDA13_NAME}-runtime.tar.gz"
CUDA13_SHA256="25ae3230c9d0a044640f65dc3824a02c650132be2cfc44874419c34a34c0e954"
CUDA13_MTIME=1773705600 # 2026-03-17 00:00 UTC, the day 1.24.4 was built
VOXTYPE_RELEASE="${VOXTYPE_RELEASE:-v1.2.2}"
VOXTYPE_RELEASE_KEY="9CCF7915B750CAE8B095ED1AA3FC9F33FD209279"
# file|sha256 of the libraries, identical in Voxtype 0.7.4, 1.2.2 and 1.3.0-rc3
CUDA13_LIBS=(
"libonnxruntime.so.${CUDA13_VERSION}|1aacefdf0b4afa145d410b2381bbc3db3d978c485fb182c42a2b0b09f91f5310"
"libonnxruntime_providers_cuda.so|b6cb7744d0efd2faced5c83ead374c13e7f2630b5a249ffaf393cdb1e092c92b"
"libonnxruntime_providers_shared.so|c6a12593396095f5670160e284c35d1700b7708cf3037b7042e2a5200ccae772"
)
WHEEL_URL="https://files.pythonhosted.org/packages/6d/ab/5b68110e0460d73fad814d5bd11c7b1ddcce5c37b10177eb264d6a36e331/onnxruntime-1.24.4-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl"
WHEEL_SHA256="0d640eb9f3782689b55cfa715094474cd5662f2f137be6a6f847a594b6e9705c"
LICENSE_SHA256="2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c"
NOTICES_SHA256="0e07b95f3a8d6230037707c5c4a2b554d12c4cb67369669ac255635528ffcee2"

sha() { sha256sum "$1" | cut -d' ' -f1; }
die() { echo "FAIL: $*" >&2; exit 1; }
pinned() { # file expected what
  local got; got=$(sha "$1")
  [[ "$got" == "$2" ]] || die "$3: sha256 $got, pinned $2"
}

# Deterministic archive of one directory: same inputs, same bytes.
pack() { # parent dir mtime output
  tar --sort=name --format=gnu --mtime="@$3" --owner=0 --group=0 --numeric-owner \
      -cf "$4.tar" -C "$1" "$2"
  echo "uncompressed sha256 $(sha "$4.tar")  $(basename "$4" .gz)"
  gzip -n -9 -c "$4.tar" > "$4.part"
  rm -f "$4.tar"; mv "$4.part" "$4"
}

# Hash an unpacked source tree the way git does and compare it with the
# upstream tree, so the bundle is tied to the commit GitHub signed and not
# only to a hash this script produced.
check_source_tree() { # dir
  local g e tree; g=$(mktemp -d "$STAGE/.work/treecheck.XXXXXX")
  git init -q "$g"
  # Undo exactly what a Linux checkout does to this tree and nothing more:
  # only *.sln is converted (to CRLF) on the way out. Two files are stored
  # with CRLF upstream, and git's default text=auto would rewrite them here.
  printf '* -text\n*.sln text eol=crlf\n' > "$g/.git/info/attributes"
  local -a G=(git --git-dir="$g/.git" --work-tree="$1" -c core.autocrlf=false -c core.fileMode=true)
  (cd "$1" && "${G[@]}" add -A -f .)
  for e in "${SOURCE_SUBMODULES[@]}"; do
    "${G[@]}" update-index --add --cacheinfo "160000,${e#*|},${e%%|*}"
  done
  tree=$("${G[@]}" write-tree)
  rm -rf "$g"
  [[ "$tree" == "$SOURCE_TREE" ]] || die "source tree hashes to $tree, upstream tree is $SOURCE_TREE"
  echo "TREE OK  $SOURCE_TREE (commit ${SOURCE_COMMIT:0:10})"
}

write_manifest() { # dir name file license json-extra
  python3 -I - "$@" <<'PY'
import hashlib, json, os, sys
d, name, bundle, lic, extra = sys.argv[1:]
def entry(p):
    s = hashlib.sha256()
    with open(os.path.join(d, p), "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            s.update(b)
    return {"path": p, "size": os.path.getsize(os.path.join(d, p)), "sha256": s.hexdigest()}
m = {"version": 1, "name": name,
     "files": [entry(p) for p in [bundle, "LICENSE", "ThirdPartyNotices.txt", "ATTRIBUTION.txt"]],
     "license": {"license": lic},
     "object_prefix": "build-inputs/onnxruntime/" + os.path.basename(d)}
m.update(json.loads(extra))
with open(os.path.join(d, "manifest.json"), "w") as f:
    json.dump(m, f, indent=2)
    f.write("\n")
PY
}

stage_source() {
  local d="$STAGE/$PREFIX/$SOURCE_DIR" w="$STAGE/.work/source"
  rm -rf "$w"; mkdir -p "$d" "$w/pack/onnxruntime-${SOURCE_VERSION}-source"
  local top="$w/pack/onnxruntime-${SOURCE_VERSION}-source" repo="$SOURCE_REPO"
  [[ -d "$repo" ]] && repo="file://$(realpath "$repo")"

  git init -q --bare "$w/repo.git"
  git -C "$w/repo.git" fetch -q --depth 1 "$repo" "refs/tags/v${SOURCE_VERSION}" \
    || die "cannot fetch v${SOURCE_VERSION} from $repo (set ORT_SOURCE_REPO to a clone that has it)"
  local commit; commit=$(git -C "$w/repo.git" rev-parse 'FETCH_HEAD^{commit}')
  [[ "$commit" == "$SOURCE_COMMIT" ]] || die "v${SOURCE_VERSION} at $repo is $commit, pinned $SOURCE_COMMIT"
  local mtime; mtime=$(git -C "$w/repo.git" show -s --format=%ct "$commit")

  # The tree as a Linux checkout has it. Submodule paths come out as empty
  # directories; the .git directory and any build output never enter.
  git -C "$w/repo.git" -c core.autocrlf=false -c core.eol=lf -c tar.umask=0022 \
      archive --format=tar --prefix=onnxruntime/ "$commit" | tar -x -C "$top"
  check_source_tree "$top/onnxruntime"
  pinned "$top/onnxruntime/LICENSE" "$LICENSE_SHA256" "LICENSE in the source tree"
  pinned "$top/onnxruntime/ThirdPartyNotices.txt" "$NOTICES_SHA256" "ThirdPartyNotices.txt in the source tree"

  # The raw commit object. Dockerfile.onnx-migraphx rebuilds a one-commit
  # shallow repository from it so ONNX Runtime's build stamps the same
  # "git-commit-id" into the binary as a real clone did.
  git -C "$w/repo.git" cat-file commit "$commit" > "$top/upstream-commit"
  cat > "$top/MIRROR.txt" <<TXT
ONNX Runtime v${SOURCE_VERSION} source, mirrored by the Voxtype project.

onnxruntime/      the tree of https://github.com/microsoft/onnxruntime at tag
                  v${SOURCE_VERSION}, commit ${SOURCE_COMMIT},
                  git tree ${SOURCE_TREE}, unmodified.
upstream-commit   that commit as a raw git object (it carries GitHub's
                  signature). \`git hash-object -t commit upstream-commit\`
                  prints the commit id.

Not included: the .git directory, and the contents of the three submodules
(cmake/external/emsdk, cmake/external/libprotobuf-mutator,
cmake/external/onnx). Their paths are empty directories. A build that needs
them, such as WebAssembly, the fuzz tests or the Python wheel, cannot be made
from this archive.

License: MIT, see onnxruntime/LICENSE and onnxruntime/ThirdPartyNotices.txt.
Built by scripts/mirror-onnxruntime-inputs.sh in the Voxtype repository.
TXT
  pack "$w/pack" "onnxruntime-${SOURCE_VERSION}-source" "$mtime" "$w/$SOURCE_FILE" | tee "$w/uncompressed"
  pinned "$w/$SOURCE_FILE" "$SOURCE_SHA256" "$SOURCE_FILE"

  cp "$top/onnxruntime/LICENSE" "$top/onnxruntime/ThirdPartyNotices.txt" "$d/"
  mv "$w/$SOURCE_FILE" "$d/$SOURCE_FILE"
  cat > "$d/ATTRIBUTION.txt" <<TXT
Build input: ONNX Runtime v${SOURCE_VERSION} source
Mirrored from: https://github.com/microsoft/onnxruntime at tag v${SOURCE_VERSION}
Commit: ${SOURCE_COMMIT}
Git tree: ${SOURCE_TREE}
Used by: Dockerfile.onnx-migraphx, which compiles it with the MIGraphX provider.
The archive holds the unmodified tree of that commit plus the raw commit
object. It omits the .git directory and the contents of the submodules
cmake/external/emsdk (WebAssembly toolchain), cmake/external/libprotobuf-mutator
(fuzz tests) and cmake/external/onnx (Python wheel and tests; the build takes
ONNX from the archive pinned in cmake/deps.txt instead).
License: MIT. See LICENSE and ThirdPartyNotices.txt.
Distribution does not imply endorsement or transfer trademark rights.
TXT
  local subs; subs=$(printf '"%s",' "${SOURCE_SUBMODULES[@]%%|*}")
  write_manifest "$d" "onnxruntime-${SOURCE_VERSION}-source" "$SOURCE_FILE" "MIT" \
    "{\"origin\":{\"repo\":\"https://github.com/microsoft/onnxruntime\",\"tag\":\"v${SOURCE_VERSION}\",\"commit\":\"${SOURCE_COMMIT}\",\"tree\":\"${SOURCE_TREE}\"},\"uncompressed_sha256\":\"$(cut -d' ' -f3 "$w/uncompressed")\",\"omitted_submodules\":[${subs%,}]}"
  rm -rf "$w"
  echo "STAGED $PREFIX/$SOURCE_DIR/$SOURCE_FILE $(stat -c %s "$d/$SOURCE_FILE") bytes sha256=$(sha "$d/$SOURCE_FILE")"
}

# The three libraries, from Microsoft's archive or from Voxtype's own signed
# release. Which one answered does not change a byte of the bundle.
fetch_cuda13_libs() { # workdir destination
  local w="$1" out="$2" e f status
  if curl -sSfL --retry 2 -o "$w/upstream.tgz" "$CUDA13_URL" 2>/dev/null; then
    echo "libraries: from $CUDA13_URL"
    mkdir -p "$w/upstream"
    tar -xzf "$w/upstream.tgz" -C "$w/upstream" --strip-components=1
    for e in "${CUDA13_LIBS[@]}"; do cp -L "$w/upstream/lib/${e%%|*}" "$out/"; done
    return
  fi
  local v="${VOXTYPE_RELEASE#v}"
  echo "libraries: Microsoft's archive is unreachable, using Voxtype release $VOXTYPE_RELEASE"
  mkdir -p "$w/release"
  gh release download "$VOXTYPE_RELEASE" -R peteonrails/voxtype -D "$w/release" --clobber \
    -p SHA256SUMS.txt -p SHA256SUMS.txt.asc -p "voxtype-${v}-linux-x86_64-onnx-cuda-13.lib*"
  status=$(gpg --status-fd 1 --verify "$w/release/SHA256SUMS.txt.asc" "$w/release/SHA256SUMS.txt" 2>/dev/null) || true
  grep -q "^\[GNUPG:\] VALIDSIG .*${VOXTYPE_RELEASE_KEY}" <<<"$status" \
    || die "SHA256SUMS.txt of $VOXTYPE_RELEASE is not signed by ${VOXTYPE_RELEASE_KEY}"
  for e in "${CUDA13_LIBS[@]}"; do
    f="voxtype-${v}-linux-x86_64-onnx-cuda-13.${e%%|*}"
    [[ "$(awk -v f="$f" '$2 == f { print $1 }' "$w/release/SHA256SUMS.txt")" == "$(sha "$w/release/$f")" ]] \
      || die "$f does not match the signed SHA256SUMS.txt of $VOXTYPE_RELEASE"
    cp "$w/release/$f" "$out/${e%%|*}"
  done
}

stage_cuda13() {
  local d="$STAGE/$PREFIX/$CUDA13_DIR" w="$STAGE/.work/cuda13"
  rm -rf "$w"; mkdir -p "$d" "$w/pack/$CUDA13_NAME/lib" "$w/wheel"
  local top="$w/pack/$CUDA13_NAME" e
  fetch_cuda13_libs "$w" "$top/lib"
  for e in "${CUDA13_LIBS[@]}"; do pinned "$top/lib/${e%%|*}" "${e#*|}" "${e%%|*}"; done

  curl -sSfL --retry 5 -o "$w/wheel.zip" "$WHEEL_URL"
  pinned "$w/wheel.zip" "$WHEEL_SHA256" "onnxruntime ${CUDA13_VERSION} wheel"
  python3 -I -c 'import sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
for n in ("LICENSE", "ThirdPartyNotices.txt"):
    open(sys.argv[2] + "/" + n, "wb").write(z.read("onnxruntime/" + n))' "$w/wheel.zip" "$top"
  pinned "$top/LICENSE" "$LICENSE_SHA256" "LICENSE"
  pinned "$top/ThirdPartyNotices.txt" "$NOTICES_SHA256" "ThirdPartyNotices.txt"

  {
    cat <<TXT
ONNX Runtime ${CUDA13_VERSION} for Linux x64 with CUDA 13: runtime libraries,
mirrored by the Voxtype project.

A subset of Microsoft's release archive
  ${CUDA13_URL}
laid out the same way. It holds the three libraries Voxtype's onnx-cuda-13
build ships next to its binary, unmodified:

TXT
    for e in "${CUDA13_LIBS[@]}"; do printf '  %s  lib/%s\n' "${e#*|}" "${e%%|*}"; done
    cat <<TXT

Not included: the headers, the CMake and pkg-config files, the TensorRT
provider, and the libonnxruntime.so / libonnxruntime.so.1 symlinks.

LICENSE and ThirdPartyNotices.txt are the files Microsoft ships in its
onnxruntime ${CUDA13_VERSION} wheel on PyPI. License: MIT.
The CUDA and cuDNN libraries these load at runtime are NVIDIA's and are not
part of this archive.
Built by scripts/mirror-onnxruntime-inputs.sh in the Voxtype repository.
TXT
  } > "$top/MIRROR.txt"
  chmod 0755 "$top" "$top/lib" "$top"/lib/*
  chmod 0644 "$top/LICENSE" "$top/ThirdPartyNotices.txt" "$top/MIRROR.txt"
  pack "$w/pack" "$CUDA13_NAME" "$CUDA13_MTIME" "$w/$CUDA13_FILE" | tee "$w/uncompressed"
  pinned "$w/$CUDA13_FILE" "$CUDA13_SHA256" "$CUDA13_FILE"

  cp "$top/LICENSE" "$top/ThirdPartyNotices.txt" "$d/"
  mv "$w/$CUDA13_FILE" "$d/$CUDA13_FILE"
  cat > "$d/ATTRIBUTION.txt" <<TXT
Build input: ONNX Runtime ${CUDA13_VERSION}, Linux x64, CUDA 13 runtime libraries
Original download: ${CUDA13_URL}
Used by: Dockerfile.onnx-cuda-13, which ships the libraries next to the binary.
The archive holds libonnxruntime.so.${CUDA13_VERSION}, libonnxruntime_providers_cuda.so
and libonnxruntime_providers_shared.so from that download, unmodified, in its
lib/ directory. Microsoft's build is used because its CUDA provider has
kernels for RTX 50-series cards (sm_120). Voxtype publishes the same three
files, signed, in its own releases (0.7.4, 1.2.2 and 1.3.0-rc3 were compared).
The mirror script takes them from Microsoft's archive or from Voxtype release
${VOXTYPE_RELEASE} and requires the pinned sha256 of each either way.
LICENSE and ThirdPartyNotices.txt are from Microsoft's onnxruntime ${CUDA13_VERSION}
wheel on PyPI (sha256 ${WHEEL_SHA256}).
License: MIT. See LICENSE and ThirdPartyNotices.txt.
Distribution does not imply endorsement or transfer trademark rights.
TXT
  local contents
  contents=$(cd "$top" && python3 -I -c 'import hashlib, json, os, sys
print(json.dumps([{"path": p, "size": os.path.getsize(p), "sha256": hashlib.sha256(open(p, "rb").read()).hexdigest()}
                  for p in sorted(os.path.join(r, f)[2:] for r, _, fs in os.walk(".") for f in fs)]))')
  write_manifest "$d" "$CUDA13_NAME-runtime" "$CUDA13_FILE" "MIT" \
    "{\"origin\":{\"url\":\"${CUDA13_URL}\",\"version\":\"${CUDA13_VERSION}\"},\"uncompressed_sha256\":\"$(cut -d' ' -f3 "$w/uncompressed")\",\"contents\":${contents}}"
  rm -rf "$w"
  echo "STAGED $PREFIX/$CUDA13_DIR/$CUDA13_FILE $(stat -c %s "$d/$CUDA13_FILE") bytes sha256=$(sha "$d/$CUDA13_FILE")"
}

dir_of() { case "$1" in source) echo "$SOURCE_DIR" ;; cuda13) echo "$CUDA13_DIR" ;; esac; }

upload_one() {
  local dir; dir=$(dir_of "$1")
  rclone copy --immutable --checksum "$STAGE/$PREFIX/$dir" "$REMOTE/$PREFIX/$dir"
  echo "UPLOADED $PREFIX/$dir"
}

verify_one() {
  local dir; dir=$(dir_of "$1"); local d="$STAGE/$PREFIX/$dir" bad=0 path sha got
  while IFS=$'\t' read -r path sha; do
    got=$(curl -sSfL --retry 3 "$PUBLIC/$PREFIX/$dir/$path" | sha256sum | cut -d' ' -f1)
    if [[ "$got" == "$sha" ]]; then echo "OK   $PREFIX/$dir/$path"; else echo "BAD  $PREFIX/$dir/$path ($got != $sha)"; bad=1; fi
  done < <(python3 -I -c 'import json,sys; [print(f["path"],f["sha256"],sep="\t") for f in json.load(open(sys.argv[1]))["files"]]' "$d/manifest.json")
  got=$(curl -sSfL --retry 3 "$PUBLIC/$PREFIX/$dir/manifest.json" | sha256sum | cut -d' ' -f1)
  if [[ "$got" == "$(sha "$d/manifest.json")" ]]; then echo "OK   $PREFIX/$dir/manifest.json"; else echo "BAD  $PREFIX/$dir/manifest.json"; bad=1; fi
  return $bad
}

action="${1:-all}"; shift || true
case "$action" in stage|upload|verify|all) ;; *) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 2 ;; esac
[[ $# -gt 0 ]] || set -- source cuda13
tar --version | grep -q 'GNU tar' || die "GNU tar is required for deterministic archives"
mkdir -p "$STAGE/.work"
rc=0
for bundle in "$@"; do
  [[ -n "$(dir_of "$bundle")" ]] || die "unknown bundle '$bundle' (source, cuda13)"
  case "$action" in
    stage)  "stage_$bundle" ;;
    upload) upload_one "$bundle" ;;
    verify) verify_one "$bundle" || rc=1 ;;
    all)    "stage_$bundle"; upload_one "$bundle"; verify_one "$bundle" || rc=1 ;;
  esac
done
exit $rc
