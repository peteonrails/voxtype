# Microphone readiness and Omarchy settings

Build the local daemon and popup with the features for your transcription engine:

```bash
cargo build --locked --features parakeet,osd-gtk4 \
  --bin voxtype --bin voxtype-osd --bin voxtype-osd-gtk4
cargo test --locked --features parakeet,osd-gtk4 --lib --bin voxtype --bin voxtype-osd-gtk4
```

The capture tests cover idle-sample discard, stereo mixing, successive recording
isolation, incremental draining, cancellation, channel closure, and resampler
tail/reset behavior. The CLI tests cover both boolean overrides, conflicts,
and preserving configuration when neither flag is passed. Schema round trips
cover config writes and defaults, and the TUI parity test covers reachability.

An optional hardware test opens the default microphone briefly, discards audio,
verifies stream reuse/recovery, then releases it. It also checks suspending and
restoring readiness for meeting mode. It never saves or transcribes audio:

```bash
cargo test --locked --features parakeet,osd-gtk4 --lib \
  ready_microphone_reuses_the_real_input_stream -- --ignored
```

## Desktop check

1. Install the plugin using `omarchy-plugin/README.md`. For a local build, set
   `voxtypeBin` in the installed plugin's `dev.json` to the built binary.
2. Click the microphone bar icon. Audio should open with **Keep microphone
   ready** first, off by default. Verify the description wraps and the switch
   remains visible; the native icon should also fit a vertical bar.
3. Turn it on. Confirm `audio.keep_ready = true` in the config and a restart
   banner. Restart while the daemon is idle. The banner should clear and the
   daemon should log `Microphone kept ready; idle audio is discarded`.
4. Start/cancel recordings immediately and after at least six idle seconds.
   Measure first frames on `audio.sock` separately from popup visibility.
   The microphone indicator may remain active between recordings. The popup
   must remain hidden while idle and for recordings started with `--no-osd`.
5. Speak while idle, then record a distinct phrase. Only the recorded phrase
   should be transcribed. Cancel and repeat; previous audio must not appear.
6. Turn readiness off and restart. The device should be released between
   recordings. Change the config externally and verify the panel reflects it.
7. Unplug/reconnect an external microphone while idle, then retry a recording.
   Also try a failed meeting start and a normal meeting start/stop on a device
   that allows only one input stream. Readiness should resume afterward.

## Local measurements

On a Dell XPS with PipeWire, measured from `record start` to the first OSD audio
frame (September 2026): readiness off, cold **621–728 ms**; readiness on,
**59–91 ms**, including after six seconds idle. These numbers include capture,
resampling and level delivery; they are hardware-dependent, not a guarantee.

## Trial rollback

Turn **Keep microphone ready** off and restart Voxtype to restore on-demand
capture. To remove the UI, remove only `io.voxtype.settings` from the bar layout
and disable that plugin. If testing through a systemd override, remove that
specific override, reload user units, and restart the idle daemon to return to
the installed binary. Preserve unrelated bar and service customizations.
