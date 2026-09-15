# GTK OSD startup latency

The GTK popup should acknowledge recording before the first microphone frame.
It displays “Starting microphone…” during device startup, then the waveform
when samples arrive. This improves visual feedback; it does not reduce the
microphone's wake-up time.

## Automated checks

```bash
cargo test --locked --features osd-gtk4 --bin voxtype-osd-gtk4
cargo test --locked --features osd-gtk4 --lib osd
```

These cover recording before audio, frames from a previous session,
cancellation, startup timeout, custom/disabled state paths, creating the state
file after the popup starts, file replacement, and supervised config forwarding.

## Live check

Run the branch's daemon and GTK popup on a Wayland desktop. Build with the
features needed for the configured transcription engine, for example:

```bash
cargo build --locked --features parakeet,osd-gtk4 \
  --bin voxtype --bin voxtype-osd --bin voxtype-osd-gtk4
```

1. Leave the microphone idle for at least six seconds, then hold the recording
   key. The startup message should appear promptly; it should not wait for
   the waveform. Speak after the waveform appears to check transcription.
2. Repeat immediately. The microphone may start faster, but popup feedback
   should remain prompt. Old waveform data should not appear during startup.
3. Cancel a recording before audio arrives. The startup message must disappear.
4. Run `voxtype record start --no-osd`, then `voxtype record cancel`. Neither
   the startup message nor the waveform should appear.
5. With `state_file = "disabled"`, confirm the existing audio-driven popup
   still works. With an explicit daemon `--config` and custom `state_file`,
   confirm immediate startup feedback still works.

For timing, compare the daemon's `Recording started` log timestamp with the
GTK popup's `showing` timestamp. The latter should report `starting=true`
when the microphone is still waking up. Measure the first frame on
`audio.sock` separately: popup visibility does not establish capture readiness.
