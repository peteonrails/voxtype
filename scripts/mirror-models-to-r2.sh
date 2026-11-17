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
#   ./scripts/mirror-models-to-r2.sh <model-name>       # one model
#   ./scripts/mirror-models-to-r2.sh --engine <prefix>  # every model of one engine
#   ./scripts/mirror-models-to-r2.sh --all              # every model
#   ./scripts/mirror-models-to-r2.sh --all --dry-run    # print the plan only
#   ./scripts/mirror-models-to-r2.sh --gguf-audit       # check every catalog GGUF on R2
#   ./scripts/mirror-models-to-r2.sh --gguf-audit --full  # ...and re-hash every byte
#
# --gguf-audit checks the curated single-file GGUF catalog (Cohere, Parakeet
# and the shared `gguf` engine) rather than the ONNX registry. For each model
# it reports MISSING when the public R2 URL does not answer 200, SIZE when the
# served size differs from the pinned size, UPSTREAM when the catalog still
# downloads it from a pinned upstream instead of R2, and with --full HASH when
# the served bytes do not match the pinned sha256. Exit status is 1 when any
# model fails. It reads public URLs only and never writes to R2.
#
# GGUF models are mirrored by hand, one approved model at a time, as a
# directory holding the pinned GGUF plus LICENSE, NOTICE, README.md (the
# conversion card), UPSTREAM-README.md, ATTRIBUTION.txt and manifest.json.
# Upload the GGUF and attribution files first and manifest.json last, to a
# directory that is empty, then re-run --gguf-audit --full. Never mirror a
# model whose weights license is on hold.
#
# --dry-run downloads nothing and uploads nothing. It prints each file it
# would fetch and where it would upload it, and checks with a HEAD request
# that every upstream file still exists, so stale registry entries show up
# in the end-of-run summary.
#
# A real run stages one model at a time under $TMPDIR (default /tmp). On
# systems where /tmp is tmpfs that is RAM, and the largest models are
# several GB, so point TMPDIR at a disk path first:
#   TMPDIR=~/.cache/voxtype-mirror ./scripts/mirror-models-to-r2.sh --all
#
# Prefer --engine over --all when adding a new engine's models: --all
# re-downloads every registry entry from HF and overwrites every manifest
# already on R2, so a changed upstream repo would silently replace bytes
# users' manifests already pin. --engine touches only that engine's tree.
#
# For the very first population (no models on R2 yet) Pete should run:
#   ./scripts/mirror-models-to-r2.sh --all
#
# This is idempotent: re-running re-downloads from HF, recomputes sha256s,
# and overwrites R2 with the fresh bytes. Identical
# uploads are a no-op for rclone's size/mtime checks but the manifest will
# always be re-uploaded.

set -euo pipefail

DRY_RUN=0
TARGET=""
ENGINE_FILTER=""
GGUF_AUDIT=0
GGUF_FULL=0

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
        --gguf-audit)
            GGUF_AUDIT=1
            shift
            ;;
        --full)
            GGUF_FULL=1
            shift
            ;;
        --help|-h)
            # The header comment, up to the first line that isn't one.
            sed -n '2,/^[^#]/{/^#/s/^# \{0,1\}//p}' "$0"
            exit 0
            ;;
        --engine)
            if [[ $# -lt 2 ]]; then
                echo "error: --engine needs a prefix (e.g. --engine openvino)" >&2
                exit 2
            fi
            ENGINE_FILTER="$2"
            shift 2
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

if [[ "$GGUF_FULL" -eq 1 && "$GGUF_AUDIT" -eq 0 ]]; then
    echo "error: --full only applies to --gguf-audit" >&2
    exit 2
fi

if [[ "$GGUF_AUDIT" -eq 1 && ( -n "$TARGET" || -n "$ENGINE_FILTER" || "$DRY_RUN" -eq 1 ) ]]; then
    echo "error: --gguf-audit cannot be combined with a model name, --all, --engine or --dry-run" >&2
    exit 2
fi

if [[ -n "$ENGINE_FILTER" && -n "$TARGET" ]]; then
    echo "error: --engine cannot be combined with a model name or --all" >&2
    exit 2
fi

if [[ -z "$TARGET" && -z "$ENGINE_FILTER" && "$GGUF_AUDIT" -eq 0 ]]; then
    echo "error: pass a model name, --engine <prefix>, or --all (try --help)" >&2
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

if [[ "$DRY_RUN" -eq 0 && "$GGUF_AUDIT" -eq 0 ]] && ! command -v rclone >/dev/null 2>&1; then
    echo "error: rclone not found on PATH (or pass --dry-run)" >&2
    exit 1
fi

cd "$REPO_DIR"

echo "[mirror] building voxtype-mirror-registry..." >&2
cargo build --quiet --bin voxtype-mirror-registry

TARGET_DIR="$(cargo metadata --no-deps --format-version 1 | jq -r .target_directory)"

if [[ "$GGUF_AUDIT" -eq 1 ]]; then
    GGUF_JSON="$("$TARGET_DIR/debug/voxtype-mirror-registry" --gguf)"
    failures=0
    count="$(echo "$GGUF_JSON" | jq 'length')"
    for ((i = 0; i < count; i++)); do
        name="$(echo "$GGUF_JSON" | jq -r ".[$i].name")"
        engine="$(echo "$GGUF_JSON" | jq -r ".[$i].engine")"
        r2_dir="$(echo "$GGUF_JSON" | jq -r ".[$i].r2_dir")"
        size="$(echo "$GGUF_JSON" | jq -r ".[$i].size")"
        sha="$(echo "$GGUF_JSON" | jq -r ".[$i].sha256")"
        mirrored="$(echo "$GGUF_JSON" | jq -r ".[$i].mirrored")"
        problems=()
        [[ "$mirrored" == "true" ]] || problems+=("UPSTREAM")
        if [[ -n "$r2_dir" ]]; then
            url="https://models.voxtype.io/$engine/$r2_dir/$name"
            head_out="$(curl -sS -o /dev/null -I -w '%{http_code} %header{content-length}' "$url" || true)"
            code="${head_out%% *}"
            served="${head_out#* }"
            if [[ "$code" != "200" ]]; then
                problems+=("MISSING")
            elif [[ "$served" != "$size" ]]; then
                problems+=("SIZE")
            elif [[ "$GGUF_FULL" -eq 1 ]]; then
                got="$(curl -sS --fail "$url" | sha256sum | cut -d' ' -f1)"
                [[ "$got" == "$sha" ]] || problems+=("HASH")
            fi
        else
            problems+=("MISSING")
        fi
        if [[ ${#problems[@]} -eq 0 ]]; then
            echo "ok       $engine/$name"
        else
            echo "FAIL     $engine/$name: ${problems[*]}"
            failures=$((failures + 1))
        fi
    done
    echo "[gguf-audit] $count models, $failures failing" >&2
    [[ "$failures" -eq 0 ]]
    exit $?
fi
REGISTRY_JSON="$("$TARGET_DIR/debug/voxtype-mirror-registry")"

mirror_one() {
    local engine="$1"
    local name="$2"
    local upstream="$3"
    local files_json="$4"

    echo "" >&2
    echo "[mirror] $engine/$name <- huggingface.co/$upstream" >&2

    if [[ "$DRY_RUN" -eq 1 ]]; then
        plan_one "$engine" "$name" "$upstream" "$files_json"
        return 0
    fi

    local workdir
    workdir="$(mktemp -d -t voxtype-mirror-XXXXXX)"
    trap 'rm -rf "$workdir"' RETURN

    # Build manifest while downloading.
    local manifest_files="[]"
    local total_size=0
    local file_count
    file_count="$(echo "$files_json" | jq 'length')"

    for i in $(seq 0 $((file_count - 1))); do
        local upstream_path local_path file_upstream url dest size sha
        upstream_path="$(echo "$files_json" | jq -r ".[$i].upstream_path")"
        local_path="$(echo "$files_json" | jq -r ".[$i].local_path")"
        file_upstream="$(echo "$files_json" | jq -r ".[$i].upstream_repo // \"$upstream\"")"
        if [[ "$upstream_path" == resolve/* ]]; then
            url="https://huggingface.co/$file_upstream/$upstream_path"
        else
            url="https://huggingface.co/$file_upstream/resolve/main/$upstream_path"
        fi
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

    rclone copy --progress "$workdir/" "$dest_path"
}

# --dry-run half of mirror_one: say what would be fetched and uploaded,
# without downloading a byte. A HEAD request per file still catches upstream
# files that have gone missing.
plan_one() {
    local engine="$1"
    local name="$2"
    local upstream="$3"
    local files_json="$4"

    local file_count missing=0
    file_count="$(echo "$files_json" | jq 'length')"

    for i in $(seq 0 $((file_count - 1))); do
        local upstream_path local_path file_upstream url
        upstream_path="$(echo "$files_json" | jq -r ".[$i].upstream_path")"
        local_path="$(echo "$files_json" | jq -r ".[$i].local_path")"
        file_upstream="$(echo "$files_json" | jq -r ".[$i].upstream_repo // \"$upstream\"")"
        if [[ "$upstream_path" == resolve/* ]]; then
            url="https://huggingface.co/$file_upstream/$upstream_path"
        else
            url="https://huggingface.co/$file_upstream/resolve/main/$upstream_path"
        fi
        echo "  would fetch $url -> $local_path" >&2
        if ! curl -fsSLI --retry 3 -o /dev/null "$url"; then
            echo "  WARN: HF 404 (or transient failure) on $upstream_path" >&2
            missing=1
            SKIPPED_MODELS+=("$engine/$name (missing: $upstream_path)")
        fi
    done

    if [[ "$missing" -eq 1 ]]; then
        echo "  WARN: a real run would skip model $engine/$name" >&2
    else
        echo "[mirror] would upload $file_count files + manifest.json to $R2_REMOTE/$engine/$name/" >&2
    fi
}

if [[ -n "$ENGINE_FILTER" ]]; then
    if ! echo "$REGISTRY_JSON" | jq -e --arg e "$ENGINE_FILTER" 'any(.[]; .engine_prefix == $e)' >/dev/null; then
        echo "error: no engine '$ENGINE_FILTER' in the registry; known engines:" >&2
        echo "$REGISTRY_JSON" | jq -r '[.[].engine_prefix] | unique | .[]' | sed 's/^/         - /' >&2
        exit 1
    fi
    while read -r entry; do
        engine="$(echo "$entry" | jq -r '.engine_prefix')"
        name="$(echo "$entry" | jq -r '.name')"
        upstream="$(echo "$entry" | jq -r '.upstream_repo')"
        files_json="$(echo "$entry" | jq -c '.files')"
        mirror_one "$engine" "$name" "$upstream" "$files_json"
    done < <(echo "$REGISTRY_JSON" | jq -c --arg e "$ENGINE_FILTER" '.[] | select(.engine_prefix == $e)')
elif [[ "$TARGET" == "--all" ]]; then
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
if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[mirror] dry run done; nothing downloaded or uploaded." >&2
else
    echo "[mirror] done." >&2
fi

if [[ ${#SKIPPED_MODELS[@]} -gt 0 ]]; then
    echo "" >&2
    echo "[mirror] WARNING: ${#SKIPPED_MODELS[@]} model(s) skipped due to upstream 404 or fetch failure:" >&2
    for entry in "${SKIPPED_MODELS[@]}"; do
        echo "  - $entry" >&2
    done
    echo "" >&2
    echo "[mirror] These registry entries point at upstream HuggingFace files that do not exist (or" >&2
    echo "         changed). Audit them in src/setup/model.rs and either fix the file list or" >&2
    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "         remove the entry. Nothing was downloaded or uploaded (--dry-run)." >&2
    else
        echo "         remove the entry. The remaining models were mirrored successfully." >&2
    fi
fi
