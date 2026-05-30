# Streaming + non-streaming model fail-fast (#442)

Pairing `[parakeet] streaming = true` with a non-streaming model
(e.g. `parakeet-tdt-0.6b-v3`) used to either fail opaquely on
`Failed to open tokenizer.model` or crash mid-chunk with an ONNX Gather
shape error. Now the streaming pipeline checks the registry before
loading and bails out with a message that names the model and the
streaming-compatible alternative.

## Unit test

```bash
cargo test --features parakeet --lib \
  transcribe::parakeet_streaming::tests::new_rejects_streaming_on_known_incompatible_model \
  -- --nocapture
# Expected: passes; error message names the model and the recommended replacement.
```

## Structural verification

```bash
grep -c "is_known_parakeet_model\|is_streaming_compatible_parakeet" \
  src/setup/model.rs src/transcribe/parakeet_streaming.rs
# Expected: 4+ (both helpers plus their call sites)
```

## Runtime test (requires `--features parakeet`)

1. Set config to a non-streaming Parakeet model with streaming on:
   ```bash
   cat <<EOF >> ~/.config/voxtype/config.toml
   engine = "parakeet"
   [parakeet]
   model = "parakeet-tdt-0.6b-v3"
   streaming = true
   EOF
   ```

2. Restart the daemon and inspect the journal:
   ```bash
   systemctl --user restart voxtype
   journalctl --user -u voxtype --since "30 seconds ago" | grep -A8 "Parakeet streaming"
   ```

   Expected error body:
   ```
   Parakeet streaming is enabled but model `parakeet-tdt-0.6b-v3` does not
   support cache-aware streaming.
   Fix one of:
     - Disable streaming in config.toml under [parakeet]:
         streaming = false
     - Or switch to the streaming-compatible model:
         voxtype setup model parakeet-unified-en-0.6b
     and set [parakeet] model = "parakeet-unified-en-0.6b" in config.toml.
   ```
