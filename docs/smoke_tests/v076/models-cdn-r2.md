# Model downloads use Cloudflare R2 with sha256 verification

`voxtype setup --download --model <name>` now fetches from
`https://models.voxtype.io/<engine>/<model>/manifest.json` and validates
each file's sha256 against the manifest. The five legacy per-engine
download functions collapsed into one `download_artifact<T: ModelArtifact>`.
HuggingFace remains as a fallback for users behind firewalls that block R2.

This is the v0.7.1 roadmap item that finally landed in v0.7.6.

## Structural verification

```bash
grep -c "MODELS_BASE_URL\|ModelArtifact\|download_artifact" src/setup/model.rs src/setup/manifest.rs
# Expected: 10+ across the two files
```

Confirm legacy per-engine download helpers are gone:

```bash
grep -c "fn download_parakeet_model_by_info\|fn download_moonshine_model_by_info\|fn download_cohere_model_by_info\|fn download_sensevoice_model_by_info\|fn download_onnx_model" src/setup/model.rs
# Expected: 0 (all consolidated into download_artifact<T: ModelArtifact>)
```

## Manifest fetch (no model required)

```bash
curl -sS https://models.voxtype.io/parakeet/parakeet-unified-en-0.6b/manifest.json | jq '{version, model, engine, files: (.files | length)}'
# Expected:
#   { "version": 1,
#     "model": "parakeet-unified-en-0.6b",
#     "engine": "parakeet",
#     "files": 5 }
```

## End-to-end download test

Run against an isolated `XDG_DATA_HOME` so the test doesn't touch a real install:

```bash
rm -rf /tmp/voxtype-r2-test
mkdir -p /tmp/voxtype-r2-test
XDG_DATA_HOME=/tmp/voxtype-r2-test voxtype setup --download --model parakeet-unified-en-0.6b
# Expected: "Downloading parakeet-unified-en-0.6b (5 files via https://models.voxtype.io/parakeet/...)"
# Expected: each of the 5 files downloads to 100% with no sha256 mismatch error

ls -la /tmp/voxtype-r2-test/voxtype/models/parakeet-unified-en-0.6b/
# Expected: encoder.onnx, encoder.onnx.data, decoder_joint.onnx, tokenizer.model, vocab.txt
```

Verify sha256 matches the manifest:

```bash
sha256sum /tmp/voxtype-r2-test/voxtype/models/parakeet-unified-en-0.6b/tokenizer.model
# Compare against the value in the manifest from the curl step above
```

## Cached-file revalidation

Re-running setup --download should skip files whose sha256 already matches
the manifest rather than re-downloading them:

```bash
XDG_DATA_HOME=/tmp/voxtype-r2-test voxtype setup --download --model parakeet-unified-en-0.6b
# Expected: completes fast; output indicates files were validated rather than re-fetched
```

## Mirror script

The R2 mirror script no longer aborts the whole run when one upstream HF
file 404s; it warns, skips the offending model, and prints a summary at
the end.

```bash
./scripts/mirror-models-to-r2.sh --all --dry-run 2>&1 | tail -20
# Expected: per-model "[mirror] foo/bar <- huggingface.co/..." lines
# Expected if any model has a stale registry entry: a final
# "WARNING: N model(s) skipped" summary listing them.
```

Confirm process-substitution is used (so `SKIPPED_MODELS` survives the loop):

```bash
grep -c "done < <(" scripts/mirror-models-to-r2.sh
# Expected: 1+
```
