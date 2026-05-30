# Engine vs binary mismatch detection (#450)

A persistent red banner appears on every TUI section when the configured
engine cannot be served by the running binary (e.g. `engine = "parakeet"`
but `/usr/bin/voxtype` dispatches to a CPU Whisper variant). F2 jumps to
the General variant picker. The daemon also fires a desktop notification
at startup so users who never open the TUI still see the warning.

This file also covers the strum-derived `TranscriptionEngine::name()`
refactor that the variant-check logic depends on: adding a new engine
no longer requires updating a separate name lookup table, and the wrong
spelling is mechanically impossible at compile time.

## Unit tests

```bash
cargo test --lib setup::variant_check -- --nocapture
# Expected: 5 tests pass:
#   whisper_engine_never_mismatches
#   parakeet_on_cpu_whisper_binary_is_mismatch
#   parakeet_on_onnx_binary_is_not_mismatch
#   source_install_recommends_rebuild
#   every_non_whisper_engine_has_a_required_feature
```

## Structural verification

```bash
grep -c "variant_mismatch\|VariantMismatch" src/tui/app.rs src/tui/mod.rs src/daemon.rs
# Expected: references in all three

grep -c "KeyCode::F(2)" src/tui/mod.rs
# Expected: 1 (the F2 binding to jump to General)

grep -c "jump_to_section" src/tui/app.rs src/tui/mod.rs
# Expected: 2+ (definition + at least one caller)
```

Engine name dedup (no manual name tables outside config.rs):

```bash
grep -c 'TranscriptionEngine::Parakeet => "parakeet"' src/menubar.rs src/setup/variant_check.rs src/config.rs
# Expected: 0 in menubar and variant_check; the only allowed home is
# config.rs (where strum derives it from the variant identifier).

grep -c '\.name()' src/menubar.rs src/setup/variant_check.rs
# Expected: 2+
```

## Runtime test (package install required; source builds use the rebuild path)

1. Force a mismatch: edit `config.toml` to use a Parakeet engine while
   `/usr/bin/voxtype` is the CPU variant:
   ```toml
   engine = "parakeet"

   [parakeet]
   model = "parakeet-unified-en-0.6b"
   ```

2. Confirm the wrapper is the CPU variant (not ONNX):
   ```bash
   readlink -f /usr/bin/voxtype
   # Expected: voxtype-avx2 or voxtype-avx512 (NOT voxtype-onnx-*)
   ```

3. `systemctl --user restart voxtype`
   Expected: desktop notification "Voxtype: parakeet unavailable"

4. `voxtype configure`
   Expected: red banner across every section reading:
   ```
   ! engine = parakeet but voxtype-avx2 was built without --features parakeet
     Fix: press F2 to open the variant picker, or run sudo voxtype setup onnx --enable
   ```

5. Press F2 from any section
   Expected: jumps to General; variant matrix focused

6. Press `?` in the TUI
   Expected: help overlay lists `F2  Jump to General (variant picker)` under Global

7. Fix the mismatch via the variant picker (or `sudo voxtype setup onnx --enable`)
8. Restart the daemon, re-open the TUI
   Expected: banner gone
