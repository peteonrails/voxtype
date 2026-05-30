#!/usr/bin/env bash
# mirror-models-to-r2.sh — populate the voxtype-models R2 bucket from
# upstream HuggingFace repos, generating manifest.json with sha256s along
# the way.
#
# Layout written to R2:
#   r2:voxtype-models/{engine_prefix}/{model_name}/manifest.json
#   r2:voxtype-models/{engine_prefix}/{model_name}/{relative_file_path}
#
# The local file path on R2 matches what `download_artifact` writes to disk
# under `{models_dir}/{model_name}/`, so the runtime sees a byte-identical
# tree.
#
# Prerequisites:
#   - rclone configured with a remote literally named `r2` pointing at the
#     Cloudflare R2 bucket holding the `voxtype-models` namespace. The
#     remote's bucket name must be `voxtype-models`.
#       rclone config show r2          # to verify
#   - curl available on PATH.
#   - sha256sum (coreutils).
#   - cargo + a checkout of this repo (the script runs
#     `cargo run --bin voxtype-mirror-registry` to discover models).
#
# Usage:
#   ./scripts/mirror-models-to-r2.sh <model-name>      # one model
#   ./scripts/mirror-models-to-r2.sh --all             # every model
#   ./scripts/mirror-models-to-r2.sh --all --dry-run   # skip rclone upload
#
# For the very first population (no models on R2 yet) Pete should run:
#   ./scripts/mirror-models-to-r2.sh --all
#
# This is idempotent: re-running re-downloads from HF, recomputes sha256s,
# and (unless --dry-run) overwrites R2 with the fresh bytes. Identical
# uploads are a no-op for rclone's size/mtime checks but the manifest will
# always be re-uploaded.

set -euo pipefail

DRY_RUN=0
TARGET=""

# Models that hit a per-file 404 (or transient fetch failure) during mirroring
# and got skipped. Reported as a non-fatal summary at the end so the operator
# knows which registry entries need follow-up cleanup. See the warn branch in
# `mirror_one` and the registry-cleanup TODO.
SKIPPED_MODELS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --all)
            TARGET="--all"
            shift
            ;;
        --help|-h)
            sed -n 's/^# \{0,1\}//p' "$0" | sed -n '1,40p'
            exit 0
            ;;
        --*)
            echo "error: unknown flag: $1" >&2
            exit 2
            ;;
        *)
            if [[ -n "$TARGET" && "$TARGET" != "--all" ]]; then
                echo "error: multiple positional args; pass one model name or --all" >&2
                exit 2
            fi
            TARGET="$1"
            shift
            ;;
    esac
done

if [[ -z "$TARGET" ]]; then
    echo "error: pass a model name or --all (try --help)" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
R2_REMOTE="${VOXTYPE_R2_REMOTE:-r2:voxtype-models}"

# Sanity-check tooling up front rather than failing mid-mirror.
for cmd in curl sha256sum cargo jq; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "error: required command not found on PATH: $cmd" >&2
        exit 1
    fi
done

if [[ "$DRY_RUN" -eq 0 ]] && ! command -v rclone >/dev/null 2>&1; then
    echo "error: rclone not found on PATH (or pass --dry-run)" >&2
    exit 1
fi

cd "$REPO_DIR"

echo "[mirror] building voxtype-mirror-registry..." >&2
cargo build --quiet --bin voxtype-mirror-registry

REGISTRY_JSON="$(./target/debug/voxtype-mirror-registry)"

mirror_one() {
    local engine="$1"
    local name="$2"
    local upstream="$3"
    local files_json="$4"

    local workdir
    workdir="$(mktemp -d -t voxtype-mirror-XXXXXX)"
    trap 'rm -rf "$workdir"' RETURN

    echo "" >&2
    echo "[mirror] $engine/$name <- huggingface.co/$upstream" >&2

    # Build manifest while downloading.
    local manifest_files="[]"
    local total_size=0
    local file_count
    file_count="$(echo "$files_json" | jq 'length')"

    for i in $(seq 0 $((file_count - 1))); do
        local upstream_path local_path url dest size sha
        upstream_path="$(echo "$files_json" | jq -r ".[$i].upstream_path")"
        local_path="$(echo "$files_json" | jq -r ".[$i].local_path")"
        url="https://huggingface.co/$upstream/resolve/main/$upstream_path"
        dest="$workdir/$local_path"

        mkdir -p "$(dirname "$dest")"
        echo "  fetching $upstream_path -> $local_path" >&2
        # `--fail` so a 404 doesn't quietly produce an HTML error page.
        # Use `if !` to opt out of `set -e` for this one call — a single
        # missing file should skip the model and continue with the next,
        # not abort the whole run. The registry has stale entries (e.g.
        # Moonshine language variants that expect decoder_model_merged.onnx
        # while the upstream HF repo only ships decoder_model.onnx); their
        # fix is a separate registry-cleanup PR, not blocking the mirror.
        if ! curl -fsSL --retry 3 -o "$dest" "$url"; then
            echo "  WARN: HF 404 (or transient failure) on $upstream_path" >&2
            echo "  WARN: skipping model $engine/$name; will not be mirrored" >&2
            SKIPPED_MODELS+=("$engine/$name (missing: $upstream_path)")
            return 0
        fi

        size="$(stat -c %s "$dest")"
        sha="$(sha256sum "$dest" | awk '{print $1}')"
        total_size=$((total_size + size))

        manifest_files="$(jq --arg p "$local_path" --argjson s "$size" --arg h "$sha" \
            '. + [{"path": $p, "size": $s, "sha256": $h}]' <<< "$manifest_files")"
    done

    local manifest_json
    manifest_json="$(jq -n \
        --argjson v 1 \
        --arg m "$name" \
        --arg e "$engine" \
        --argjson f "$manifest_files" \
        '{version: $v, model: $m, engine: $e, files: $f}')"
    echo "$manifest_json" > "$workdir/manifest.json"

    local manifest_sha
    manifest_sha="$(sha256sum "$workdir/manifest.json" | awk '{print $1}')"
    local dest_path="$R2_REMOTE/$engine/$name/"

    echo "[mirror] $name: $file_count files, $total_size bytes, manifest sha256 $manifest_sha" >&2
    echo "[mirror] destination: $dest_path" >&2

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[mirror] --dry-run set, skipping rclone copy" >&2
    else
        rclone copy --progress "$workdir/" "$dest_path"
    fi
}

if [[ "$TARGET" == "--all" ]]; then
    # Process-substitution rather than `... | while` so SKIPPED_MODELS+=
    # inside mirror_one persists past the loop body (a pipe would put the
    # whole while-loop in a subshell and lose the array on exit).
    while read -r entry; do
        engine="$(echo "$entry" | jq -r '.engine_prefix')"
        name="$(echo "$entry" | jq -r '.name')"
        upstream="$(echo "$entry" | jq -r '.upstream_repo')"
        files_json="$(echo "$entry" | jq -c '.files')"
        mirror_one "$engine" "$name" "$upstream" "$files_json"
    done < <(echo "$REGISTRY_JSON" | jq -c '.[]')
else
    entry="$(echo "$REGISTRY_JSON" | jq -c --arg n "$TARGET" '.[] | select(.name == $n)')"
    if [[ -z "$entry" ]]; then
        echo "error: no model named '$TARGET' in the registry" >&2
        echo "       try --all to mirror everything, or pick one of:" >&2
        echo "$REGISTRY_JSON" | jq -r '.[].name' | sed 's/^/         - /' >&2
        exit 1
    fi
    engine="$(echo "$entry" | jq -r '.engine_prefix')"
    name="$(echo "$entry" | jq -r '.name')"
    upstream="$(echo "$entry" | jq -r '.upstream_repo')"
    files_json="$(echo "$entry" | jq -c '.files')"
    mirror_one "$engine" "$name" "$upstream" "$files_json"
fi

echo "" >&2
echo "[mirror] done." >&2

if [[ ${#SKIPPED_MODELS[@]} -gt 0 ]]; then
    echo "" >&2
    echo "[mirror] WARNING: ${#SKIPPED_MODELS[@]} model(s) skipped due to upstream 404 or fetch failure:" >&2
    for entry in "${SKIPPED_MODELS[@]}"; do
        echo "  - $entry" >&2
    done
    echo "" >&2
    echo "[mirror] These registry entries point at upstream HuggingFace files that do not exist (or" >&2
    echo "         changed). Audit them in src/setup/model.rs and either fix the file list or" >&2
    echo "         remove the entry. The remaining models were mirrored successfully." >&2
fi
