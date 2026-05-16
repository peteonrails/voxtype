# v0.7.2: Streaming Dictation, Modifier-Release Guard, Streaming Status Icon

Live transcription appears at the cursor as you speak, modifier keys no
longer collide with typed output, and Waybar gets a dedicated streaming
icon. Streaming requires toggle activation, not push-to-talk.

## Streaming Dictation (Experimental)

Parakeet streaming output types text incrementally while you dictate.
Powered by the `parakeet-rs` cache-aware streaming pipeline, it works
with `parakeet-unified-en-0.6b` (English) and runs on CPU, NVIDIA CUDA,
or AMD MIGraphX.

**Why use it:** Words appear as you speak instead of after you release a
key. Useful for long-form dictation, live notes, and interactive editing
where waiting for batch transcription breaks flow.

Enable in the TUI Advanced section or in config:

```toml
engine = "parakeet"

[parakeet]
model = "parakeet-unified-en-0.6b"
streaming = true

[hotkey]
mode = "toggle"
```

**Streaming requires toggle activation.** Push-to-talk does not work
with streaming output: voxtype types characters at the cursor while you
are still holding the key, and the synthetic key events from `wtype` or
`dotool` clobber the held-key state tracker that Hyprland, Sway, and
River use through libinput. After a few seconds the compositor decides
the key was already released, so the actual release event never fires
and the daemon stays in streaming mode.

Toggle activation sidesteps the issue entirely: no key is held during
the session. The daemon emits a strong warning and auto-promotes
`push_to_talk` to `toggle` for the running session when streaming is
enabled, and the TUI rewrites the config to match.

Tunable parameters control the latency/accuracy tradeoff:

```toml
[parakeet]
streaming_chunk_secs = 0.32         # how often partials are emitted
streaming_left_context_secs = 5.6   # history fed to each chunk
streaming_right_context_secs = 0.32 # lookahead per chunk
```

## Modifier-Release Guard ([#350](https://github.com/peteonrails/voxtype/issues/350))

When you bind voxtype to a chord like `Super+Ctrl+X`, the modifiers stay
physically pressed while wtype types the transcription. On some
compositors this caused the first typed letter to combine with the
modifiers and fire an unrelated keybinding (closing windows, switching
workspaces, opening menus).

The daemon now takes an `EVIOCGKEY` snapshot of keyboard state via evdev
before typing output, and waits for any held modifier to be released
before sending keystrokes. If the timeout is hit (default 750ms) the
output falls back to clipboard so the transcription is not lost, and a
desktop notification tells the user where the text went.

**Why use it:** No more "voxtype just closed my window" surprises after
a long dictation. Works on Hyprland, Sway, River, and any
compositor-agnostic setup where the user is in the `input` group.

Configure via:

```toml
[output]
wait_for_modifier_release = true     # default when input group is available
modifier_release_timeout_ms = 750
```

Disabled automatically during streaming (your modifiers are still down
because you are still actively dictating; the guard would never fire).

## Streaming Status Icon

Waybar status followers now distinguish "recording" (batch capture) from
"streaming" (live capture with cursor output). Custom themes can
override the streaming glyph via `[status.icons]`:

```toml
[status.icons]
streaming = "󰜟"  # nf-md-radio-handheld in Nerd Font
```

The built-in themes (omarchy, nerd-font, material, phosphor, codicons,
emoji, minimal, dots, arrows, text) each ship a sensible streaming
glyph. The Waybar `format-icons` map has a new `"streaming"` key. The
omarchy-voxtype-status script reports the new state.

## Notification Stacking ([#345](https://github.com/peteonrails/voxtype/issues/345))

Voxtype's status notifications ("Recording...", "Transcribing...",
"Transcribed", "Modifier key held...") now use the
`x-canonical-private-synchronous` and `transient` libnotify hints, so a
compositor with a notification daemon (mako, dunst, GNOME Shell, KDE)
overwrites the previous Voxtype notification in place instead of
stacking them in the notification history.

**Why use it:** The notification panel stays clean on GNOME/Ubuntu and
similar setups instead of accumulating a long trail of "Recording..."
/ "Transcribing..." / "Transcribed" entries every time you dictate.

Patch contributed by Stephan Schuster.

## Bug Fixes

- **Streaming cursor protection:** the parakeet backend used to emit
  one last `Final` event after `SIGUSR2`, which would type into
  whichever window had focus by the time the event arrived. The daemon
  now disowns the streaming session synchronously on stop so post-stop
  emissions are dropped instead of typed.

- **`voxtype record toggle` during streaming:** the toggle command
  checked for `"recording"` state only, so toggling during a streaming
  session would send `SIGUSR1` and start a second concurrent session
  instead of stopping the first. Both `"recording"` and `"streaming"`
  are now treated as active states.

- **Cohere model validator:** updated to match HuggingFace Optimum
  filenames so `voxtype setup model` validates the Cohere variants
  correctly.

- **GTK4 OSD startup visibility:** the OSD no longer flashes its chrome
  on the first frame at boot. Fix from André Silva.

- **Nix flake:** the OSD binaries are now packaged in the flake. From
  André Silva.

## Documentation

- `docs/INSTALL.md` rewritten end-to-end (clearer per-distro paths,
  no repetitive blocks).
- `docs/USER_MANUAL.md` and `docs/CONFIGURATION.md` document the
  streaming requirement and the modifier-release guard.

## Acknowledgments

- **André Silva** for the OSD startup-visibility fix and the Nix flake
  OSD packaging.
- **Stephan Schuster** for the notification-stacking patch (#345),
  including the libnotify hint research and a working diff.
- **Jean-Paul van Tillo** for early streaming feedback that informed
  the partial-typing implementation.

## Downloads

8 Linux binaries plus a macOS arm64 DMG. See the [Downloads](#downloads)
table at the bottom of this release. Checksums in `SHA256SUMS`.
