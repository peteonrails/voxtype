#!/usr/bin/env bash
# mirror-flat-models-to-r2.sh - mirror the single-file models voxtype
# installs flat in its models directory (Whisper ggml models, the Silero VAD
# model, the ECAPA-TDNN speaker embedding model and the GTCRN speech
# enhancement model) into r2:voxtype-models, then verify every file through
# the public https://models.voxtype.io URL.
#
# The runtime never downloads these from Hugging Face or GitHub: it fetches
# {engine}/{model}/manifest.json from the mirror, then the model file, and
# checks its size and sha256 against the manifest (`download_mirrored_file`
# in src/setup/model.rs). This script is how those mirror directories are
# produced, so the process can be repeated when a model is added.
#
# Each mirror directory holds the model file, LICENSE, ATTRIBUTION.txt and a
# schema-version-1 manifest.json listing all three with their sizes and
# sha256s, plus `origin`, `license` and `object_prefix` provenance fields the
# runtime ignores. Upstream files are pinned to a revision, and where the
# upstream publishes a digest the staged bytes must match it.
#
# The ONNX model directories and the GGUF catalogue are handled by
# scripts/mirror-models-to-r2.sh instead.
#
# Usage: mirror-flat-models-to-r2.sh [stage|upload|verify|all] [model ...]
#   stage   download from upstream and build LICENSE/ATTRIBUTION/manifest
#   upload  rclone copy --immutable to r2:voxtype-models (never overwrites)
#   verify  re-hash every file and the manifest via the public URL
#   all     stage, upload and verify (default)
#   model   limit the run to these model names (e.g. tiny.en silero-v6.2.0)
#
# Prerequisites: curl, sha256sum, python3, and for upload an rclone remote
# named `r2` pointing at the Cloudflare account holding `voxtype-models`.
#
# Staging uses $STAGE (default ~/.cache/voxtype-mirror/stage). The large
# Whisper models total several GB, so keep it on disk, never on a tmpfs /tmp.
set -euo pipefail

STAGE="${STAGE:-${XDG_CACHE_HOME:-$HOME/.cache}/voxtype-mirror/stage}"
REMOTE="r2:voxtype-models"
PUBLIC="https://models.voxtype.io"
WHISPER_REV="5359861c739e955e79d9a303bcbc70fb988958b1"
VAD_REV="9ffd54a1e1ee413ddf265af9913beaf518d1639b"
ECAPA_REV="04c3ffe4fd00b3b7853fd57db44e2e531d4817f2"

# prefix|model|file|url|expected_sha256 (empty = no upstream digest)|license|license_urls(space)|origin_repo|revision|note
ENTRIES=(
"whisper|tiny|ggml-tiny.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-tiny.bin|be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|tiny.en|ggml-tiny.en.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-tiny.en.bin|921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|base|ggml-base.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-base.bin|60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|base.en|ggml-base.en.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-base.en.bin|a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|small|ggml-small.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-small.bin|1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|small.en|ggml-small.en.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-small.en.bin|c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|medium|ggml-medium.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-medium.bin|6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|medium.en|ggml-medium.en.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-medium.en.bin|cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|large-v3|ggml-large-v3.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-large-v3.bin|64d182b440b98d5203c4f9bd541544d84c605196c4f7b845dfa11fb23594d1e2|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"whisper|large-v3-turbo|ggml-large-v3-turbo.bin|https://huggingface.co/ggerganov/whisper.cpp/resolve/$WHISPER_REV/ggml-large-v3-turbo.bin|1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69|MIT|https://raw.githubusercontent.com/openai/whisper/main/LICENSE https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/LICENSE|ggerganov/whisper.cpp|$WHISPER_REV|OpenAI Whisper weights, converted to ggml by whisper.cpp"
"vad|silero-v6.2.0|ggml-silero-v6.2.0.bin|https://huggingface.co/ggml-org/whisper-vad/resolve/$VAD_REV/ggml-silero-v6.2.0.bin|2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987|MIT|https://raw.githubusercontent.com/snakers4/silero-vad/master/LICENSE|ggml-org/whisper-vad|$VAD_REV|Silero VAD v6.2.0, converted to ggml for whisper.cpp"
"diarization|ecapa-tdnn|ecapa_tdnn.onnx|https://huggingface.co/pranjal-pravesh/ecapa_tdnn_onnx/resolve/$ECAPA_REV/ecapa_tdnn.onnx|245eb5995cfffd74494862dee33da2b00c1c2579eb0c6703847784e9901ed458|MIT AND Apache-2.0|https://raw.githubusercontent.com/speechbrain/speechbrain/develop/LICENSE|pranjal-pravesh/ecapa_tdnn_onnx|$ECAPA_REV|ONNX export of SpeechBrain spkrec-ecapa-voxceleb. The conversion repo is labelled MIT; the SpeechBrain weights are Apache-2.0, whose license is included"
"enhancement|gtcrn-simple|gtcrn_simple.onnx|https://github.com/k2-fsa/sherpa-onnx/releases/download/speech-enhancement-models/gtcrn_simple.onnx||MIT|https://raw.githubusercontent.com/Xiaobin-Rong/gtcrn/main/LICENSE|k2-fsa/sherpa-onnx (release speech-enhancement-models)|speech-enhancement-models|GTCRN by Xiaobin Rong (MIT), ONNX export from sherpa-onnx. GitHub publishes no digest for this asset; the sha256 here is of the bytes downloaded on mirroring"
)

want() { # is entry selected?
  local m="$1"; shift; [[ $# -eq 0 ]] && return 0
  for x in "$@"; do [[ "$x" == "$m" ]] && return 0; done; return 1
}

stage_one() {
  IFS='|' read -r prefix model file url sha lic lurls repo rev note <<<"$1"
  local d="$STAGE/$prefix/$model"; mkdir -p "$d"
  if [[ ! -f "$d/$file" ]] || [[ -n "$sha" && "$(sha256sum "$d/$file" | cut -d' ' -f1)" != "$sha" ]]; then
    curl -sSfL --retry 5 -o "$d/$file.part" "$url"; mv "$d/$file.part" "$d/$file"
  fi
  local got; got=$(sha256sum "$d/$file" | cut -d' ' -f1)
  if [[ -n "$sha" && "$got" != "$sha" ]]; then echo "HASH MISMATCH $prefix/$model: $got != $sha" >&2; return 1; fi
  : > "$d/LICENSE"; local first=1
  for lu in $lurls; do
    [[ $first -eq 1 ]] || printf '\n\n==========\n\n' >> "$d/LICENSE"; first=0
    printf 'Source: %s\n\n' "$lu" >> "$d/LICENSE"; curl -sSfL "$lu" >> "$d/LICENSE"
  done
  cat > "$d/ATTRIBUTION.txt" <<EOF
Model: $model ($prefix)
Mirrored from: $repo at $rev
Original download: $url
$note.
Model license: $lic. See LICENSE.
Voxtype mirrors these exact bytes without retraining, fine-tuning or numeric changes.
Distribution does not imply endorsement or transfer trademark rights.
EOF
  python3 -I - "$d" "$prefix" "$model" "$file" "$repo" "$rev" "$lic" "$sha" <<'PY'
import hashlib, json, os, sys
d, prefix, model, file, repo, rev, lic, upstream_sha = sys.argv[1:]
def h(p):
    s=hashlib.sha256()
    with open(p,"rb") as f:
        for b in iter(lambda: f.read(1<<20), b""): s.update(b)
    return s.hexdigest()
files=[{"path":p,"size":os.path.getsize(os.path.join(d,p)),"sha256":h(os.path.join(d,p))}
       for p in [file,"LICENSE","ATTRIBUTION.txt"]]
m={"version":1,"model":model,"engine":prefix,"files":files,
   "origin":{"repo":repo,"revision":rev,"upstream_sha256":upstream_sha or None},
   "license":{"license":lic},"object_prefix":f"{prefix}/{model}"}
json.dump(m,open(os.path.join(d,"manifest.json"),"w"),indent=2); open(os.path.join(d,"manifest.json"),"a").write("\n")
PY
  echo "STAGED $prefix/$model $(stat -c %s "$d/$file") bytes sha256=$got"
}

upload_one() {
  IFS='|' read -r prefix model _ <<<"$1"
  rclone copy --immutable --checksum "$STAGE/$prefix/$model" "$REMOTE/$prefix/$model"
  echo "UPLOADED $prefix/$model"
}

verify_one() {
  IFS='|' read -r prefix model _ <<<"$1"
  local d="$STAGE/$prefix/$model" bad=0
  while IFS=$'\t' read -r path size sha; do
    local got; got=$(curl -sSfL --retry 3 "$PUBLIC/$prefix/$model/$path" | sha256sum | cut -d' ' -f1)
    if [[ "$got" == "$sha" ]]; then echo "OK   $prefix/$model/$path"; else echo "BAD  $prefix/$model/$path ($got != $sha)"; bad=1; fi
  done < <(python3 -I -c 'import json,sys; [print(f["path"],f["size"],f["sha256"],sep="\t") for f in json.load(open(sys.argv[1]))["files"]]' "$d/manifest.json")
  local m; m=$(curl -sSf "$PUBLIC/$prefix/$model/manifest.json" | sha256sum | cut -d' ' -f1)
  [[ "$m" == "$(sha256sum "$d/manifest.json" | cut -d' ' -f1)" ]] && echo "OK   $prefix/$model/manifest.json" || { echo "BAD  $prefix/$model/manifest.json"; bad=1; }
  return $bad
}

action="${1:-all}"; shift || true
rc=0
for e in "${ENTRIES[@]}"; do
  IFS='|' read -r prefix model _ <<<"$e"
  want "$model" "$@" || continue
  case "$action" in
    stage)  stage_one "$e" || rc=1 ;;
    upload) upload_one "$e" || rc=1 ;;
    verify) verify_one "$e" || rc=1 ;;
    all)    { stage_one "$e" && upload_one "$e" && verify_one "$e"; } || rc=1 ;;
  esac
done
exit $rc
