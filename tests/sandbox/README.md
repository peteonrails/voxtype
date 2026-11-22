# Sandbox tests

Scripts here drive a real voxtype against a **private** sound server and are
not part of `cargo test`. Run them inside an isolated session: its own `/dev`
(no `/dev/snd`), its own `XDG_RUNTIME_DIR`, and no access to the user's
PipeWire. They refuse to run when `/dev/snd` is visible, and they only ever
create null sinks, so nothing is audible.

A bwrap session that fits, with the worktree bound read-write:

```bash
WT=$PWD
systemd-run --user --scope --quiet -p MemoryMax=12G \
  bwrap --unshare-all --die-with-parent --ro-bind / / --dev /dev --proc /proc \
    --tmpfs /run/user/$UID --tmpfs /tmp --bind "$WT" "$WT" --bind ~/.cargo ~/.cargo \
    --chdir "$WT" --unsetenv WAYLAND_DISPLAY --unsetenv DISPLAY \
    --unsetenv PULSE_SERVER --unsetenv PIPEWIRE_REMOTE \
    --setenv XDG_RUNTIME_DIR /run/user/$UID \
    bash -c 'tests/sandbox/meeting-aec.sh "$PWD/target/debug/voxtype" you.wav remote.wav'
```

## meeting-aec.sh

Meeting echo cancellation end to end (`[meeting.audio] echo_cancel`). It
starts PipeWire, WirePlumber and pipewire-pulse with a `speakers` null sink
as the default output and a `mic` source fed from a second null sink, and
leaks `speakers` into `mic` at 0.12 gain with 30 ms of latency, as a laptop
speaker does. Arguments: the voxtype binary, then two 16 kHz mono WAVs, your
side and the remote side of a conversation (the remote side speaking while
you are silent). It needs `ggml-base.en.bin` in
`~/.local/share/voxtype/models` (or `VOXTYPE_TEST_MODEL`), and `ffmpeg`.

For `webrtc` and for `disabled` it records an echo-only meeting and a call
through the real daemon, with loopback capture off so transcript dedup
cannot hide the echo. It passes when:

- `meeting.json` and `meeting status --json` report the mode (`webrtc`,
  `none`);
- with `webrtc` at most one remote-only word reaches the mic transcript, and
  with `disabled` (the control) at least five do;
- the kept mic track's echo is at least 20 dB quieter with `webrtc`;
- a default source that is a filter over the mic (a `module-remap-source`,
  standing in for Camera Effects) is unwrapped: the canceller records the
  mic under it, and the echo still stays off the mic track;
- an unmapped `mic_device`, and a user's own echo canceller as the default
  source, leave the canceller off, each with its log line;
- after `kill -9` of the daemon mid-meeting, the canceller's source and
  process are gone.

What it cannot check: a real room. Real speakers add reverb, distortion and
clock drift that null sinks do not have; that check is step 13 of
`docs/smoke_tests/meeting-mode.md`, done by hand at a normal hour.
