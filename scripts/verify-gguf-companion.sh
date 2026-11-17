#!/bin/bash
# Verify a release binary and its GGUF worker companion.
#
# Every release binary compiles the GGUF client (features gguf, cohere-gguf,
# parakeet-gguf) and ships a voxtype-gguf-worker built for the same variant.
# This gate runs inside the Dockerfiles and again in build-linux.yml.
#
# Usage:
#   scripts/verify-gguf-companion.sh [options] <voxtype-binary> <gguf-worker>
#
# Options:
#   --vulkan          The worker must link the Vulkan loader (libvulkan.so.1).
#   --cpu-only        The worker must NOT link the Vulkan loader.
#   --haswell-floor   The worker must carry no AVX-512, GFNI, AVX-VNNI or
#                     SSE4a instructions (the x86_64-v3 floor).
#   --run             Execute the binary and require `info engines --json` to
#                     report the gguf engine compiled. Skip for cross-compiled
#                     AVX-512 binaries, which cannot run on the build host.
set -euo pipefail

vulkan=""
floor=false
run=false
while [[ $# -gt 2 ]]; do
    case "$1" in
        --vulkan) vulkan=required ;;
        --cpu-only) vulkan=forbidden ;;
        --haswell-floor) floor=true ;;
        --run) run=true ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
if [[ $# -ne 2 ]]; then
    echo "Usage: $0 [--vulkan|--cpu-only] [--haswell-floor] [--run] <voxtype-binary> <gguf-worker>" >&2
    exit 2
fi
binary="$1"
worker="$2"

# The binary is only executed with --run; the linkage and ISA gates read the
# worker alone, so packaging can check a worker whose binary is optional.
artifacts=("$worker")
[[ "$run" == true ]] && artifacts+=("$binary")
for artifact in "${artifacts[@]}"; do
    if [[ ! -f "$artifact" ]]; then
        echo "FAIL: missing $artifact" >&2
        exit 1
    fi
done

if [[ -n "$vulkan" ]]; then
    # Not grep -q: it exits at the first match, objdump then dies of SIGPIPE
    # on its next write, and pipefail reports a linked worker as unlinked
    # (31 of 200 runs on Fedora 44).
    if objdump -p "$worker" | grep -E 'NEEDED +libvulkan\.so\.1$' >/dev/null; then
        linked=required
    else
        linked=forbidden
    fi
    if [[ "$linked" != "$vulkan" ]]; then
        if [[ "$vulkan" == required ]]; then
            echo "FAIL: $worker does not link libvulkan.so.1; build it with -p voxtype-gguf-worker --features vulkan" >&2
        else
            echo "FAIL: $worker links libvulkan.so.1 but this variant ships a CPU worker" >&2
        fi
        exit 1
    fi
    echo "  GGUF worker Vulkan linkage: $vulkan (ok)"
fi

if [[ "$floor" == true ]]; then
    dump=$(objdump -d "$worker")
    count() { grep -cE "$1" <<<"$dump" || true; }
    zmm=$(count 'zmm')
    evex=$(count 'vpternlog|vpermt2|vpermi2|vpblendm|valign[dq]|vpcompress|vpexpand|\{1to')
    gfni=$(count '\b(vgf2p8|gf2p8)')
    sse4a=$(count '\b(insertq|extrq)\b')
    avxvnni=$(count '\bvpdpbus(d|ds)|vpdpwss(d|ds)')
    echo "  GGUF worker zmm=$zmm evex=$evex gfni=$gfni sse4a=$sse4a avx-vnni=$avxvnni"
    if (( zmm + evex + gfni + sse4a + avxvnni > 0 )); then
        echo "FAIL: $worker contains instructions above the x86_64-v3 floor" >&2
        exit 1
    fi
fi

if [[ "$run" == true ]]; then
    # Read the value, then compare: never gate on a pipeline's exit status.
    # Plain shell, no python: the CUDA images do not ship an interpreter.
    engines=$(RUST_LOG=off "$binary" info engines --json | tr -d ' \n')
    for engine in gguf parakeet cohere; do
        entry=$(grep -oE "\{[^}]*\"name\":\"$engine\"[^}]*\}" <<<"$engines" || true)
        echo "  info engines: ${entry:-<missing $engine>}"
        if [[ "$entry" != *'"compiled":true'* ]]; then
            echo "FAIL: $binary does not compile the $engine GGUF client" >&2
            exit 1
        fi
    done
fi

echo "✓ GGUF companion checks passed for $(basename "$worker")"
