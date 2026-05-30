# TUI Audio/VAD float serialization (#451)

The TUI used to write `volume = "0.70"` and `threshold = "0.50"` as quoted
strings, which the daemon's serde config rejected with `invalid type:
string "0.70", expected f32`. Saving any Audio or VAD change broke the
daemon on next start. Now serialized as bare TOML floats, with legacy
string and int forms still tolerated on load so existing configs migrate
on first save.

## Unit tests

```bash
cargo test --lib tui::config_editor::tests::set_f32 -- --nocapture
cargo test --lib tui::config_editor::tests::get_f32_or -- --nocapture
# Expected: set_f32_writes_toml_number_not_string passes
# Expected: get_f32_or_recovers_legacy_string_and_int_forms passes
```

## Structural verification

```bash
grep -c "set_string.*format!" src/tui/audio.rs src/tui/vad_section.rs
# Expected: 0 (the anti-pattern is gone)

grep -c "set_f32\|get_f32_or" src/tui/audio.rs src/tui/vad_section.rs
# Expected: 4+ (each section reads + writes)
```

## Runtime test

1. Open the TUI: `voxtype configure`
2. Audio section, change Volume, Save
3. Inspect `~/.config/voxtype/config.toml`:
   ```bash
   grep -A1 "audio.feedback" ~/.config/voxtype/config.toml
   # Expected: volume = 0.7 (bare number, no quotes)
   ```
4. `systemctl --user restart voxtype`
   Expected: daemon starts cleanly, no f32 parse error in journalctl

## Migration test

Hand-edit a quoted volume back in, then save via the TUI:

```bash
sed -i 's/^volume = 0\./volume = "0./; s/^\(volume = "0\.[0-9]*\)$/\1"/' ~/.config/voxtype/config.toml
```

Open the TUI, save, re-check:

```bash
grep volume ~/.config/voxtype/config.toml
# Expected: bare TOML number again after the save (auto-migration worked)
```
