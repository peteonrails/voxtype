#!/bin/bash
# Test the voxtype-bin AUR package: the variant selector in its PKGBUILD, the
# pacman hooks in voxtype-bin.install, and that .SRCINFO lists every variant.
#
# Usage: scripts/test-aur-bin.sh [--sums SHA256SUMS.txt] [--no-makepkg] <pkgbuild-dir>
#
#   <pkgbuild-dir>  a checkout of the voxtype-bin AUR repo (PKGBUILD,
#                   voxtype-bin.install, .SRCINFO)
#   --sums FILE     also check every checksum in the PKGBUILD's asset table
#                   against the release's SHA256SUMS.txt (verify its
#                   signature first; scripts/aur-fill-sums.sh does)
#   --no-makepkg    skip the checks that need makepkg and namcap
#
# Everything runs against fixture trees in a temporary directory: the
# selector reads /proc/cpuinfo, /usr/bin/voxtype and
# /usr/lib/voxtype/package-variants through the PKGBUILD's _vt_root, and the
# hooks write through the install script's _backend_root and _backend_state.
# Nothing on the host is read for a decision or written.
set -uo pipefail

usage() {
    sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

SUMS=""
RUN_MAKEPKG=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --sums) SUMS=${2:?--sums needs a file}; shift 2 ;;
        --no-makepkg) RUN_MAKEPKG=0; shift ;;
        -h|--help) usage ;;
        -*) echo "unknown option: $1" >&2; usage ;;
        *) break ;;
    esac
done
[[ $# -eq 1 ]] || usage
PKG_DIR=$(realpath "$1")
PKGBUILD="$PKG_DIR/PKGBUILD"
INSTALL_SCRIPT="$PKG_DIR/voxtype-bin.install"
[[ -f $PKGBUILD && -f $INSTALL_SCRIPT ]] || { echo "no PKGBUILD/voxtype-bin.install in $PKG_DIR" >&2; exit 2; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/test-aur-bin.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

PASSED=0
FAILED=0
pass() { PASSED=$((PASSED + 1)); echo "ok - $1"; }
fail() { FAILED=$((FAILED + 1)); echo "not ok - $1" >&2; }

ALL_X86="baseline avx2 avx512 vulkan onnx-avx2 onnx-avx512 onnx-cuda-12 onnx-cuda-13 onnx-migraphx"

# ---------------------------------------------------------------------------
# Selector matrix
# ---------------------------------------------------------------------------

# A fresh fixture root. Callers add files with the helpers below.
new_root() {
    local root="$WORK/root.$RANDOM$RANDOM"
    mkdir -p "$root/proc" "$root/usr/bin" "$root/usr/lib/voxtype"
    printf 'processor : 0\nflags : fpu sse4_2 %s\n' "${1:-}" >"$root/proc/cpuinfo"
    echo "$root"
}
link_active() { ln -s "$2" "$1/usr/bin/voxtype"; }
wrapper_active() { printf '#!/bin/sh\nexec %s "$@"\n' "$2" >"$1/usr/bin/voxtype"; }
record() {
    printf '# voxtype-bin 1.2.1-1: variants selected when the package was built\ninstall_method=aur-bin\nincluded=%s\nexplicit=%s\n' \
        "$2" "$3" >"$1/usr/lib/voxtype/package-variants"
}

# Source the PKGBUILD in a clean shell against fixture root $2 for CARCH $1,
# with VOXTYPE_VARIANTS=$3 (unset when $3 is "-"). Prints
#   selected=<list>
#   explicit=<list>
# then one "error: ..." line per inconsistency between the source and
# checksum arrays and the asset table, for both arches.
run_selector() {
    local arch=$1 root=$2 request=$3 extra=${4:-}
    local -a envs=(PATH="$PATH" HOME="$WORK" CARCH="$arch" _vt_root="$root")
    [[ $request != - ]] && envs+=(VOXTYPE_VARIANTS="$request")
    [[ -n $extra ]] && envs+=("$extra")
    env -i "${envs[@]}" bash -c '
        source "$1" || exit 1
        echo "selected=${_vt_selected[*]}"
        echo "explicit=${_vt_explicit[*]}"
        for a in "${arch[@]}"; do
            declare -n src="source_$a" sums="sha256sums_$a"
            if [[ $a == "$CARCH" ]]; then
                want=" ${_vt_selected[*]} "
            else
                want=" $(_vt_known "$a") "
            fi
            [[ ${#src[@]} -eq ${#sums[@]} ]] ||
                echo "error: $a: ${#src[@]} sources but ${#sums[@]} checksums"
            declare -A listed=()
            for i in "${!src[@]}"; do
                name=${src[$i]%%::*}
                if [[ $name == *.asc ]]; then
                    [[ ${sums[$i]} == SKIP ]] || echo "error: $a: $name is not SKIP"
                    [[ ${src[$((i - 1))]%%::*} == "${name%.asc}" ]] ||
                        echo "error: $a: $name does not follow its file"
                    continue
                fi
                asset=${name#voxtype-$pkgver-}
                listed[$asset]=1
                found=""
                declare -n table="_vt_assets_$a"
                for e in "${table[@]}"; do
                    IFS="|" read -r e_var e_asset e_sum <<<"$e"
                    [[ $e_asset == "$asset" ]] || continue
                    found=1
                    [[ ${sums[$i]} == "$e_sum" ]] || echo "error: $a: $asset checksum is not the table entry"
                    [[ $e_var == common || $want == *" $e_var "* ]] ||
                        echo "error: $a: $asset belongs to unselected variant $e_var"
                    url=${src[$i]#*::}
                    [[ $url == "$_github/voxtype-$_vt_ver-linux-$a-$asset" ]] ||
                        echo "error: $a: $asset downloads from $url"
                done
                [[ -n $found ]] || echo "error: $a: $asset is not in the asset table"
            done
            for e in "${table[@]}"; do
                IFS="|" read -r e_var e_asset e_sum <<<"$e"
                if [[ $e_var == common || $want == *" $e_var "* ]]; then
                    [[ -n ${listed[$e_asset]:-} ]] || echo "error: $a: $e_asset of $e_var is missing"
                fi
            done
            unset -n src sums table
            unset listed
        done
    ' _ "$PKGBUILD" 2>&1
}

# expect_select <description> <arch> <root> <VOXTYPE_VARIANTS or -> <selected> [explicit]
expect_select() {
    local desc=$1 arch=$2 root=$3 request=$4 selected=$5 explicit=${6:-}
    local out
    out=$(run_selector "$arch" "$root" "$request")
    local got_sel got_exp errors
    got_sel=$(sed -n 's/^selected=//p' <<<"$out")
    got_exp=$(sed -n 's/^explicit=//p' <<<"$out")
    errors=$(grep -v '^selected=\|^explicit=\|-> voxtype-bin variants:' <<<"$out" || true)
    if [[ $got_sel == "$selected" && $got_exp == "$explicit" && -z $errors ]]; then
        pass "select: $desc"
    else
        fail "select: $desc"
        echo "    want selected=[$selected] explicit=[$explicit]" >&2
        echo "    got  selected=[$got_sel] explicit=[$got_exp]" >&2
        [[ -n $errors ]] && sed 's/^/    /' <<<"$errors" >&2
    fi
}

echo "# selector"

r=$(new_root "avx2 avx512f")
expect_select "AVX-512 CPU, nothing installed" x86_64 "$r" - "baseline avx512 vulkan"
r=$(new_root "avx2")
expect_select "AVX2 CPU, nothing installed" x86_64 "$r" - "baseline avx2 vulkan"
r=$(new_root "")
expect_select "CPU without AVX2, nothing installed" x86_64 "$r" - "baseline vulkan"

r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-avx2
expect_select "active voxtype-avx2 adds nothing" x86_64 "$r" - "baseline avx2 vulkan"
r=$(new_root "avx2 avx512f"); link_active "$r" /usr/lib/voxtype/voxtype-avx2
expect_select "active voxtype-avx2 on an AVX-512 CPU keeps avx2" x86_64 "$r" - "baseline avx2 avx512 vulkan"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-vulkan
expect_select "active voxtype-vulkan" x86_64 "$r" - "baseline avx2 vulkan"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-avx2
expect_select "active voxtype-onnx-avx2" x86_64 "$r" - "baseline avx2 vulkan onnx-avx2"
r=$(new_root "avx2 avx512f"); wrapper_active "$r" /usr/lib/voxtype/migraphx/voxtype-onnx-migraphx
expect_select "active MIGraphX wrapper" x86_64 "$r" - "baseline avx512 vulkan onnx-migraphx"
r=$(new_root "avx2"); wrapper_active "$r" /usr/lib/voxtype/cuda-13/voxtype-onnx-cuda-13
expect_select "active CUDA 13 wrapper" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-13"
r=$(new_root "avx2"); wrapper_active "$r" /usr/lib/voxtype/cuda-12/voxtype-onnx-cuda-12
expect_select "active CUDA 12 wrapper" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-12"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-cuda
expect_select "unversioned voxtype-onnx-cuda, no libcudart: cuda-13" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-13"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-cuda
touch "$r/usr/lib/libcudart.so.12"
expect_select "unversioned voxtype-onnx-cuda with libcudart.so.12: cuda-12" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-12"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-cuda
mkdir -p "$r/opt/cuda/lib64"; touch "$r/opt/cuda/lib64/libcudart.so.12"
expect_select "unversioned voxtype-onnx-cuda with /opt/cuda libcudart.so.12" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-12"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-cuda
ln -s cuda-12/voxtype-onnx-cuda-12 "$r/usr/lib/voxtype/voxtype-onnx-cuda"
expect_select "unversioned link that resolves to cuda-12 follows the link" x86_64 "$r" - "baseline avx2 vulkan onnx-cuda-12"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-migraphx
expect_select "symlink to the top-level MIGraphX alias" x86_64 "$r" - "baseline avx2 vulkan onnx-migraphx"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-onnx-rocm
expect_select "pre-0.7 voxtype-onnx-rocm maps to MIGraphX" x86_64 "$r" - "baseline avx2 vulkan onnx-migraphx"
r=$(new_root "avx2"); link_active "$r" /usr/lib/voxtype/voxtype-parakeet-avx2
expect_select "0.6 voxtype-parakeet-avx2 maps to onnx-avx2" x86_64 "$r" - "baseline avx2 vulkan onnx-avx2"
r=$(new_root "avx2"); link_active "$r" /usr/local/lib/voxtype/voxtype-onnx-cuda-13
expect_select "a launcher outside /usr/lib/voxtype is ignored" x86_64 "$r" - "baseline avx2 vulkan"
r=$(new_root "avx2"); printf '\177ELF not a wrapper\n' >"$r/usr/bin/voxtype"
expect_select "a source-built /usr/bin/voxtype is ignored" x86_64 "$r" - "baseline avx2 vulkan"

r=$(new_root "avx2")
expect_select "VOXTYPE_VARIANTS=all" x86_64 "$r" all "$ALL_X86" "$ALL_X86"
r=$(new_root "avx2")
expect_select "VOXTYPE_VARIANTS=onnx-migraphx" x86_64 "$r" onnx-migraphx \
    "baseline avx2 vulkan onnx-migraphx" "onnx-migraphx"
r=$(new_root "avx2")
expect_select "comma list with a voxtype- prefix and spaces" x86_64 "$r" "voxtype-onnx-cuda-13, onnx-avx512" \
    "baseline avx2 vulkan onnx-avx512 onnx-cuda-13" "onnx-avx512 onnx-cuda-13"
r=$(new_root "avx2"); touch "$r/usr/lib/libcudart.so.12"
expect_select "VOXTYPE_VARIANTS=onnx-cuda picks the host's CUDA major" x86_64 "$r" onnx-cuda \
    "baseline avx2 vulkan onnx-cuda-12" "onnx-cuda-12"
r=$(new_root "avx2"); record "$r" "baseline avx2 vulkan onnx-cuda-13" "onnx-cuda-13"
expect_select "explicit picks carry over from package-variants" x86_64 "$r" - \
    "baseline avx2 vulkan onnx-cuda-13" "onnx-cuda-13"
r=$(new_root "avx2"); record "$r" "baseline avx2 vulkan onnx-cuda-13" "onnx-cuda-13"
expect_select "VOXTYPE_VARIANTS adds to recorded picks" x86_64 "$r" onnx-avx2 \
    "baseline avx2 vulkan onnx-avx2 onnx-cuda-13" "onnx-avx2 onnx-cuda-13"
r=$(new_root "avx2"); record "$r" "baseline avx2 vulkan onnx-cuda-13" "onnx-cuda-13"
expect_select "VOXTYPE_VARIANTS=default drops recorded picks" x86_64 "$r" default "baseline avx2 vulkan"
r=$(new_root "avx2"); record "$r" "baseline avx2 vulkan onnx-cuda-13" "onnx-cuda-13"
wrapper_active "$r" /usr/lib/voxtype/migraphx/voxtype-onnx-migraphx
expect_select "default still keeps the active backend" x86_64 "$r" default "baseline avx2 vulkan onnx-migraphx"
r=$(new_root "avx2"); record "$r" "baseline avx2 vulkan" "onnx-retired onnx-avx2"
expect_select "a recorded name no longer shipped is dropped quietly" x86_64 "$r" - \
    "baseline avx2 vulkan onnx-avx2" "onnx-avx2"
r=$(new_root "avx2"); record "$r" "$ALL_X86" "$ALL_X86"
expect_select "a recorded all-variants build stays all" x86_64 "$r" - "$ALL_X86" "$ALL_X86"

r=$(new_root "")
expect_select "aarch64, nothing installed" aarch64 "$r" - "cpu"
r=$(new_root ""); link_active "$r" /usr/lib/voxtype/voxtype-cpu
expect_select "aarch64, active voxtype-cpu" aarch64 "$r" - "cpu"
r=$(new_root ""); link_active "$r" /usr/lib/voxtype/voxtype-onnx
expect_select "aarch64, active voxtype-onnx" aarch64 "$r" - "cpu onnx"
r=$(new_root "")
expect_select "aarch64, VOXTYPE_VARIANTS=all" aarch64 "$r" all "cpu onnx" "cpu onnx"

r=$(new_root "avx2")
out=$(run_selector x86_64 "$r" "onnx-bogus" 2>&1)
if [[ $out == *"unknown variant 'onnx-bogus'"* && $out != *selected=* ]]; then
    pass "select: an unknown VOXTYPE_VARIANTS name stops the build"
else
    fail "select: an unknown VOXTYPE_VARIANTS name stops the build"; sed 's/^/    /' <<<"$out" >&2
fi
r=$(new_root "")
out=$(run_selector aarch64 "$r" "vulkan" 2>&1)
if [[ $out == *"unknown variant 'vulkan' for aarch64"* ]]; then
    pass "select: an x86_64-only variant is rejected on aarch64"
else
    fail "select: an x86_64-only variant is rejected on aarch64"; sed 's/^/    /' <<<"$out" >&2
fi
r=$(new_root "avx2")
out=$(run_selector x86_64 "$r" - PRINTSRCINFO=1)
if [[ $(sed -n 's/^selected=//p' <<<"$out") == "$ALL_X86" ]] && ! grep -q '^error' <<<"$out"; then
    pass "select: makepkg --printsrcinfo always lists every variant"
else
    fail "select: makepkg --printsrcinfo always lists every variant"; sed 's/^/    /' <<<"$out" >&2
fi

if [[ -n $SUMS ]]; then
    errors=$(env -i PATH="$PATH" CARCH=x86_64 _vt_root="$WORK/none" VOXTYPE_VARIANTS=all bash -c '
        source "$1" || exit 1
        for a in "${arch[@]}"; do
          declare -n table="_vt_assets_$a"
          for e in "${table[@]}"; do
            IFS="|" read -r v asset sum <<<"$e"
            file="voxtype-$_vt_ver-linux-$a-$asset"
            want=$(awk -v f="$file" "\$2 == f || \$2 == \"*\" f { print \$1 }" "$2")
            if [[ -z $want ]]; then
                echo "$file is not in SHA256SUMS.txt"
            elif [[ $want != "$sum" ]]; then
                echo "$file: table has $sum, SHA256SUMS.txt has $want"
            fi
          done
          unset -n table
        done
    ' _ "$PKGBUILD" "$SUMS" 2>&1 | grep -v -- "-> voxtype-bin variants:")
    if [[ -z $errors ]]; then
        pass "sums: every asset table checksum matches SHA256SUMS.txt"
    else
        fail "sums: every asset table checksum matches SHA256SUMS.txt"; sed 's/^/    /' <<<"$errors" >&2
    fi
fi

# ---------------------------------------------------------------------------
# Install-script matrix
# ---------------------------------------------------------------------------
echo "# install script"

# Run a snippet with voxtype-bin.install sourced and its system seams pointed
# into a fixture: $T/lib is /usr/lib/voxtype, $T/active records what would
# become /usr/bin/voxtype. FLAGS, CUDA and MACHINE set the CPU flags, the
# detected CUDA major and uname -m.
run_hook() {
    local T=$1 snippet=$2
    T="$T" FLAGS="${FLAGS:-}" CUDA="${CUDA:-}" MACHINE="${MACHINE:-x86_64}" bash -c '
        source "$1"
        _backend_state="$T/state"
        _backend_root="$T/lib"
        _omarchy_dir="$T/omarchy"
        _cpu_has() { [[ " $FLAGS " == *" $1 "* ]]; }
        _install_active_binary() { printf "%s\n" "$1" >"$T/active"; }
        _detect_cuda_major() { [[ -n $CUDA ]] && echo "$CUDA"; return 0; }
        _detect_gpu() { :; }
        uname() { echo "$MACHINE"; }
        eval "$2"
    ' _ "$INSTALL_SCRIPT" "$snippet"
}

# A fixture /usr/lib/voxtype holding the given variants.
new_lib() {
    local T="$WORK/hook.$RANDOM$RANDOM" v
    mkdir -p "$T/lib"
    for v in "$@"; do
        case "$v" in
            onnx-cuda-12|onnx-cuda-13|onnx-migraphx)
                mkdir -p "$T/lib/${v#onnx-}"
                touch "$T/lib/${v#onnx-}/voxtype-$v"
                ln -s "${v#onnx-}/voxtype-$v" "$T/lib/voxtype-$v"
                ;;
            *) touch "$T/lib/voxtype-$v" ;;
        esac
    done
    echo "$T"
}

# expect_fresh <description> <backend> <variants...>
expect_fresh() {
    local desc=$1 backend=$2 T out
    shift 2
    T=$(new_lib "$@")
    out=$(run_hook "$T" '_set_default_backend')
    if [[ $out == "$backend" && $(<"$T/active") == "$T/lib/voxtype-$backend" ]]; then
        pass "fresh: $desc"
    else
        fail "fresh: $desc: got $out -> $(cat "$T/active" 2>/dev/null)"
    fi
}

FLAGS="sse4_2" expect_fresh "no AVX2 gets baseline" baseline baseline avx2 vulkan
FLAGS="sse4_2 avx2" expect_fresh "AVX2 gets avx2" avx2 baseline avx2 vulkan
FLAGS="sse4_2 avx2 avx512f" expect_fresh "AVX-512 gets avx512" avx512 baseline avx512 vulkan
FLAGS="sse4_2 avx2 avx512f" expect_fresh "AVX-512 CPU, build without avx512, gets avx2" avx2 baseline avx2 vulkan
FLAGS="sse4_2 avx2" expect_fresh "AVX2 CPU, build without avx2, gets baseline" baseline baseline avx512 vulkan
MACHINE=aarch64 expect_fresh "aarch64 gets cpu" cpu cpu

# expect_upgrade <description> <saved path, @ = fixture lib> <backend> <active, @ = lib>
#                <warning: lost name or -> <variants...>
expect_upgrade() {
    local desc=$1 saved=$2 backend=$3 active=$4 lost=$5 T out err
    shift 5
    T=$(new_lib "$@")
    printf '%s\n' "${saved//@/$T/lib}" >"$T/state"
    out=$(run_hook "$T" '_preserve_or_set_backend' 2>"$T/stderr")
    err=$(<"$T/stderr")
    local ok=1
    [[ $out == "$backend" && $(<"$T/active") == "${active//@/$T/lib}" ]] || ok=0
    [[ ! -e $T/state ]] || ok=0
    if [[ $lost == - ]]; then
        [[ -z $err ]] || ok=0
    else
        [[ $err == *"Your previous backend ($lost) is not in this"* &&
           $err == *"Dictation now uses voxtype-$backend."* &&
           $err == *"VOXTYPE_VARIANTS=${lost#voxtype-} yay -S --rebuild voxtype-bin"* &&
           $err == *"VOXTYPE_VARIANTS=${lost#voxtype-} makepkg -si"* ]] || ok=0
    fi
    if (( ok )); then
        pass "upgrade: $desc"
    else
        fail "upgrade: $desc: got $out -> $(cat "$T/active" 2>/dev/null)"
        [[ -n $err ]] && sed 's/^/    /' <<<"$err" >&2
    fi
}

FLAGS="sse4_2" expect_upgrade "no AVX2 moves off a saved avx2" @/voxtype-avx2 baseline @/voxtype-baseline - \
    baseline avx2 vulkan
FLAGS="sse4_2 avx2" expect_upgrade "no AVX-512 moves off a saved avx512" @/voxtype-avx512 avx2 @/voxtype-avx2 - \
    baseline avx2 avx512 vulkan
FLAGS="sse4_2 avx2" expect_upgrade "keeps a CPU build the host can run" @/voxtype-avx2 avx2 @/voxtype-avx2 - \
    baseline avx2 vulkan
FLAGS="sse4_2" expect_upgrade "keeps the GPU build the user chose" @/voxtype-vulkan vulkan @/voxtype-vulkan - \
    baseline vulkan
FLAGS="sse4_2 avx2 avx512f" expect_upgrade "keeps MIGraphX when the build has it" \
    @/migraphx/voxtype-onnx-migraphx onnx-migraphx @/migraphx/voxtype-onnx-migraphx - \
    baseline avx512 vulkan onnx-migraphx
FLAGS="sse4_2 avx2 avx512f" expect_upgrade "keeps MIGraphX saved by its top-level alias" \
    @/voxtype-onnx-migraphx onnx-migraphx @/migraphx/voxtype-onnx-migraphx - \
    baseline avx512 vulkan onnx-migraphx
FLAGS="sse4_2 avx2 avx512f" expect_upgrade "MIGraphX not in the build: default and a warning" \
    @/migraphx/voxtype-onnx-migraphx avx512 @/voxtype-avx512 voxtype-onnx-migraphx \
    baseline avx512 vulkan
FLAGS="sse4_2 avx2" expect_upgrade "CUDA 13 not in the build: default and a warning" \
    @/cuda-13/voxtype-onnx-cuda-13 avx2 @/voxtype-avx2 voxtype-onnx-cuda-13 \
    baseline avx2 vulkan
FLAGS="sse4_2 avx2" expect_upgrade "onnx-avx2 not in the build: default and a warning" \
    @/voxtype-onnx-avx2 avx2 @/voxtype-avx2 voxtype-onnx-avx2 \
    baseline avx2 vulkan
FLAGS="sse4_2 avx2" CUDA=12 expect_upgrade "unversioned onnx-cuda maps to the host's CUDA 12" \
    @/voxtype-onnx-cuda onnx-cuda-12 @/cuda-12/voxtype-onnx-cuda-12 - \
    baseline avx2 vulkan onnx-cuda-12
FLAGS="sse4_2 avx2" expect_upgrade "unversioned onnx-cuda without CUDA maps to cuda-13" \
    @/voxtype-onnx-cuda onnx-cuda-13 @/cuda-13/voxtype-onnx-cuda-13 - \
    baseline avx2 vulkan onnx-cuda-13
FLAGS="sse4_2 avx2" expect_upgrade "pre-0.7 voxtype-onnx-rocm maps to MIGraphX" \
    @/voxtype-onnx-rocm onnx-migraphx @/migraphx/voxtype-onnx-migraphx - \
    baseline avx2 vulkan onnx-migraphx
FLAGS="sse4_2 avx2" expect_upgrade "a saved backend that is gone falls back with a warning" \
    @/voxtype-missing avx2 @/voxtype-avx2 voxtype-missing baseline avx2 vulkan

# What another local user could write into the state file: only a binary
# under the backend directory may become /usr/bin/voxtype, and none of these
# earn a warning.
T=$(new_lib baseline avx2 vulkan); touch "$T/payload"
FLAGS="sse4_2 avx2" expect_upgrade "a saved path outside the backend directory is ignored" \
    "$T/payload" avx2 @/voxtype-avx2 - baseline avx2 vulkan
FLAGS="sse4_2 avx2" expect_upgrade "a saved path that climbs out of the backend directory is ignored" \
    @/../payload avx2 @/voxtype-avx2 - baseline avx2 vulkan
T=$(new_lib baseline avx2 vulkan); touch "$T/payload"; ln -s "$T/payload" "$T/lib/voxtype-escape"
printf '%s\n' "$T/lib/voxtype-escape" >"$T/state"
out=$(FLAGS="sse4_2 avx2" run_hook "$T" '_preserve_or_set_backend' 2>"$T/stderr")
if [[ $out == avx2 && $(<"$T/active") == "$T/lib/voxtype-avx2" && ! -s $T/stderr ]]; then
    pass "upgrade: a saved symlink that leaves the backend directory is ignored"
else
    fail "upgrade: a saved symlink that leaves the backend directory is ignored: got $out"
fi
T=$(new_lib baseline avx2 vulkan)
out=$(FLAGS="sse4_2 avx2" run_hook "$T" '_preserve_or_set_backend' 2>"$T/stderr")
if [[ $out == avx2 && ! -s $T/stderr ]]; then
    pass "upgrade: no saved state sets the default quietly"
else
    fail "upgrade: no saved state sets the default quietly: got $out"
fi

# pre_upgrade must not hand a file an interrupted upgrade left to the next one.
T=$(new_lib baseline avx2 vulkan)
printf '%s\n' "$T/lib/voxtype-vulkan" >"$T/state"
run_hook "$T" '_resolve_active_binary() { :; }; pre_upgrade'
if [[ ! -e $T/state ]]; then
    pass "pre_upgrade drops a stale state file when it finds no launcher"
else
    fail "pre_upgrade drops a stale state file when it finds no launcher"
fi
run_hook "$T" '_resolve_active_binary() { echo "$T/lib/voxtype-avx2"; }; pre_upgrade'
if [[ $(cat "$T/state" 2>/dev/null) == "$T/lib/voxtype-avx2" ]]; then
    pass "pre_upgrade saves the launcher's backend"
else
    fail "pre_upgrade saves the launcher's backend"
fi
if grep -q '/tmp/' "$INSTALL_SCRIPT"; then
    fail "the install script hands state through /tmp"
else
    pass "the install script hands state through /run, not /tmp"
fi

# The unversioned voxtype-onnx-cuda link.
T=$(new_lib baseline avx2 vulkan); ln -s cuda-13/voxtype-onnx-cuda-13 "$T/lib/voxtype-onnx-cuda"
out=$(run_hook "$T" '_set_onnx_cuda_symlink')
if [[ -z $out && ! -L $T/lib/voxtype-onnx-cuda ]]; then
    pass "cuda link: a dangling link from an all-variants install is removed"
else
    fail "cuda link: a dangling link from an all-variants install is removed"
fi
T=$(new_lib baseline avx2 vulkan onnx-cuda-13)
out=$(run_hook "$T" '_set_onnx_cuda_symlink')
if [[ $out == 13 && $(readlink "$T/lib/voxtype-onnx-cuda") == cuda-13/voxtype-onnx-cuda-13 ]]; then
    pass "cuda link: points at cuda-13 when the build has it and no CUDA is installed"
else
    fail "cuda link: points at cuda-13 when the build has it and no CUDA is installed: got $out"
fi
T=$(new_lib baseline avx2 vulkan onnx-cuda-12 onnx-cuda-13)
out=$(CUDA=12 run_hook "$T" '_set_onnx_cuda_symlink')
if [[ $out == 12 && $(readlink "$T/lib/voxtype-onnx-cuda") == cuda-12/voxtype-onnx-cuda-12 ]]; then
    pass "cuda link: follows the host's CUDA 12 runtime"
else
    fail "cuda link: follows the host's CUDA 12 runtime: got $out"
fi
T=$(new_lib baseline avx2 vulkan onnx-cuda-13)
out=$(CUDA=12 run_hook "$T" '_set_onnx_cuda_symlink')
if [[ -z $out && ! -e $T/lib/voxtype-onnx-cuda ]]; then
    pass "cuda link: no link to a CUDA major the host does not run"
else
    fail "cuda link: no link to a CUDA major the host does not run: got $out"
fi

# Messages.
T=$(new_lib baseline avx2 vulkan)
printf 'included=baseline avx2 vulkan\nexplicit=\n' >"$T/lib/package-variants"
printf '%s\n' "$T/lib/migraphx/voxtype-onnx-migraphx" >"$T/state"
out=$(FLAGS="sse4_2 avx2" run_hook "$T" 'post_upgrade 1.2.1-1' 2>&1)
if [[ $out == *"Voxtype upgraded to 1.2.1"* &&
      $out == *"releases/tag/v1.2.1" &&
      $out == *"Backend: avx2 (using voxtype-avx2)"* &&
      $out == *"Your previous backend (voxtype-onnx-migraphx)"* &&
      $out == *"Variants in this build: baseline avx2 vulkan"* &&
      $out != *"Omarchy:"* ]]; then
    pass "post_upgrade prints the version, lost backend, variants and no Omarchy line off Omarchy"
else
    fail "post_upgrade output"; sed 's/^/    /' <<<"$out" >&2
fi
mkdir -p "$T/omarchy"
out=$(FLAGS="sse4_2 avx2" run_hook "$T" 'post_upgrade 1.2.1.rc1-1' 2>&1)
if [[ $out == *"releases/tag/v1.2.1-rc1"* && $out == *"voxtype setup omarchy"* ]]; then
    pass "post_upgrade links an rc to its tag and points Omarchy users at voxtype setup omarchy"
else
    fail "post_upgrade rc tag and Omarchy line"; sed 's/^/    /' <<<"$out" >&2
fi
out=$(FLAGS="sse4_2 avx2" run_hook "$T" 'post_install' 2>&1)
if [[ $out == *"CPU backend: avx2"* && $out == *"voxtype setup omarchy"* &&
      $out == *"Variants in this build: baseline avx2 vulkan"* ]]; then
    pass "post_install prints the backend, variants and the Omarchy line"
else
    fail "post_install output"; sed 's/^/    /' <<<"$out" >&2
fi

# ---------------------------------------------------------------------------
# Static checks
# ---------------------------------------------------------------------------
echo "# static"

for f in "$PKGBUILD" "$INSTALL_SCRIPT" "$0"; do
    if bash -n "$f"; then pass "bash -n ${f##*/}"; else fail "bash -n ${f##*/}"; fi
done
if grep -n $'—' "$PKGBUILD" "$INSTALL_SCRIPT"; then
    fail "no em dashes in the PKGBUILD or install script"
else
    pass "no em dashes in the PKGBUILD or install script"
fi

if (( RUN_MAKEPKG )); then
    if command -v makepkg >/dev/null; then
        if diff -u "$PKG_DIR/.SRCINFO" <(cd "$PKG_DIR" && VOXTYPE_VARIANTS=all makepkg --printsrcinfo 2>/dev/null) >"$WORK/srcinfo.diff"; then
            pass ".SRCINFO matches VOXTYPE_VARIANTS=all makepkg --printsrcinfo"
        else
            fail ".SRCINFO matches VOXTYPE_VARIANTS=all makepkg --printsrcinfo"; head -40 "$WORK/srcinfo.diff" >&2
        fi
        # A maintainer who forgets VOXTYPE_VARIANTS=all must still get the full list.
        if diff -q "$PKG_DIR/.SRCINFO" <(cd "$PKG_DIR" && env -u VOXTYPE_VARIANTS makepkg --printsrcinfo 2>/dev/null) >/dev/null; then
            pass ".SRCINFO is the same without VOXTYPE_VARIANTS"
        else
            fail ".SRCINFO is the same without VOXTYPE_VARIANTS"
        fi
    else
        fail "makepkg not found (use --no-makepkg to skip)"
    fi
    if command -v namcap >/dev/null; then
        out=$(cd "$PKG_DIR" && namcap PKGBUILD 2>&1)
        if [[ -z $out ]]; then
            pass "namcap PKGBUILD"
        else
            fail "namcap PKGBUILD"; sed 's/^/    /' <<<"$out" >&2
        fi
    else
        fail "namcap not found (use --no-makepkg to skip)"
    fi
fi

echo
echo "$PASSED passed, $FAILED failed"
(( FAILED == 0 ))
