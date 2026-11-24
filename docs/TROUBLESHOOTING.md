# Voxtype Troubleshooting Guide

## Browsing GGUF models without installed weights

The graphical engine picker should stay on GGUF and show Download buttons while
the configured engine remains unchanged. Download failures and cancellation do
not activate a model. Retry Download, then choose Use after completion. A failed
Use shows a short error with optional details and preserves the working config.
Check the selected checkpoint, language and matching executable worker. No
download, config-key workaround or service restart is required merely to browse.

Solutions to common issues when using Voxtype.

## Start Here: `voxtype doctor`

Before reading further, run:

```bash
voxtype doctor
```

It checks your model files, VAD model, LARD cleanup model, GPU runtime libraries, audio device,
output tools, display environment of the systemd service, input device
access, notifications, OSD and meeting-mode tools, and the systemd unit, and
prints the exact fix for anything that is wrong (`✗` means broken, `!` means
degraded). Only features enabled in your config are checked. Most of the
problems below show up there with the fix on the next line.

The daemon also runs the quick part of these checks at startup and sends one
desktop notification listing any errors. If you got a "Voxtype: N problems
need attention" notification, `voxtype doctor` shows the same problems in
full.

## Ultra is missing, cannot download, or is not the current model

Use `voxtype info models --json` to distinguish catalogued, supported and locally
installed weights. Ultra F16/Q8_0/Q4_K_M need a GGUF-capable Parakeet build and its
matching worker. ONNX-only packages cannot load these files and keep their ONNX
default. Install a compatible build deliberately rather than renaming a file.
An existing explicit v3 selection is intentionally preserved, even when Ultra
weights are installed. Select Ultra explicitly to change it.

Ultra downloads come from Voxtype's model mirror at `models.voxtype.io`, like
every other model, and need network access to that host. Size/hash rejection does not replace an
existing good model. Retry a failed explicit download, not a settings save.
The automatic, read-only Ultra language display is expected: this runtime has
no selectable language hint for Parakeet. No translation or streaming is added.

## Model upgrade offer fails, is missing, or keeps showing

`voxtype setup migrate` prints the offers for the configured model, and
`voxtype setup migrate --json` prints the object the tray and Settings read.

- **No offer appears.** Offers need a build with the GGUF runtime for the engine
  (`voxtype info engines`) and a model in the offer table. Streaming Parakeet
  setups and models already on Parakeet Ultra get none.
- **Accept failed.** Download, checksum, activation and restart failures all
  leave `config.toml` byte-identical and keep the offer. The error names the
  step. A failed download needs network access to the model host; a busy daemon
  needs the current dictation or meeting to finish first. If the restarted
  daemon did not come up, voxtype puts the previous config back and restarts it;
  `journalctl --user -u voxtype -n 50` shows why the new model failed (or
  `journalctl _SYSTEMD_USER_UNIT=voxtype.service -n 50` when
  [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)).
- **Without a terminal**, accepting needs `--yes`.
- **The offer still shows after declining.** That is intended: declined offers
  stay listed, marked declined, so you can accept later. The tooltip line goes
  away. Declines are stored in `~/.local/state/voxtype/model_upgrade.json`;
  deleting that file brings the offer back as new.
- **`--cleanup` refuses.** It only runs after the daemon has transcribed
  successfully with the accepted model, and it only deletes the previous model
  when it sits directly in the models directory. Symbolic links and files
  elsewhere are never removed.
- **No Parakeet Ultra or GGUF format offer, only the ONNX notice.** Voxtype
  only offers a GGUF model when it can find the GGUF worker
  (`voxtype-gguf-worker` beside the voxtype binary, on `PATH`, or set with
  `VOXTYPE_GGUF_WORKER`). A package built without the worker files shows no
  offer, because accepting it could not work. The notice that ONNX support ends
  in 2.0 still shows. Install a voxtype package that includes the GGUF worker.

## "not available" next to an engine in `voxtype setup model`

The binary you are running was built without that engine. Voxtype prints the
fix for your install:

- **`run sudo voxtype setup onnx --enable`**: the ONNX build is already
  installed. This switches to it; switch back with `sudo voxtype setup onnx --disable`.
- **`rebuild voxtype-bin with the <variant> variant`**: your AUR `voxtype-bin`
  was built for this machine and left that variant out. Run
  `VOXTYPE_VARIANTS=<variant> yay -S --rebuild voxtype-bin`, or
  `VOXTYPE_VARIANTS=<variant> makepkg -si` from a clone of the AUR repo.
- **`reinstall the voxtype package`**: the package is missing a file it should
  have. Reinstall it with your package manager.
- **`rebuild with --features <name>`**: you built voxtype from source. Rebuild
  with that Cargo feature.

Whisper builds since 1.2.0 run Parakeet and Cohere as GGUF models with no ONNX
build at all. `voxtype setup model` lists them under "Parakeet GGUF" and
"Cohere Transcribe GGUF".

## Backend changed after upgrading voxtype-bin

The upgrade prints "Your previous backend (voxtype-X) is not in this build of
voxtype-bin." This happens when the package was built in a clean chroot or on a
different machine, so the build could not see the backend you use. Dictation
keeps working on the CPU build. Get your backend back with
`VOXTYPE_VARIANTS=X yay -S --rebuild voxtype-bin` (X is the name in the message
without `voxtype-`), then `sudo voxtype setup onnx --enable` or
`sudo voxtype setup gpu --enable` if needed.

If `voxtype setup onnx --enable` (or `setup gpu --enable`) says the variant is
not installed, check `/usr/lib/voxtype/package-variants` and rebuild with the
one you need, for example `VOXTYPE_VARIANTS=onnx-cuda-13 yay -S --rebuild voxtype-bin`.

## "Illegal CPU instruction (SIGILL)" from voxtype, the OSD or the audio bridge

voxtype prints one of three messages instead of a bare "Illegal instruction":

- **"This CPU does not support AVX2"**: the daemon variant needs a newer CPU
  than you have. Switch to the baseline build, which needs only x86-64-v2:
  `sudo voxtype setup variant --to voxtype-baseline`, then
  `systemctl --user restart voxtype`.
- **"This CPU is below x86-64-v2"**: the CPU lacks SSE4.2 or POPCNT (Intel
  Core 2 and older, AMD before Bulldozer). Every prebuilt binary, including
  the baseline daemon, the OSD and the audio bridge, is built for
  x86-64-v2, so none of them can run there. Check with
  `grep -o -w -e sse4_2 -e popcnt /proc/cpuinfo | sort -u`; both must be
  listed. Building from source with `RUSTFLAGS="-C target-cpu=native"` may
  work. If only the OSD crashes, set `[osd] enabled = false`. In a VM, enable
  CPU passthrough first: the host often meets the floor when the guest's CPU
  model does not.
- **"This binary is built for x86-64-v2 ... should run on your CPU"**: a
  build bug. Please report it with your `lscpu` output.

The CPU requirements for each binary are in
[INSTALL.md](INSTALL.md#cpu-prebuilt-linux-binaries).

## Status reports an ONNX backend for a GGUF model

Upgrade to a build where `voxtype status --format json --extended` reports
`GGUF ... (transcribe.cpp)` for `.gguf` models. GGUF models run in
`voxtype-gguf-worker` regardless of the main binary's ONNX variant. With
`gguf_backend = "auto"` the label reads `GGUF (transcribe.cpp)` until the worker
has loaded the model and reported Vulkan or CPU; with `on_demand_loading = true`
that happens at the first dictation.

## `voxtype info accel` with a GGUF model

A `.gguf` model runs in `voxtype-gguf-worker`, not in the main binary, so the
binary variant, the daemon's journal and the daemon's VRAM say nothing about
it. `voxtype info accel` reports what the worker bound instead: `State: gpu`,
the `Backend` (`vulkan`, `cuda`, `rocm` or `metal`) and the `Device`, or
`cpu-fallback` with backend `cpu` when the model runs on the CPU. The evidence
line says why: `<engine>.gguf_backend = "cpu"` asks for the CPU, while `auto`
lands there when the worker finds no usable GPU.

Before the first dictation, or with `on_demand_loading = true`, the worker has
not loaded the model yet. The state is `unknown` and the backend is the
configured `gguf_backend`. Dictate once and run it again.

`voxtype setup gpu --status` gives GGUF-specific advice too. The ONNX and
Vulkan variants ship a worker that can use Vulkan, so the backend is set with
`voxtype config set <engine>.gguf_backend vulkan` (or `cpu`) and
`setup gpu --enable` is not needed. The CPU variants ship a CPU-only worker;
there, `sudo voxtype setup gpu --enable` switches to the Vulkan variant and its
worker.

## Parakeet is compiled but startup requests an unavailable ONNX feature

Check the exact model with `voxtype config get parakeet.model`. A name such as
`parakeet-tdt-0.6b-v2` selects legacy ONNX, whereas a `.gguf` file selects the
GGUF worker. `info engines` reports engine-level compilation, not both formats.
The same distinction applies to Cohere.

Run `voxtype config validate-selection` for a format-specific diagnosis. Select
an already installed compatible checkpoint, or explicitly download one using
the command in the error. Do not merely restart repeatedly or replace a working
MIGraphX binary with a CPU build. Selection failure leaves the config unchanged;
an existing impossible config is explained but never silently migrated on load.

## Language menu is read-only or does not change

An English-only checkpoint or an engine without a verified selectable hint is
read-only. Auto is not offered for Cohere. An unrecognized custom model or
remote/CLI backend is not assumed to implement the local Whisper language list.
The submenu names the default checkpoint, not a temporary per-record model.

A language change during recording/transcription is saved for the next dictation;
it does not change or discard the active utterance. A save failure leaves the
previous checkmark, logs the cause and displays a menu explanation/notification.
Check file permissions and whether another UI changed the engine/model since the
daemon started. Do not restart in the middle of a recording to force a reload.

On the current Omarchy host, a second click reopens the menu instead of closing
it. That click is handled entirely by the shell for `ItemIsMenu=true`; the native
daemon never receives an Activate call. A separate isolated host patch is
provided for review, not applied to the installed shell by the local installer.

## Native tray does not appear

The tray is enabled by default: check `voxtype config get tray.enabled` and restart the daemon
after changing it. Explicit `false` disables it. A settings panel or recording OSD is not a tray host. Omarchy,
KDE and Waybar support StatusNotifierItem; GNOME needs its AppIndicator extension.

The tray uses embedded pixmaps even when optional hicolor icons are not installed.
Left-click opens the native menu; only its Settings row launches the full frontend.

Inspect the session bus with:

```sh
busctl --user get-property org.kde.StatusNotifierWatcher /StatusNotifierWatcher \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems
```

No session bus or host is nonfatal to dictation. A host starting late or restarting
is handled without restarting Voxtype. Idle is Active (not Passive), but a host
can still hide an item or put it in its drawer; look for the `voxtype` item ID in
its settings. Embedded icon pixmaps do not require a writable home directory.

If opening settings fails, run `voxtype-configure-launcher` from a terminal for
its diagnostic. Quickshell must have the settings plugin integrated and responding.
There is no bundled GTK settings app, and the GTK OSD is never used as one. If no
graphical frontend is installed, install a supported terminal or run
`voxtype configure` directly. Tray action failures are logged, not ignored.


## Omarchy Settings panel

`voxtype setup omarchy --status` shows how the panel is installed: `linked`
(the packaged copy, updated with voxtype), `linked-stale`, `copy`,
`foreign-link`, `absent`, `no-package` or `not-omarchy`.

- **Still shows the old version after an upgrade** (`linked-stale`). Restart the
  daemon (`systemctl --user restart voxtype`), which asks Omarchy to rescan. Or
  run `omarchy-shell shell rescanPlugins` yourself, or as a last resort
  `omarchy-restart-shell`.
- **Does not update with voxtype** (`copy`). The panel came from
  `omarchy plugin add`. Switch with `voxtype setup omarchy --replace`; your copy
  is backed up. `foreign-link` means the panel is a symlink to somewhere else,
  such as a development checkout, and voxtype leaves it alone in the same way.
  If you chose **Not now** on the Switch banner, it stays hidden for that copy;
  `--status` still shows `copy`, and `--replace` still switches.
- **Did not appear on first start.** If Omarchy's shell was not running yet, the
  daemon links the panel and enables it at its next start. To do it now, run
  `voxtype setup omarchy --restart-shell`.
- **Removed it and want it back.** Run `voxtype setup omarchy`. The daemon never
  re-adds it on its own.
- **"The Voxtype panel could not run ..."** The panel could not start the voxtype
  binary it names, so it shows nothing rather than stale values. If the message
  names a `dev.json` (next to the plugin's `manifest.json`), that file points at
  a build that no longer exists: fix the path, or delete the file to use
  `voxtype` from `PATH`. Otherwise voxtype is not installed or not on `PATH` for
  the shell. On Omarchy, run `omarchy-install-dictation-voxtype` (the panel's
  Install / Update button runs it). Reopen the panel after fixing it.
- **A red "voxtype info ... failed" strip.** `voxtype info engines`,
  `info devices` or `info styles` exited with the error shown. That list (engine
  availability, input devices, OSD styles) is left empty rather than showing old
  entries. Run the same command with `--json` in a terminal for the full error.

## Dictation history

### No entries after upgrading

Existing configurations keep history off unless you choose it. Run
`voxtype history status --json`, then `voxtype history enable` if you want
dictations stored. Enable and resume are different: an enabled but paused
history still writes nothing. The daemon reloads valid history settings without
a restart. Explicit daemon flags or environment overrides can keep the
effective setting different from the saved file.

Commands return nonzero on failure. If a config write fails validation or the
config cannot be read, do not assume that history was paused or disabled. Fix
the reported config error and read the state back before private dictation.

### An exclusion did not stop an entry

Exclusions use the focused app's class or app ID, not its window title. They
apply when either the starting or ending window is identified as excluded.
Unknown focus is still recorded: unsupported desktops, a failed compositor
connection, a timeout or an unrecognized response can leave the app unknown.
A password manager running inside a browser is seen as the browser. Pause
history before sensitive dictation when app identification cannot protect it.
Turning `record_app` off hides the recorded app field; it does not disable
exclusion checks or make unknown focus fail closed.

### History files exist but search is missing or stale

Day files are authoritative; `index.db` is derived. Run
`voxtype history reindex --json` and inspect both stdout and its exit status.
Unreadable day files are reported, not silently certified as a complete rebuild.
Days older than a week can be `.jsonl.gz`; do not rename them to `.jsonl`.

A symlinked history directory or day file is rejected rather than followed.
Links owned by root above the history directory, such as `/var` on macOS or
`/home` on atomic Fedora, are part of the system layout and are allowed.
Rewrites use exclusively created, randomly named temporary files instead of
opening a pre-existing temporary path. Check the path reported by the error. Do not solve
this by broadly changing permissions or recursively deleting files. Preserve
the data before correcting an unintended link or ownership problem.

### A delete reports that SQLite readers block WAL cleanup

The authoritative file change may already have completed, but cleanup is not
finished. Close other history and meeting views, then run
`voxtype history reindex --json` and require a successful exit. A successful
file rewrite alone does not mean old WAL pages were cleared. Filesystem
snapshots, backups and previously allocated disk blocks are outside SQLite's
secure-delete guarantee; use full-disk encryption for data at rest.

### A correction is refused for a newer schema

Upgrade Voxtype before editing that entry. The original file is left unchanged
so unknown nested fields are not lost. An empty correction is intentional: it
must not restore the original text when copying or re-sending.

### `history copy` or `history paste` says there is no text

The entry is empty: the engine returned nothing, or you saved an empty
correction. There is nothing to copy or type, so the command exits nonzero
and leaves the clipboard unchanged. Give the entry text with
`voxtype history correct <ID> "TEXT"` if you want to copy it.

## Meeting view

The meeting view (in the Omarchy plugin and in voxtype's Quickshell shell) says
what is wrong instead of showing an empty pane:

- **"Voxtype is not running"**: the daemon is down. Start it with
  `systemctl --user start voxtype`. Past meetings still list and open.
- **"Voxtype is running without its meeting socket"**: the daemon answers but
  does not publish `$XDG_RUNTIME_DIR/voxtype/meeting.sock`, usually because it
  is older than 1.3.0. Update voxtype and `systemctl --user restart voxtype`.
- **"Could not run voxtype. Is voxtype installed and on PATH?"**: the shell
  cannot start the voxtype command. In the Omarchy plugin, check `dev.json` as
  for the Settings panel above.
- **"This voxtype is too old for …"**: the command for notes, Ask, summaries or
  moving lines does not exist in the installed voxtype. Update it, then reopen
  the view (and restart the shell after an install, so it loads the new view).
- **"No agent or local model was found"** on the Ask or Summary tab: questions
  and summaries need Claude Code or Codex on `PATH`, or Ollama for a local
  model. `voxtype meeting assistant status` shows what was found and why an
  agent cannot be used. Codex needs `bwrap` to run in its jail; without it the
  agent menu shows it disabled, with the reason.
- **"Copying needs wl-copy"**: install `wl-clipboard`.
- **"Couldn't write the summary"** at the top of the Summary tab: the backend
  that writes summaries failed, and the card says which and why. The chip in
  the tab's header names that backend; it can be a local model (an older
  `[meeting.summary]` backend such as Ollama) even when questions go to an
  agent. "status code 500" from `localhost:11434` means Ollama is running but
  failed: check `ollama list` for the model, or choose **Use Claude instead**
  (shown when a local model failed and you have allowed an agent), which runs
  `voxtype meeting summarize <id> --agent claude` for this meeting only.

## Table of Contents

- [Start Here: `voxtype doctor`](#start-here-voxtype-doctor)

- [Modifier Key Interference (Hyprland/Sway/River)](#modifier-key-interference-hyprlandswayriver)
- [Hotkey Detection on KDE Plasma](#hotkey-detection-on-kde-plasma)
- [Hotkey Detection with Key Remappers (keyd)](#hotkey-detection-with-key-remappers-keyd)
- [Permission Issues](#permission-issues)
- [Audio Problems](#audio-problems)
- [Transcription Issues](#transcription-issues)
  - [GGUF worker selection](#gguf-worker-selection)
- [Voice Activity Detection (VAD)](#voice-activity-detection-vad)
- [Output Problems](#output-problems)
  - [wtype not working on KDE Plasma or GNOME Wayland](#wtype-not-working-on-kde-plasma-or-gnome-wayland)
  - [Text output not working on X11](#text-output-not-working-on-x11)
  - [Wrong characters on non-US keyboard layouts](#wrong-characters-on-non-us-keyboard-layouts-yz-swapped-qwertz-azerty)
  - [Edit-list post-processing keeps the original text](#edit-list-post-processing-keeps-the-original-text)
  - [Copied to clipboard, paste manually](#copied-to-clipboard-paste-manually-notification)
  - [Transcription not delivered](#transcription-not-delivered-notification)
- [Performance Issues](#performance-issues)
- [ElevenLabs Backend Issues](#elevenlabs-backend-issues)
- [Soniox Backend Issues](#soniox-backend-issues)
- [Media Does Not Pause While Recording (Omarchy Quattro)](#media-does-not-pause-while-recording-omarchy-quattro)
- [Quickshell OSD Issues](#quickshell-osd-issues)
- [Meeting Audio and Post-Meeting Command](#meeting-audio-and-post-meeting-command)
  - [The recording announcement only played on my speakers](#the-recording-announcement-only-played-on-my-speakers)
  - [The announcement used the built-in voice instead of Jessica](#the-announcement-used-the-built-in-voice-instead-of-jessica)
- [Meeting Assistant: Summaries Don't Appear](#meeting-assistant-summaries-dont-appear)
- [No Meeting Window Appears](#no-meeting-window-appears)
- [Action Items: Todoist and Reminders](#action-items-todoist-and-reminders)
- [Meeting Titles: The Calendar Title Was Not Used](#meeting-titles-the-calendar-title-was-not-used)
- [Systemd Service Issues](#systemd-service-issues)
- [Debug Mode](#debug-mode)

---

## Modifier Key Interference (Hyprland/Sway/River)

### Typed text triggers window manager shortcuts instead of inserting text

**Symptoms:** When using compositor keybindings with modifiers (e.g., `SUPER+CTRL+X` or `SUPER+O`), releasing keys slowly causes typed output to trigger shortcuts instead of inserting text. For example, if you release X while still holding SUPER, the transcribed "hello" might trigger SUPER+h, SUPER+e, SUPER+l, etc.

**Cause:** The compositor tracks physical keyboard state. Even though voxtype types text, if you're still physically holding a modifier key, the compositor combines them.

**Solution:** Use the compositor setup command to automatically install a fix that blocks modifier keys during text output.

**For Hyprland:**
```bash
voxtype setup compositor hyprland
hyprctl reload
systemctl --user restart voxtype
```

**For Sway:**
```bash
voxtype setup compositor sway
swaymsg reload
systemctl --user restart voxtype
```

**For River:**
```bash
voxtype setup compositor river
# Restart River or source the new config
systemctl --user restart voxtype
```

**Note:** This command does NOT set up keybindings—it only installs the modifier interference fix. See the [User Manual](USER_MANUAL.md#compositor-keybindings) to set up your push-to-talk hotkey.

This command:
1. Writes a modifier-blocking submap/mode to `~/.config/hypr/conf.d/voxtype-submap.conf` (or `sway/conf.d/voxtype-mode.conf`, or `river/conf.d/voxtype-mode.sh`)
2. Adds pre/post output hooks to your voxtype config
3. Checks that your compositor config sources the conf.d directory

If voxtype crashes while typing, press **F12** to escape the submap and restore normal modifier behavior.

**Manual setup:** See `voxtype setup compositor hyprland --show` for the full configuration if you prefer to set it up manually.

**Alternative workaround:** If you can't use submaps, a simple delay before typing may help:

```toml
[output.post_process]
command = "sleep 0.3 && cat"
timeout_ms = 5000
```

### Compositors Without Mode/Submap Support

The automatic fix (`voxtype setup compositor`) only works on compositors that support input modes or submaps:

| Compositor | Supported | Why |
|------------|-----------|-----|
| Hyprland | Yes | Has submaps |
| Sway | Yes | Has modes |
| River | Yes | Has modes |
| Qtile | No | No mode/submap concept |
| Niri | No | No mode/submap concept |
| GNOME | No | No mode/submap concept |
| KDE | No | No mode/submap concept |

**For unsupported compositors, use one of these alternatives:**

1. **Use a dedicated key without modifiers** - Keys like ScrollLock, Pause, or F13-F24 don't have this problem since there are no modifiers to interfere:
   ```toml
   [hotkey]
   key = "SCROLLLOCK"
   ```

2. **Use the post-processor delay** (works on any compositor):
   ```toml
   [output.post_process]
   command = "sleep 0.3 && cat"
   timeout_ms = 5000
   ```
   This gives you 300ms to release all keys before typing starts.

3. **Use voxtype's built-in evdev hotkey** instead of compositor keybindings - release timing doesn't matter since voxtype controls the entire recording flow.

---

## Hotkey Detection on KDE Plasma

### Meta+modifier hotkeys not detected (evdev mode)

**Symptoms:** When using evdev hotkey detection (`[hotkey] enabled = true`) on KDE Plasma, hotkeys like Meta+Shift, Meta+Ctrl, or Meta+Alt are not detected. Voxtype shows no events in debug logs when these keys are pressed. Non-modifier keys with Meta work fine (e.g., Meta+RightAlt).

**Example:**
```toml
[hotkey]
enabled = true
key = "LEFTSHIFT"
modifiers = ["LEFTMETA"]
```

Pressing Meta+Shift produces no debug output. Changing to `key = "RIGHTALT"` with the same modifiers works correctly.

**Cause:** KDE Plasma grabs Meta+modifier combinations (Meta+Shift, Meta+Ctrl, Meta+Alt) at the compositor level for desktop switching, window management, and other shortcuts. The compositor handles these key combinations before they reach evdev, preventing voxtype from receiving the events. This is compositor behavior, not a voxtype bug.

The kernel delivers these events correctly (visible via `sudo evtest`), but KDE prevents them from reaching applications using evdev.

**Workarounds:**

**1. Use compositor keybindings instead of evdev (recommended)**

Disable evdev hotkey detection and bind voxtype commands directly in KDE System Settings:

```toml
[hotkey]
enabled = false  # Disable evdev, use compositor bindings instead
```

Then set up KDE shortcuts:
1. Open System Settings → Shortcuts → Custom Shortcuts
2. Create a new shortcut for `voxtype record toggle`
3. Assign it to your desired key combination (e.g., Meta+Shift)

This approach works reliably because the compositor processes the hotkey before any grabbing happens.

**2. Disable conflicting KDE shortcuts**

If you want to use evdev mode, remove KDE shortcuts that use your target hotkey:

1. Open System Settings → Shortcuts
2. Search for shortcuts using Meta+Shift (or your target combination)
3. Disable or rebind conflicting shortcuts
4. Test with `voxtype -vv` to verify events are received

**3. Use non-modifier target keys**

Keys like F13-F24, ScrollLock, or Pause are not grabbed by KDE:

```toml
[hotkey]
enabled = true
key = "SCROLLLOCK"
modifiers = ["LEFTMETA"]  # Meta+ScrollLock works
```

Or use a single non-modifier key without any modifiers:

```toml
[hotkey]
enabled = true
key = "SCROLLLOCK"  # No modifiers needed
```

**4. Use Alt+Shift with correct key ordering**

On KDE, Alt+Shift has strict ordering requirements. This configuration works reliably:

```toml
[hotkey]
enabled = true
key = "LEFTALT"
modifiers = ["LEFTSHIFT"]  # Press Shift first, then Alt
```

Press Shift first, then Alt while holding Shift. This avoids KDE keyboard layout switching and works consistently.

**Note:** This is not a voxtype limitation. Any application using evdev on KDE Plasma will experience the same behavior with Meta+modifier hotkeys. The compositor keybinding approach is the most reliable solution on KDE.

---

## Hotkey Detection with Key Remappers (keyd)

### Right Ctrl, Right Shift, or Right Meta arrives as the left key

**Symptoms:** With [keyd](https://github.com/rvaiya/keyd) running, a hotkey set to `RIGHTCTRL`, `RIGHTSHIFT`, or `RIGHTMETA` never fires, and a `modifiers` entry for one of those keys never matches. `sudo evtest` on the "keyd virtual keyboard" device shows `KEY_LEFTCTRL` (or `KEY_LEFTSHIFT` / `KEY_LEFTMETA`) when you press the right-hand key, and never the right-hand name.

**Cause:** keyd grabs the physical keyboard and re-emits every event from its own "keyd virtual keyboard" device, which is the device evdev hotkey detection reads. keyd's built-in defaults treat the left and right variants of Ctrl, Shift, and Meta as one key: `rightcontrol = control`, `rightshift = shift`, and `rightmeta = meta` are bound to the matching modifier layer, and that layer emits the **left** key. This is keyd's design, not a voxtype bug. The defaults are the same in keyd v2.6.0, the latest release at the time of writing, so upgrading keyd does not change it.

`RIGHTALT` is not merged by default: it maps to keyd's `altgr` layer, which emits `RIGHTALT`. It is affected only if your keyd config binds it to another layer (for example `rightalt = layer(alt)` or `rightalt = overload(alt, ...)`), in which case holding it emits `LEFTALT`.

**Check what voxtype receives:** run `sudo evtest`, pick the "keyd virtual keyboard" device, press your key, and read the event name. That name is what `[hotkey] key` has to match.

**Workarounds:**

- Use a key that keyd does not remap, such as `SCROLLLOCK`, `PAUSE`, or `F13` through `F24`. A key with no binding in your keyd config passes through unchanged.
- If any Ctrl key is acceptable, set `key = "LEFTCTRL"`. Under keyd, it fires for both Ctrl keys.

---

## Permission Issues

### "Cannot open input device" or "Permission denied"

**Cause:** The built-in evdev hotkey reads `/dev/input/event*` directly, and only members of the `input` group may. Your user is not one, which is the safer state: membership lets every program you run read every keystroke. Some distributions, Omarchy among them, remove users from the group on purpose.

**Solution (recommended):** trigger voxtype from a compositor or desktop keybinding instead. Bind `voxtype record start` and `voxtype record stop` (or `voxtype record toggle` where the desktop has no key-release binds), then turn the built-in hotkey off:

```toml
[hotkey]
enabled = false
```

This works on Hyprland, Sway, River, niri, KDE Plasma and GNOME without any extra permissions. See [Compositor Keybindings](USER_MANUAL.md#compositor-keybindings).

**Only if nothing can run `voxtype record`** (some X11 window managers, the console): adding yourself to the `input` group makes the built-in hotkey work, at the cost described above. Read [The `input` group trade-off](USER_MANUAL.md#the-input-group-trade-off) before you decide. Never do this on Hyprland or Omarchy, where a keybinding does the same job.

When `[hotkey] enabled = true` and the daemon cannot open any keyboard, it keeps running for `voxtype record` commands and shows a "Voxtype: built-in hotkey unavailable" desktop notification. It is shown once, not on every start, and again only if the cause changes or the built-in hotkey breaks after having worked; the daemon log has the warning on every start. The notification names the cause:

- **Not in the `input` group:** bind `voxtype record` in your compositor and set `[hotkey] enabled = false`, as above. The group is a deliberate trade-off, not the default fix.
- **In the group, but this session started before you were added:** `/etc/group` lists you, but the running daemon does not have the group yet. Log out completely and back in, or reboot. A systemd user service keeps the old groups until every session of yours has ended.
- **In the group and still denied:** the device nodes have unusual ownership or permissions. Check `ls -l /dev/input/event*`.
- **No keyboard found:** nothing in `/dev/input` looks like a keyboard. Plug one in and restart voxtype.

If you already use a compositor keybinding, your hotkey works and this notification is only about the built-in one: `[hotkey] enabled = false` (or starting the daemon with `--no-hotkey`) turns the built-in hotkey off and stops the notification.

### "Failed to access /dev/input/event*"

**Cause:** The built-in evdev hotkey cannot read the keyboard: you are not in the `input` group, the membership has not reached this session yet, or udev rules give the devices an unusual owner.

**Solution:** Prefer a compositor keybinding and `[hotkey] enabled = false` (see above). If you have deliberately chosen the `input` group ([trade-off](USER_MANUAL.md#the-input-group-trade-off)):
1. Verify group membership: `groups | grep input`
2. If not shown, log out and back in completely
3. If still failing, check udev rules:
```bash
ls -la /dev/input/event*
# Should show group 'input' with rw permissions
```

### "Unable to create uinput device" (ydotool)

**Cause:** uinput module not loaded or wrong permissions.

**Solution:**
```bash
# Load uinput module
sudo modprobe uinput

# Make it persistent
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf

# Check ydotool daemon
systemctl --user status ydotool
```

If the module is loaded and ydotoold still cannot open `/dev/uinput`, give it write access through a dedicated group rather than the `input` group: see [Giving dotool and ydotool access to uinput](USER_MANUAL.md#giving-dotool-and-ydotool-access-to-uinput).

---

## Audio Problems

### "Voxtype: microphone not found" or "could not open the microphone" notification

The daemon posts this notification when you start a recording and the microphone can't be opened. It appears once per distinct failure (again after five minutes if the failure persists), so pressing the hotkey repeatedly doesn't stack notifications. The full error is in the log (`journalctl --user -u voxtype`, or `journalctl _SYSTEMD_USER_UNIT=voxtype.service` when [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)).

- **microphone not found:** the device named in `[audio] device` isn't connected. List what is available with `voxtype info devices`, then `voxtype config set audio.device <name>` (or `default` to follow the system default) and restart voxtype.
- **no microphone found:** the system has no default input source. Plug in or unmute a microphone and check `wpctl status` or `pactl list sources short`.
- **could not open the microphone:** the audio server refused the stream. Check that PipeWire or PulseAudio is running: `systemctl --user status pipewire wireplumber`.

### "No audio captured" or empty transcriptions

**Possible causes and solutions:**

#### 1. Wrong audio device selected

```bash
# List available audio sources
pactl list sources short

# Test recording with system default
arecord -d 3 -f S16_LE -r 16000 test.wav
aplay test.wav
```

If test recording works, check your Voxtype config:
```toml
[audio]
device = "default"  # Or specific device name from pactl
```

#### 2. Microphone muted or volume too low

```bash
# Check PulseAudio/PipeWire volume
pavucontrol
# Or
pactl list sources | grep -A 10 "Default"
```

#### 3. PipeWire/PulseAudio not running

```bash
# Check audio server status
pactl info

# Restart if needed
systemctl --user restart pipewire pipewire-pulse
# Or for PulseAudio:
systemctl --user restart pulseaudio
```

### "No sound server answered within 5 s"

Voxtype asked the audio system for the microphone and got no reply. This happens when PipeWire or PulseAudio is wedged (its socket exists but it does not answer), or in a sandbox, container or clean chroot with no sound server. Recording, `voxtype info devices` and the Settings panel's device list stop waiting after 5 seconds and report this instead of hanging.

```bash
systemctl --user status pipewire pipewire-pulse wireplumber
systemctl --user restart pipewire pipewire-pulse wireplumber
voxtype info devices
```

If `info devices` lists your microphone again, recording works too; the daemon does not need a restart.

### "Microphone silent" notification stops a recording

**Cause:** Every sample the input delivered for the first 5 seconds of the
recording was exactly zero. This usually means the
selected microphone is off, muted in hardware, or asleep. The notification
names the audio source that stayed silent.

**Solution:** Turn the microphone on, unmute it, or select another source:

```bash
pactl get-default-source
pactl list sources short
pavucontrol
```

If the source is noise-gated and you often wait more than 5 seconds before
speaking, or it takes longer than that to wake, increase `window_ms` or
disable the guard. The threshold cannot change the verdict: only exact
digital silence stops a recording.

```toml
[audio.dead_input]
window_ms = 10000
# enabled = false
```

A source stuck on a DC offset is not stopped: its samples are not zero. If a
dictation still produces no text without this alert, record the source with
`arecord` and check the playback.

### "Audio format not supported"

**Cause:** Audio device doesn't support 16kHz sample rate.

**Solution:** Voxtype handles resampling internally, but ensure your device works:
```bash
# Test at native rate
arecord -d 2 test.wav
aplay test.wav
```

### Feedback cues: start cue clipped on Bluetooth headphones

**Cause:** Since 1.3.2 voxtype opens an audio output only while a feedback cue plays, and closes it afterwards. Earlier versions held one output open for the daemon's lifetime, which broke after suspend and resume (cues went silent) and could spin a CPU core on `alsa::poll() returned POLLERR`. With nothing open while idle, PipeWire suspends the sink, and a Bluetooth sink can take a moment to wake, so the first part of the start cue may be lost. The same change lets multipoint headphones hand off to another device while voxtype is idle.

**Solution:** If the clipping bothers you more than the idle sink suspending, tell WirePlumber not to suspend sinks. For example, in `~/.config/wireplumber/wireplumber.conf.d/51-no-suspend.conf`:

```
monitor.bluez.rules = [
  {
    matches = [ { node.name = "~bluez_output.*" } ]
    actions = { update-props = { session.suspend-timeout-seconds = 0 } }
  }
]
```

Then `systemctl --user restart wireplumber`. A warning `Audio output stalled, feedback cue abandoned` in the log means the output device stopped accepting audio; voxtype gives up on that cue after its length plus one second and opens a fresh output for the next one.

### Recording stops unexpectedly

**Cause:** `max_duration_secs` limit reached. Voxtype shows a "Recording Limit Approaching" notification shortly before the limit (30 seconds before for limits of two minutes or more, 10 seconds before for shorter ones) and a "Recording Limit Reached" notification when it stops. What was captured up to the limit is still transcribed.

**Solution:** Increase the limit:
```toml
[audio]
max_duration_secs = 120  # 2 minutes
```

Or from the command line: `voxtype config set audio.max_duration_secs 1800`, or `voxtype --max-duration 1800` for a single daemon run.

### Transcription failed and I lost my dictation

A recording whose transcription fails is not thrown away. Voxtype saves it as a WAV file in `~/.local/state/voxtype/failed-audio/` (or `$XDG_STATE_HOME/voxtype/failed-audio/`), logs the path, and shows a "Transcription Failed" notification with the command to retry it:

```bash
voxtype transcribe ~/.local/state/voxtype/failed-audio/recording-20261020-143015.wav
```

The 20 most recent failed recordings are kept; older ones are removed automatically. The directory and the files in it are private to your user: other accounts on the machine can't read them.

To stop Voxtype keeping these recordings:

```bash
voxtype config set audio.keep_failed_recordings false
systemctl --user restart voxtype
```

With it off nothing is written, so a dictation whose transcription fails can't be retried, and the "Transcription Failed" notification says the recording was not kept. Recordings saved before you turned it off stay until you delete them: `rm -r ~/.local/state/voxtype/failed-audio/`.

### Meeting transcript has no remote participants

**Cause:** Meeting mode records other participants through a PulseAudio/PipeWire monitor source via `pactl` and `parec`. When those tools, the PulseAudio server, or a monitor source are missing, the meeting records the microphone only. The daemon shows a "Meeting started with limitations" notification saying which one.

**Solution:** Install `libpulse` (Arch) or `pulseaudio-utils` (Debian, Ubuntu, Fedora); on PipeWire, make sure `pipewire-pulse` is running; or set `[meeting.audio] loopback_device` to a source from `pactl list short sources`. See [Meeting Mode troubleshooting](MEETING_MODE.md#troubleshooting) for the other meeting notifications.

### One side of the meeting is not being transcribed

**Symptom:** A notification with this title during a meeting (or "Neither side of the meeting is being transcribed" when it is both), a `Not transcribed:` line in `voxtype meeting status`, or a transcript that has one side of the conversation and nothing from the other, although that side was talking.

**Cause:** Each chunk of a track has to pass a speech check before it is transcribed: 300 ms of it louder than `[meeting.audio] vad_threshold` (default `0.01`). When the threshold is above the level a track reaches, every chunk of it is skipped as silence. The usual way to get there is raising `vad_threshold` to keep noise out and going past the level of your own voice. The check to see it for yourself:

```bash
voxtype config get meeting.audio.vad_threshold
voxtype meeting status     # Not transcribed: Your microphone has sound, but nothing you say is transcribed: it reaches about 0.052 and [meeting.audio] vad_threshold is 0.11. ...
journalctl --user -u voxtype | grep "not being transcribed"
```

**Solution:** Lower the threshold with the command the notification gives, then restart voxtype once the meeting is over (the running meeting keeps the threshold it started with, and a restart interrupts it):

```bash
voxtype config set meeting.audio.vad_threshold 0.01
systemctl --user restart voxtype
```

If the meeting keeps its audio (`[meeting] retain_audio = true`), get the missing lines back after it ends:

```bash
voxtype meeting retranscribe latest
```

If the notification says the threshold is already at or below the default, the track is too quiet: raise the microphone's input volume, or the call's volume in the app that plays it, or lower the threshold to the value the notification gives. See [Meeting Mode troubleshooting](MEETING_MODE.md#troubleshooting) for when it is raised and where else it shows.

The notification is shown whatever `[meeting] status_notifications` is set to, like the other meeting error notifications; it comes at most once per side per meeting.

The notification does not cover a track with no sound at all: a muted or wrong microphone, a loopback that records nothing, or an echo canceller that delivers silence. Check those with `voxtype meeting soundcheck`.

---

## Transcription Issues

### Shared GGUF selection is refused

The `gguf` engine validates the selected checkpoint's architecture, language,
file size/header, compiled client, and companion worker before saving an engine
or model change. Download the named model first with
`voxtype setup --download --model <filename>` and keep the worker shipped with
the same Voxtype build. `streaming = true` is intentionally unsupported. An
explicit `vulkan` backend also fails when no hardware Vulkan device is visible;
use `auto` or `cpu` only when that is the backend you intend.

### GGUF worker selection

Cohere and Parakeet GGUF models run in a companion worker process. Release
packages install a generic CPU worker named `voxtype-gguf-worker` and may also
install workers for accelerated binary variants, such as
`voxtype-gguf-worker-vulkan`.

Voxtype selects the worker that matches the running executable. For example,
`voxtype-vulkan` first tries `voxtype-gguf-worker-vulkan` in the same directory,
then falls back to `voxtype-gguf-worker`. This also works when `voxtype` is a
symlink to a variant binary, as created by `voxtype setup gpu --enable`.

Set `VOXTYPE_GGUF_WORKER` to use a worker at another path or to diagnose a
packaging problem:

```bash
VOXTYPE_GGUF_WORKER=/path/to/voxtype-gguf-worker-vulkan voxtype daemon
```

The override always takes priority. If a worker cannot initialize, the error
names the selected worker path so you can verify that its variant matches the
configured `gguf_backend`.

ONNX CUDA and MIGraphX variants keep their ONNX engines and provider libraries.
Their packaged provider subdirectories also contain a link to the generic CPU
GGUF worker, so Cohere GGUF can run without replacing the legacy-capable
application. Those variants' bundled workers are CPU-only: to use hardware
Vulkan with Cohere GGUF while retaining ONNX support, set `VOXTYPE_GGUF_WORKER`
to `/usr/lib/voxtype/voxtype-gguf-worker-vulkan` and configure
`[cohere] gguf_backend = "vulkan"`. The Vulkan application itself does not
include the legacy ONNX engines. Parakeet GGUF additionally requires an
application built with `parakeet-gguf` (included in `gpu-vulkan`, not in the
ONNX-only feature lists).

### Cohere or Parakeet changed defaults, but my existing model is still ONNX

This is intentional. GGUF-capable builds recommend transcribe.cpp/GGUF for a
first engine selection, but preserve explicit saved model names and paths.
Check `voxtype config get cohere.model` or `voxtype config get parakeet.model`
and `voxtype info models --json` to distinguish your saved model from the
compiled default. Do not delete a working ONNX/MIGraphX configuration to fix a
missing GGUF worker. Keep the legacy-capable binary and its provider libraries
for ONNX models. To migrate deliberately, download and select a GGUF model,
install the matching `voxtype-gguf-worker`, and turn off Parakeet streaming.
A GGUF-capable main binary alone does not guarantee that worker or GPU support
is installed.

### "Model not found"

**Cause:** Whisper model not downloaded or wrong path.

The daemon also posts a "model missing" or "model failed to load" notification naming the engine and model, for every engine, either when a recording needs the model (on-demand loading) or when the daemon fails to start because of it. Under systemd, the startup notification is posted once, not on every automatic restart.

**Solution:**
```bash
# Download the model
voxtype setup --download

# Download a specific model, for any engine
voxtype setup --download --model parakeet-tdt-0.6b-v3

# Pick an installed model interactively
voxtype setup model
```

Whisper models download from Voxtype's model mirror at `models.voxtype.io`, and
each file is checked against the SHA-256 in the mirror's manifest before it is
saved. A download that does not match is deleted and the command fails, so a
partial or corrupted file never ends up where the daemon looks for it. If the
download fails, check that the machine can reach `https://models.voxtype.io`
(for example `curl -sI https://models.voxtype.io/whisper/base.en/manifest.json`).

To download by hand instead, fetch the file and compare its SHA-256 with the
one listed in the manifest:

```bash
mkdir -p ~/.local/share/voxtype/models
curl -L -o ~/.local/share/voxtype/models/ggml-base.en.bin \
    https://models.voxtype.io/whisper/base.en/ggml-base.en.bin
curl -s https://models.voxtype.io/whisper/base.en/manifest.json   # lists the sha256
sha256sum ~/.local/share/voxtype/models/ggml-base.en.bin
```

### Punctuation/casing stays lowercase or logs a model warning

The optional cleanup classifier is separate from the speech model. Download
it with `voxtype setup punctuation`, then check it with
`voxtype setup punctuation --status`. This stage runs only for Paraformer,
Dolphin, Omnilingual, Parakeet CTC, and SenseVoice with `use_itn = false`;
engines that already punctuate are intentionally unchanged.

If the log says the feature is unavailable, use an ONNX release binary or
build with `--features punctuation-casing`. Unsupported languages and any
inference error fall back to the original transcript, so a cleanup failure
does not lose dictated text.

### OpenVINO: transcription fails with "unknown exception" (but the daemon starts fine)

**Cause:** The model directory is missing `preprocessor_config.json` (mel-spectrogram feature-extraction parameters). This file isn't needed to *load* the model — only the `.xml`/`.bin` graph files are — so the daemon starts and logs "Model loaded, ready for voice input" with no complaint. OpenVINO GenAI's `WhisperPipeline` only reads it on the *first real transcription call*, where its absence surfaces as an opaque `unknown exception` instead of a clear error.

Models downloaded by older Voxtype versions may be missing it because the downloader's file list did not include it. Current builds check for this file at startup and fail with a clear message and fix command rather than letting the model reach this confusing runtime error.

**Solution:**
```bash
# Re-download the model (fetches the missing file along with everything else)
voxtype setup --download --model <model-name>

# Or fetch just the missing file directly (note: NOT identical across model
# sizes -- large-v3/large-v3-turbo use 128 mel bins, everything else uses 80,
# so fetch it from the specific model's own directory, not a different one's)
curl -Lo ~/.local/share/voxtype/models/<model-dir>/preprocessor_config.json \
    https://models.voxtype.io/openvino/<model-dir>/preprocessor_config.json
```

### OpenVINO: setup prints "Compiling ... for Intel NPU" and takes minutes, or ends with a warning

**Cause:** When config.toml has an `[openvino]` section with `device = "NPU"`, `voxtype setup` compiles the model for the NPU right after downloading or activating it, so the one-time compile wait happens during setup instead of during your first recording. Large models genuinely take minutes to compile (about 15 minutes for `large-v3-int4` on Lunar Lake); this is the NPU compiler working, not a hang.

If the compile fails, setup prints a warning and continues. This is not a broken install: the model files are downloaded, the config is updated, and the daemon compiles the model on first use, falling back to GPU or CPU when the NPU is unavailable. The warning usually means the OpenVINO GenAI runtime or the NPU driver is missing, and it includes the package list for the configured device (also documented under `[openvino]` in the configuration guide).

**Solution:** Install the packages named in the warning, verify the NPU device exists (`ls /dev/accel/accel*`), then rerun setup:

```bash
voxtype setup --download --model <model-name>
```

Setup skips the compile when the model's cache blob already exists, so rerunning after a successful compile is cheap.

### ONNX engines run on CPU despite an NVIDIA GPU (cuDNN missing)

**Symptoms:** `voxtype-onnx-cuda` is installed and selected, but transcription is slow, and the journal shows:

```
Failed to load library /usr/lib/voxtype/cuda-13/libonnxruntime_providers_cuda.so with error: libcudnn.so.9: cannot open shared object file
```

**Cause:** ONNX Runtime's CUDA execution provider links against cuDNN 9, which is a separate package from the CUDA toolkit. Without it the provider cannot load and every ONNX engine (Parakeet, Moonshine, SenseVoice and the rest) runs on CPU.

Voxtype reports this at daemon start with a "GPU unavailable, using CPU" notification, in `voxtype info accel` (state `cpu-fallback` with `fix:` lines), and in `voxtype setup gpu --status`.

**Solution:** Install cuDNN 9, then restart the daemon (`systemctl --user restart voxtype`):

```bash
# Arch
sudo pacman -S cudnn

# Debian/Ubuntu/Fedora: from NVIDIA's CUDA repository, matching your variant
sudo apt install libcudnn9-cuda-12   # or libcudnn9-cuda-13 for voxtype-onnx-cuda-13
```

### "whisper-cli not found": dictation records but nothing is typed

**Symptom:** Recording works, but nothing is typed, and the log or a
notification says `whisper-cli not found` (Omarchy #18694).

**Cause:** `[whisper] mode = "cli"` runs a separate `whisper-cli` program
for every transcription, and no voxtype package installs it. Most people
never meant to choose cli mode: it is a workaround for one kind of crash
(below), and `mode = "local"`, the default, transcribes inside voxtype with
the same models, GPU included.

**Solution:** Switch back to local mode, which needs nothing extra:

```bash
voxtype config set whisper.mode local
systemctl --user restart voxtype
```

If you do want cli mode, install whisper.cpp's `whisper-cli` (the
`whisper-cpp` package on Arch and Omarchy, or build it as shown below), or
point `[whisper] whisper_cli_path` at your build.

`voxtype doctor`, `voxtype setup check` and `voxtype config set` all report
cli mode without `whisper-cli`. `voxtype configure` only offers cli mode once
`whisper-cli` is installed, and warns when you save it without one.

### Voxtype crashes during transcription (Linux)

**Cause:** On some Linux systems (particularly with glibc 2.42+ like Ubuntu 25.10), the whisper-rs FFI bindings crash due to C++ exceptions crossing the FFI boundary.

**Solution:** Use the CLI backend which runs whisper-cli as a subprocess:

```toml
[whisper]
backend = "cli"
```

This requires `whisper-cli` to be installed: the `whisper-cpp` package on Arch, or build it from [whisper.cpp](https://github.com/ggerganov/whisper.cpp):

```bash
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp
cmake -B build
cmake --build build --config Release
sudo cp build/bin/whisper-cli /usr/local/bin/
```

See [CLI Backend](USER_MANUAL.md#cli-backend-whisper-cli) in the User Manual for details.

### Poor transcription accuracy

**Possible causes:**

#### 1. Using wrong model
- For English: Use `.en` models (e.g., `base.en`)
- For accuracy: Use larger models (`small.en`, `medium.en`)

#### 2. Audio quality issues
- Use a quality microphone
- Reduce background noise
- Maintain consistent distance from mic

#### 3. Wrong language setting
```toml
[whisper]
model = "base.en"  # For English
language = "en"
```

#### 4. Context window optimization

Context window optimization is disabled by default because it can cause phrase repetition with some models (especially large-v3 and large-v3-turbo).

If you want faster transcription and your model works well with it, you can enable it:

```toml
[whisper]
context_window_optimization = true
```

Or via command line:
```bash
voxtype --whisper-context-optimization daemon
```

If you experience phrase repetition (e.g., "word word word"), make sure this setting is disabled (the default).

### Transcription includes "[BLANK_AUDIO]" or similar

**Cause:** Recording contains mostly silence.

**Solution:**
- Check microphone is working
- Increase microphone sensitivity
- Speak closer to the microphone

### Hallucinations (transcribed text not spoken)

**Cause:** Known Whisper behavior with silence or noise.

Whisper may also occasionally return a punctuation-only transcript, such as a
lone dash (`-`), even when the recording contains speech. Voxtype treats this
as a degenerate decode: it retries the same audio once with a more conservative
decode path, then drops the result if the retry is still punctuation-only.

**Solutions:**
1. Use a larger model for better accuracy
2. Avoid recording ambient noise
3. Keep recordings short and speech-focused

### Phrase repetition (same words repeated multiple times)

**Cause:** Known issue with Whisper large-v3 models, especially when context window optimization is enabled.

**Example:** Saying "increase the limit" produces "increase the limit increase the limit increase the limit"

**Solutions:**
1. Ensure `context_window_optimization` is disabled (the default):
   ```toml
   [whisper]
   context_window_optimization = false
   ```
2. Try a different model (large-v3-turbo and large-v3 are most affected)
3. If using context optimization and experiencing issues, disable it

---

## Voice Activity Detection (VAD)

### VAD filters too aggressively (rejects recordings with speech)

**Symptom:** VAD rejects recordings that contain speech, showing "No speech detected" in notifications or logs.

**Cause:** The detection threshold is too high for your microphone or environment.

**Solution:** Lower the threshold. The default is 0.5. Values closer to 0.0 are more sensitive:

```toml
[vad]
enabled = true
threshold = 0.2  # More sensitive, less likely to reject speech
```

You can also reduce `min_speech_duration_ms` if very short utterances are being rejected:

```toml
[vad]
enabled = true
threshold = 0.3
min_speech_duration_ms = 50  # Accept shorter speech segments (default: 100)
```

### VAD model not found (Whisper VAD backend)

**Symptom:** A "Voxtype: voice activity detection is off" notification when the daemon starts, with `backend = "whisper"`. Silent recordings get transcribed, sometimes as made-up text. With `backend = "auto"` and the Whisper engine, the daemon instead logs `Whisper VAD unavailable, [vad] backend = "auto" falls back to Energy VAD` and keeps filtering silence with Energy VAD.

**Solution:** Download the Silero VAD model:

```bash
voxtype setup vad
```

Alternatively, switch to the energy backend which requires no model download:

```toml
[vad]
enabled = true
backend = "energy"
```

### VAD doesn't filter silence

**Symptom:** VAD is enabled but silent recordings still get transcribed, producing hallucinations.

**Possible causes:**
1. Background noise above the threshold. Lower your microphone gain or raise the threshold:
   ```toml
   [vad]
   enabled = true
   threshold = 0.7  # Require louder speech
   ```
2. The energy backend may be less accurate than the Whisper backend for borderline cases. Try switching:
   ```toml
   [vad]
   enabled = true
   backend = "whisper"  # More accurate, requires: voxtype setup vad
   ```

**Debugging:** Run with verbose logging to see VAD decisions:

```bash
voxtype -vv
```

Look for log messages about speech detection to understand what VAD is doing with your recordings.

---

## Output Problems

### wtype not working on KDE Plasma or GNOME Wayland

**Symptom:** wtype fails with "Compositor does not support the virtual keyboard protocol"

**Cause:** KDE Plasma and GNOME do not implement the `zwp_virtual_keyboard_v1` Wayland protocol that wtype requires. This is a compositor limitation, not a voxtype bug.

**What happens:** Voxtype detects this failure and automatically falls back to dotool, then ydotool. If neither is set up, it falls back to clipboard mode.

**Solution 1 (Recommended):** Install dotool. Unlike ydotool, direct dotool fallback does not require a daemon and supports keyboard layouts for non-US keyboards. eitype needs no device access at all and is the simpler choice on a US layout.

dotool needs write access to `/dev/uinput`. Grant it with a dedicated `uinput` group, not the `input` group, which would also let your programs read every keystroke. The commands are in [Giving dotool and ydotool access to uinput](USER_MANUAL.md#giving-dotool-and-ydotool-access-to-uinput).

```bash
# 1. Install dotool (check your distribution's package manager)
# Arch (AUR):
yay -S dotool
# From source: https://sr.ht/~geb/dotool/

# 2. Give your user write access to /dev/uinput (dedicated group, see link above)

# 3. Configure keyboard layout if needed (non-US keyboards)
# Add to config.toml:
# [output]
# dotool_xkb_layout = "de"  # German, French ("fr"), etc.
```

**Solution 2:** Set up ydotool as your typing backend. Unlike dotool, ydotool requires a daemon to be running:

```bash
# 1. Install ydotool
# Arch:
sudo pacman -S ydotool
# Fedora:
sudo dnf install ydotool
# Ubuntu/Debian:
sudo apt install ydotool

# 2. Enable and start the daemon (required!)
systemctl --user enable --now ydotool

# 3. Verify it's running
systemctl --user status ydotool
```

**Important:** For ydotool, simply having it installed is not enough. The daemon must be running for the fallback to work.

**Alternative:** Use clipboard or paste mode instead of type mode:

```toml
[output]
mode = "clipboard"  # Copies to clipboard, you paste manually
# or
mode = "paste"      # Copies to clipboard, then simulates Ctrl+V
```

**Compositor compatibility:**

| Desktop | wtype | dotool | ydotool | Recommended |
|---------|-------|--------|---------|-------------|
| Hyprland | ✓ | ✓ | ✓ | wtype |
| Sway | ✓ | ✓ | ✓ | wtype |
| River | ✓ | ✓ | ✓ | wtype |
| KDE Plasma (Wayland) | ✗ | ✓ | ✓ | dotool |
| GNOME (Wayland) | ✗ | ✓ | ✓ | dotool |
| X11 (any desktop) | ✗ | ✓ | ✓ | dotool |

---

### Text output not working on X11

**Symptom:** You're running X11 (not Wayland) and see errors like:
```
WARN  wtype failed: Wayland connection failed
WARN  clipboard (wl-copy) failed: Text injection failed
ERROR Output failed: All output methods failed.
```

**Cause:** wtype and wl-copy are Wayland-only tools. On X11, voxtype needs dotool, ydotool, or xclip installed.

**Solution:** Install one of these X11-compatible tools:

**Option 1 (Recommended): Install dotool**

direct dotool works on X11, supports keyboard layouts, and doesn't need a daemon:

```bash
# Ubuntu/Debian (from source):
sudo apt install libxkbcommon-dev
git clone https://git.sr.ht/~geb/dotool
cd dotool && ./build.sh && sudo cp dotool /usr/local/bin/

# Arch (AUR):
yay -S dotool
```

Then give your user write access to `/dev/uinput` through a dedicated group, not the `input` group: see [Giving dotool and ydotool access to uinput](USER_MANUAL.md#giving-dotool-and-ydotool-access-to-uinput).

**Option 2: Install ydotool**

ydotool works on X11 but requires a running daemon:

```bash
# Ubuntu/Debian:
sudo apt install ydotool

# Start the daemon (see "ydotool daemon not running" section for Fedora)
systemctl --user enable --now ydotool
```

**Option 3: Use clipboard mode with xclip**

For clipboard-only output (you paste manually with Ctrl+V):

```bash
# Ubuntu/Debian:
sudo apt install xclip
```

Then configure voxtype to use clipboard mode:
```toml
[output]
mode = "clipboard"
```

**Verify your setup:**

```bash
voxtype setup
```

This shows which output tools are installed and available.

---

### Wrong characters on non-US keyboard layouts (y/z swapped, QWERTZ, AZERTY)

**Symptom:** Transcribed text has wrong characters. For example, on a German keyboard, "Python" becomes "Pzthon" and "zebra" becomes "yebra" (y and z are swapped).

**Cause:** ydotool sends raw US keycodes and doesn't support keyboard layouts. When voxtype falls back to ydotool (e.g., on X11, Cinnamon, or when wtype fails), characters are typed as if you had a US keyboard layout.

**Solution:** Install dotool and configure your keyboard layout. Unlike
ydotool, direct dotool fallback supports keyboard layouts via XKB:

```bash
# 1. Install dotool
# Arch (AUR):
yay -S dotool
# Ubuntu/Debian (from source):
# See https://sr.ht/~geb/dotool/ for instructions
# Fedora (from source):
# See https://sr.ht/~geb/dotool/ for instructions

# 2. Give your user write access to /dev/uinput through a dedicated
#    group, not the input group (which would also expose every keystroke):
#    see USER_MANUAL.md, "Giving dotool and ydotool access to uinput"

# 3. Configure your keyboard layout in config.toml:
```

Add to `~/.config/voxtype/config.toml`:

```toml
[output]
dotool_xkb_layout = "de"  # German QWERTZ
```

Then switch your desktop/compositor keyboard layout to the same layout before
dictating. dotool sends key events; it does not switch the active layout for
the focused app.

Common layout codes:
- `de` - German (QWERTZ)
- `fr` - French (AZERTY)
- `es` - Spanish
- `uk` - Ukrainian
- `ru` - Russian
- `pl` - Polish
- `it` - Italian
- `pt` - Portuguese

For layout variants (e.g., German without dead keys):

```toml
[output]
dotool_xkb_layout = "de"
dotool_xkb_variant = "nodeadkeys"
```

The active desktop layout must use the same variant.

For multilingual dictation, use per-language mappings so the variant only
applies to the language that needs it. For example, Russian phonetic typing:

```toml
[whisper]
language = ["en", "ru"]

[output.language_to_layout]
en = "us"
ru = "ru"

[output.language_to_variant]
ru = "phonetic"
```

Before dictating Russian through direct dotool fallback, switch the active
desktop layout to Russian phonetic. If the active layout is still English,
dotool will send the right key positions for Russian phonetic, but the focused
app will receive English letters such as `Probuem goworitx po-russki`.

**Alternative:** Use paste mode, which copies text to the clipboard and simulates Ctrl+V. This works regardless of keyboard layout:

```toml
[output]
mode = "paste"
```

**Note:** The keyboard layout fix requires voxtype v0.5.0 or later. If you're on an older version, upgrade first.

---

### Wrong characters when transcribing a second language

**Symptom:** With `language = ["en", "ru"]`, transcribing Russian on a US
system layout fails with eitype reporting `Character not found in keymap`, or
dotool/eitype prints garbled text. English transcriptions work fine.

**Cause:** Before voxtype v0.7.3, the daemon did not pass a layout hint to
the `eitype` binary. eitype fell back to the system's active XKB layout,
which lacked the keycodes for Cyrillic (or any non-system language) and so
either errored or typed the wrong characters. This is issue #180.

**Solution:** Upgrade to a version with per-language XKB hints. The daemon
reads the language Whisper picked for each transcription and passes a matching
layout/variant hint to eitype or direct dotool fallback for that call.

`dotoolc` does not work with variants and cannot receive voxtype's per-call XKB
hints, so voxtype uses direct `dotool` for these hinted calls. This hint only
controls dotool's text-to-key lookup. You must switch the active desktop layout
to the target language/variant before dictating. If you dictate Russian while
the active layout is English, the result can look transliterated
(`Probuem goworitx po-russki`) even though dotool sent the intended key
positions.

Verify:

```bash
voxtype -v daemon  # debug logs
# After a Russian transcription you should see:
# DEBUG Auto layout for eitype: language='ru' -> layout='ru'
# DEBUG Auto variant for eitype: language='ru' -> variant='phonetic'
```

**Forcing a specific layout.** To pin eitype to a fixed layout regardless of
the detected language, set it explicitly:

```toml
[output]
driver_order = ["eitype"]
eitype_xkb_layout = "us"          # or "de", "ru", etc.
# eitype_xkb_variant = "dvorak"   # optional
```

**Customizing the language-to-layout and variant maps.** Voxtype ships built-in
layout defaults (`en->us`, `ru->ru`, `de->de`, etc.). Layouts that don't match
the language code (e.g. Brazilian Portuguese uses `br`, not `pt`) need an
override in config. Variants are empty by default because they are user-specific:

```toml
[output.language_to_layout]
en = "us"
pt = "br"
ru = "ru"

[output.language_to_variant]
ru = "phonetic"
```

See `docs/CONFIGURATION.md` for the full list of built-in defaults and merge
semantics (the user table replaces the defaults, so copy the entries you
want to keep).

---

### "ydotool daemon not running"

**Cause:** ydotool systemd service not started.

**Solution:**
```bash
# Enable and start ydotool
systemctl --user enable --now ydotool

# Verify it's running
systemctl --user status ydotool

# Check for errors
journalctl --user -u ydotool
```

### "Copied to clipboard, paste manually" notification

**Symptom:** Instead of typing, voxtype shows a notification titled "Voxtype: copied to clipboard, paste manually" (the first time) or "Copied to clipboard, paste manually" (later dictations).

**Cause:** None of the typing tools in the output chain (wtype, eitype, dotool, ydotool) could be used, so voxtype put the text on the clipboard instead. The first notification after the daemon starts lists each tool and why it was skipped, with the fix, for example:

```
Voxtype could not type the text, so it is on the clipboard.
wtype: not installed. Fix: sudo pacman -S wtype
eitype: not installed. Fix: cargo install eitype
dotool: not installed. Fix: install it from https://sr.ht/~geb/dotool/
ydotool: ydotoold is not running. Fix: systemctl --user enable --now ydotool
```

**Solution:** Apply the fix for one of the listed tools, then try again. Paste the current text with Ctrl+V (Ctrl+Shift+V in most terminals). The warning is shown once per problem per daemon run; the full reason is also in `journalctl --user -u voxtype` (or `journalctl _SYSTEMD_USER_UNIT=voxtype.service` when [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)).

If you set `mode = "clipboard"` or a `driver_order` with only clipboard drivers, voxtype does not warn: the clipboard is what you asked for.

### "Transcription not delivered" notification

**Symptom:** A critical notification titled "Voxtype: transcription not delivered".

**Cause:** Every output method failed, including the clipboard (or the clipboard fallback is off). The notification lists what failed and how to fix it, and includes the text.

**Solution:** Nothing is lost. Each undelivered transcription is saved as its own file in `~/.local/state/voxtype/undelivered/` (or `$XDG_STATE_HOME/voxtype/undelivered/`), named by date and time, readable only by you, and kept across logouts and reboots. The newest 200 are kept. The notification names the exact file. To see the latest one:

```bash
ls -t ~/.local/state/voxtype/undelivered/ | head
cat "$(ls -t ~/.local/state/voxtype/undelivered/*.txt | head -1)"
```

Then install the tool the notification names.

To stop Voxtype keeping undelivered transcriptions:

```bash
voxtype config set output.keep_undelivered_text false
systemctl --user restart voxtype
```

With it off no file is saved, and the notification no longer names a file or shows the text (notification daemons keep a log of what they display), so a dictation that can't be delivered is lost. The notification still lists what failed and how to fix it, and says the text was not kept. Files saved before you turned it off stay until you delete them: `rm -r ~/.local/state/voxtype/undelivered/`.

### Text not typed / nothing happens

**Possible causes:**

#### 1. ydotool not working
```bash
# Test ydotool directly
ydotool type "test"
```

#### Clipboard helper exits before accepting text

If `wl-copy` or another output helper exits before reading its input, the daemon
reports the broken pipe as an output error and tries the configured fallbacks.
It no longer exits silently from SIGPIPE. Check the helper's installation and
session environment if output still fails. This does not change the configured
output order or make an unavailable clipboard service work.

#### 2. Fallback to clipboard not working
```bash
# Test wl-copy
echo "test" | wl-copy
wl-paste
```

#### 3. Application blocking input
Some applications (terminals, games) may block simulated input.

**Solution:** Use clipboard mode:
```toml
[output]
mode = "clipboard"
```

### First CJK character dropped (wtype)

**Symptom:** When using wtype, the first Chinese, Japanese, or Korean character is missing from the output. This happens in some apps like Discord.

**Cause:** Some Wayland applications swallow the first character from wtype's virtual keyboard input, particularly with CJK text.

**Solution:** Enable the Shift prefix workaround:

```toml
[output]
wtype_shift_prefix = true
```

This prefixes each wtype command with a Shift key press and release (`-P Shift_L -p Shift_L`), which prevents the first character from being dropped. It only affects the wtype driver and has no visible effect on the output text.

You can also enable it via CLI flag (`--wtype-shift-prefix`) or environment variable (`VOXTYPE_WTYPE_SHIFT_PREFIX=true`).

### Characters dropped or garbled

**Cause:** Typing too fast for the application.

**Solution:** Increase typing delay:
```toml
[output]
type_delay_ms = 10  # Try 10-50ms
```

### Non-ASCII characters move to the front of the text (GNOME, type mode)

**Symptom:** Dictating text with non-ASCII characters (umlauts, accents, ß)
into GTK applications (GNOME Terminal, GNOME Text Editor) delivers them
clustered at the start of the output: `Müll äöüß` arrives as `üäöüß Mll `.
The log shows the correct transcription once
[debug logging](#enable-verbose-logging) is on (the text is not logged by
default), and Chrome or other non-GTK apps receive the same text correctly. Short
dictations often arrive intact, longer ones reliably fail.

**Cause:** IBus, GNOME's default input method, reorders non-ASCII key
events relative to ASCII when synthetic input arrives faster than human
typing. The events leave the typing tool (eitype) in the correct order,
so this is neither a voxtype nor an eitype bug. Reported upstream as
ibus/ibus#2934; details and measurements in
Adam-D-Lewis/eitype#21.

**Solution:** Use paste mode, which transfers the text atomically through
the clipboard and cannot be reordered:

```toml
[output]
mode = "paste"
paste_keys = "ctrl+shift+v"  # terminal convention; GUI apps expect ctrl+v
```

Partial alternatives: `type_delay_ms` shrinks the displacement but does
not remove it (about 100 ms per key would be needed), and launching the
receiving app with `GTK_IM_MODULE=simple` avoids the bug at the cost of
disabling IBus features for that app.

### Clipboard not working

**Cause:** wl-copy not installed or Wayland session issue.

**Solution:**
```bash
# Install wl-clipboard
# Arch: sudo pacman -S wl-clipboard
# Debian: sudo apt install wl-clipboard
# Fedora: sudo dnf install wl-clipboard

# Test it works
echo "test" | wl-copy
wl-paste
```

### Ctrl+V pastes an image in Codex, or a ^V in a terminal

**Symptom:** In paste mode, dictation into a terminal never arrives. The
Codex CLI reports that it found no image on the clipboard, a shell prints
nothing or a literal `^V`, or tmux waits for the rest of a key sequence.

**Cause:** Paste mode sent Ctrl+V. Terminals don't treat Ctrl+V as paste:
they pass it to the running program, and Codex uses it to paste an image,
readline to insert the next key literally.

**Solution:** Let voxtype pick the keystroke per window:

```toml
[output]
mode = "paste"
paste_keys = "auto"   # or remove paste_keys entirely
```

On Hyprland, Sway and niri, `auto` sends `paste_keys_terminal` (default
Ctrl+Shift+V) when a terminal has focus and Ctrl+V elsewhere. If a
terminal still gets Ctrl+V:

1. Check that the terminal is detected. On Hyprland run
   `hyprctl activewindow -j` with the terminal focused: it needs a
   `terminal` (or `terminal*`) tag, or a class voxtype knows. For an
   unknown terminal, add a Hyprland window rule that gives its class the
   `terminal` tag (Omarchy's rule is in
   `~/.local/share/omarchy/default/hypr/apps/terminals.lua`).
2. Check which keystroke your terminal pastes the clipboard with. If it
   isn't Ctrl+Shift+V, set it, for example
   `paste_keys_terminal = "shift+insert"` for Omarchy's terminals.
3. Run `voxtype -vv` and look for `Focused window terminal check` and
   `Text pasted via clipboard + ...` in the log.

On desktops without detection (GNOME, KDE, X11), pick one keystroke that
works in both places, such as `paste_keys = "shift+insert"` if your
terminals map it to the clipboard, or switch to type mode.

### Paste arrives as Escape, digits or an app shortcut (XWayland and Electron apps)

**Symptom:** In paste mode, an XWayland or Electron app (Codex, Slack, some
Chromium builds) does something other than paste: a confirmation dialog
opens, the field loses focus, or digits appear. With
`paste_keys = "shift+insert"`, Codex opens its "Mark all as read" dialog,
which is bound to Shift+Escape.

**Cause:** wtype sends keys through its own temporary keymap and puts the
first one on keycode 1. Apps that read raw keycodes, as XWayland apps do,
see keycode 1 as Escape, so Shift+Insert arrives as Shift+Escape (#938).

**Solution on Hyprland:** send the paste keystroke through Hyprland's
dispatcher, which uses the real keymap:

```toml
[output]
mode = "paste"
driver_order = ["hyprland", "wtype", "eitype", "ydotool"]
```

Run `voxtype -vv` and look for `Paste keystroke sent via hyprland`. If it
says `hyprctl dispatch failed`, check that `hyprctl version` works in the
same session voxtype runs in (the systemd user service needs
`HYPRLAND_INSTANCE_SIGNATURE` in its environment, which Hyprland and UWSM
sessions import).

**Elsewhere:** put `ydotool` first (`driver_order = ["ydotool", "wtype"]`).
It sends real evdev keycodes, but needs the `ydotoold` daemon and uinput
access.

### Clipboard not restored after paste

**Cause:** The restore delay may be too short for your application, or `wl-paste` is not installed.

**Solution:**

1. Make sure `wl-clipboard` is installed (provides both `wl-copy` and `wl-paste`):
```bash
# Arch: sudo pacman -S wl-clipboard
# Debian: sudo apt install wl-clipboard
# Fedora: sudo dnf install wl-clipboard
```

2. If the clipboard is restored before the application reads it, increase the delay:
```toml
[output]
restore_clipboard_delay_ms = 500  # Try 300-500ms for slow applications
```

3. On X11, make sure `xclip` is installed for clipboard restoration support.

4. If you copied something after the paste and before the restore, the restore is skipped on purpose so your newer copy is kept.

5. If the log says "Could not save the clipboard", the clipboard could not be read before the paste, usually because the app that owns it stopped responding (each read gives up after 2 seconds). The dictation is still pasted, and the clipboard keeps it. Copy the original again from its source.

### Formatted text comes back as plain text after a paste

With `restore_clipboard = true`, only one representation of the clipboard can be put back, and voxtype keeps the plain-text one. If you copied formatted text (HTML from a browser, for example), pasting it after a dictation gives the plain text. Copy it again from the source to get the formatting back.

### "Post-processing failed", "timed out", "command not found", "returned nothing" or "edit list rejected"

**Cause:** The `[output.post_process]` command (or a profile's `post_process_command`) didn't produce cleaned text, so Voxtype typed the original transcription instead. The notification says which case it was.

**Solution:**
- **Command not found:** install the program, or use its full path in `command`. A daemon started by systemd may not have the same `PATH` as your shell.
- **Exited with code N:** run the command by hand with a sample on stdin, e.g. `echo "hello world" | ollama run llama3.2:1b 'Clean up:'`, and fix what its error says (often the model isn't pulled or the server isn't running).
- **Timed out:** raise the timeout, e.g. `voxtype config set output.post_process.timeout_ms 60000` (for a profile, `post_process_timeout_ms` in `[profiles.<name>]`), or use a smaller model. The first call to a local LLM can be slow while the model loads.
- **Returned nothing:** make the command print the cleaned text on stdout. If empty output is intended (to drop `[BLANK_AUDIO]`, say), set `fallback_on_empty = false`.
- **Edit list rejected:** with `edit_list = true`, the command must print only a JSON array of `find`/`replace`/`reason` edits whose targets occur once in the transcription. The notification gives the reason the list was rejected; fix the prompt or script, or set `edit_list = false` in `[output.post_process]`.

The same failure is only notified once every 5 minutes; run `journalctl --user -u voxtype -f` to see every occurrence (or `journalctl _SYSTEMD_USER_UNIT=voxtype.service -f` when [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)).

Meeting mode never shows these notifications, so a failing command can't pop up over a call. During a meeting the failure is only logged, and the transcript keeps the original text.

### "Text cleanup by the model failed" or "text cleanup timed out"

**Cause:** `[output.post_process] backend` is `"omarchy"` or `"openai"`, and the model did not answer in time or at all. Voxtype typed the original transcription.

**Solution:**
- **No Omarchy text model is running:** voxtype never starts one. Start it with `omarchy models start <recipe>` or from the local models panel, or set `backend = "command"` to turn model cleanup off. `voxtype doctor` shows whether Omarchy, a running model, its key and its endpoint were found.
- **Timed out:** a chat model adds roughly 2 to 6 seconds per dictation, more for a long one or a cold start. Raise `timeout_ms` (`voxtype config set output.post_process.timeout_ms 15000`) or pin a smaller model with `model`.
- **Refused the API key:** for `"openai"`, check the file in `api_key_file`. For Omarchy, restart the model from the local models panel.
- **Every dictation is slow:** that is the cost of the model backends. LARD (`voxtype setup lard`, `[text] lard_disfluency = true`) removes fillers in milliseconds without a model.

### "pre_output hook failed" or "post_output hook failed"

**Cause:** `pre_output_command` or `post_output_command` exited with an error. The text was still delivered, but whatever the hook does (switching a compositor submap, for example) didn't happen.

**Solution:** run the command from the notification in a terminal and fix it, or correct it in the `[output]` section of `config.toml`.

### No desktop notification

**Cause:** No notification server is running, the session bus is unreachable from the daemon, or notifications are disabled in `[output.notification]`.

Voxtype sends notifications with `notify-send` (libnotify) and, if that is missing or fails, talks to the notification server directly over D-Bus (`org.freedesktop.Notifications`). When neither works, the daemon logs this warning once:

```
Desktop notifications cannot be delivered, so Voxtype alerts will only appear in this log (...)
```

The part in parentheses says what each route reported. `ServiceUnknown` means no notification server is running; a "No such file or directory" error means the daemon cannot reach the session bus.

**Solution:**
```bash
# Check whether a notification server is running
busctl --user call org.freedesktop.Notifications /org/freedesktop/Notifications \
    org.freedesktop.Notifications GetServerInformation

# If not, install and start one (GNOME and KDE ship their own), e.g.
# Arch: sudo pacman -S mako   (or swaync, dunst)

# Optional: install notify-send for testing
# Arch: sudo pacman -S libnotify
# Debian: sudo apt install libnotify-bin
# Fedora: sudo dnf install libnotify

# Test
notify-send "Test" "This is a test"
```

If the test works in your terminal but the daemon still logs the warning, the systemd user service may not see your session bus. Run `systemctl --user import-environment DBUS_SESSION_BUS_ADDRESS` from your compositor's startup and restart voxtype.

### Edit-list post-processing keeps the original text

**Symptoms:** `output.post_process.edit_list = true` is enabled, but Voxtype
types the original transcription. Verbose logs report an invalid edit list.

**Cause:** The command did not return the strict edit-list contract. Common
causes are a prose preamble, a Markdown code fence, a missing `reason`, a
`find` string that is absent or occurs more than once, or overlapping edits.

**Solution:** Run the configured command by hand with a sample transcription
on stdin. Its entire stdout must be one JSON array:

```json
[
  {"find":"It are ready","replace":"It is ready","reason":"grammar"}
]
```

Use `voxtype -vv` to see why Voxtype rejected the response. Tighten the LLM
prompt or script so it prints no text outside the JSON array. If the same target
appears more than once, have the command return a larger, unique `find` string.

---

## Performance Issues

### Slow transcription

**Solutions:**

1. **Use a smaller model:**
```toml
[whisper]
model = "tiny.en"  # Fastest
```

2. **Increase thread count:**
```toml
[whisper]
threads = 8  # Match your CPU cores
```

3. **Use English-only model:**
`.en` models are faster than multilingual models.

### "Whisper is running on CPU" notification (GPU build)

**Cause:** The daemon is running a GPU build of Whisper (the Vulkan binary, or a CUDA/ROCm/Metal source build), but whisper.cpp found no usable GPU when it loaded the model, so it is transcribing on CPU. whisper.cpp itself only logs `no GPU found` and carries on; Voxtype notifies once per daemon start so the slowdown isn't a mystery. Transcription keeps working, just at CPU speed.

The usual reason on the Vulkan build is a missing Vulkan driver (ICD): the Vulkan loader is installed, but no driver for your GPU is.

**Solutions:**

1. Install the Vulkan driver for your GPU and check that it lists a device:
```bash
# Arch: AMD / Intel / NVIDIA
sudo pacman -S vulkan-radeon      # or vulkan-intel, or nvidia-utils
# Debian/Ubuntu/Fedora (AMD/Intel): mesa-vulkan-drivers
vulkaninfo --summary
systemctl --user restart voxtype
```

2. If the notification says `gpu_device = N but only M GPU(s) were found`, `[whisper] gpu_device` points past the end of the device list. Devices are numbered from 0; pick one from `vulkaninfo --summary` or remove the setting.

3. Check what the running daemon actually got:
```bash
voxtype info accel
```

### `setup gpu --enable` says Vulkan is not available on this machine

**Cause:** `voxtype setup gpu --enable` checks that the Vulkan build can run before switching to it, because a build that cannot run crashes on every start and systemd restarts it in a loop. It refuses in two cases:

- **The CPU has no AVX2.** The packaged `voxtype-vulkan` is compiled for Haswell-class CPUs and later. On an older CPU it would stop with an illegal instruction.
- **The GPU has no 16-bit storage.** Whisper's Vulkan backend requires 16-bit storage buffers. Intel Haswell graphics on Mesa's `hasvk` driver (Iris Pro 5200, HD 4600 and similar) do not have them, and the Vulkan build aborts when it loads a model. voxtype reads this from `vulkaninfo` (package `vulkan-tools`). Without `vulkaninfo` it recognises Haswell graphics by PCI ID and otherwise allows the switch.

`voxtype setup gpu --status` shows the same verdict next to the Vulkan backend.

**Solution:** Stay on the CPU build. On a CPU without AVX2, use the baseline build:

```bash
sudo voxtype setup variant --to baseline
systemctl --user restart voxtype
```

If an earlier version already switched you to Vulkan and the daemon is crash-looping, switch back:

```bash
sudo voxtype setup gpu --disable
systemctl --user restart voxtype
```

A machine with a second GPU that does support 16-bit storage is not refused: Whisper's Vulkan backend skips the unsupported GPU and uses the other one.

### "OpenVINO fell back to GPU/CPU" notification

**Cause:** `[openvino] device` names a device (usually `NPU`) that failed to initialize, so Voxtype fell back to the next device in the chain (GPU, then CPU). The notification names the packages for the configured device.

**Solution:** Install the packages named in the notification (for NPU: `openvino-intel-npu-plugin intel-npu-driver`, then reboot), check the device with `voxtype setup npu --status`, and restart the daemon. `voxtype info accel` shows which device the running daemon is using.

### Slow transcription on multi-GPU systems (Vulkan)

**Cause:** whisper.cpp 1.7+ enumerates integrated GPUs via Vulkan. On systems with both an integrated GPU (e.g., Intel UHD) and a discrete GPU (e.g., NVIDIA RTX), the integrated GPU gets index 0 and becomes the default. This can cause ~3x slower transcription.

**Solutions:**

1. **Use VOXTYPE_VULKAN_DEVICE** (recommended for different-vendor GPUs):

If your integrated and discrete GPUs are from different vendors (e.g., Intel iGPU + NVIDIA dGPU), this is the simplest fix. It filters out the unwanted vendor's Vulkan driver entirely.

```bash
# In your environment or systemd override:
VOXTYPE_VULKAN_DEVICE=nvidia
```

Valid values: `nvidia`, `amd`, `intel`. See `voxtype setup gpu` for detected GPUs.

2. **Set gpu_device in config** (for same-vendor GPUs or precise index control):

This passes a device index directly to whisper.cpp, useful when both GPUs are from the same vendor.

```toml
[whisper]
gpu_device = 1  # Use discrete GPU instead of integrated at index 0
```

3. **Or use the GGML_VK_VISIBLE_DEVICES env var:**
```bash
GGML_VK_VISIBLE_DEVICES=1 voxtype
```

**How to find the right GPU:** Run `voxtype setup gpu` to see detected GPUs, or `vulkaninfo --summary` to see the Vulkan device list and their indices.

### High CPU usage

**Cause:** Whisper inference is CPU-intensive.

**Solutions:**
1. Limit threads:
```toml
[whisper]
threads = 4  # Limit CPU usage
```

2. Use smaller model:
```toml
[whisper]
model = "tiny.en"
```

### High memory usage

**Cause:** Large Whisper models require significant RAM.

| Model | Approximate RAM |
|-------|-----------------|
| tiny.en | ~400 MB |
| base.en | ~500 MB |
| small.en | ~1 GB |
| medium.en | ~2.5 GB |
| large-v3 | ~4 GB |

**Solution:** Use a smaller model if RAM is limited.

### Hotkey lag / delayed recording start

**Cause:** System load or evdev latency.

**Solutions:**
1. Ensure voxtype is running with normal priority
2. Check for other applications using evdev
3. Try a different hotkey

---

## ElevenLabs Backend Issues

### "Streaming output stopped" or partial revisions do not rewind

ElevenLabs changed a provisional tail, but Voxtype could not send the required
BackSpace keys or could not type the replacement. Voxtype stops the stream: it
ends the session, leaves visible text in place, shows one "Streaming Error"
notification saying streaming output stopped, and returns to idle. It does not
rewind text after cursor and provider state may have diverged.

Install and configure at least one BackSpace-capable driver:

- `wtype` on a compositor that supports the virtual keyboard protocol
- `dotool`, or `dotoold` with `dotoolc` for lower streaming latency
- `ydotool` with its daemon running

`eitype` and clipboard-only output do not provide the streaming BackSpace
fallback. If you cannot use a supported driver, set:

```toml
[elevenlabs]
mode = "realtime"
```

Committed-only realtime mode avoids provisional cursor edits.

### Punctuation remains stale after a pause

With `commit_strategy = "vad"`, a pause can commit the preceding segment.
ElevenLabs does not send authoritative revisions for an already committed
segment, so later speech cannot join or repunctuate it.

Keep the segment open across ordinary pauses:

```toml
[elevenlabs]
mode = "partials"
commit_strategy = "manual"
```

A later partial may revise the open tail, but the model does not guarantee a
specific punctuation choice. Provider automatic commits during long sessions
still create permanent segment boundaries.

### Manual realtime mode types nothing until recording stops

This is expected with `mode = "realtime"` and
`commit_strategy = "manual"`. Realtime mode ignores partial snapshots, and
manual mode waits for Voxtype's explicit commit on stop. ElevenLabs may also
commit automatically after about 36 seconds.

Use `mode = "partials"` for live text, or use `commit_strategy = "vad"` for
committed output after silence.

### Provisional text changes while speaking

`mode = "partials"` displays provider snapshots before they are final. Words
or punctuation may appear, retract, and change as ElevenLabs receives more
audio. Keep focus in the target application and leave the caret at the end of
the streamed text. Voxtype cannot recover if you move the caret or switch
focus during a correction.

Use `mode = "realtime"` when you prefer stable committed segments without
visible partial churn.

---

## Soniox Backend Issues

### "Soniox API key required: set [soniox] api_key or SONIOX_API_KEY"

The backend can't find a credential. Either set the env var:

```bash
export SONIOX_API_KEY="your-key-here"
```

…or add it to `~/.config/voxtype/config.toml`:

```toml
[soniox]
api_key = "your-key-here"   # less safe — lands in dotfiles
```

The env var is preferred (no key in shell history, no key in config backups).

### "Soniox: WS connect failed: ..." or "connect timeout"

Network or DNS issue reaching the Soniox realtime host (`wss://stt-rt.soniox.com`, or `stt-rt.<region>.soniox.com` when `[soniox] region` is set). Check:
- Internet connectivity (`curl https://api.soniox.com`)
- Firewall / corporate proxy blocking outbound 443
- VPN that mangles WebSocket handshakes

Voxtype retains captured audio and recovers it through the configured batch backend when streaming fails before delivery. It notifies you with **Dictation Recovered**. To disable this fallback or change its stop deadline:

```toml
[streaming_recovery]
enabled = false
stop_timeout_secs = 30
```

### 401 Unauthorized / 403 Forbidden

API key is invalid, revoked, or out of credit. Check the dashboard at https://console.soniox.com.

A key that works in the Soniox console but gets `401` from voxtype usually belongs to a project in another region. Soniox projects are pinned to one region (US, EU, Japan or India) and their keys are refused by every other region's hosts. Set the region to match the project:

```toml
[soniox]
region = "eu"   # or "jp", "in"; "us" is the default
```

Voxtype 1.3.2 and later send the key in the WebSocket `Authorization` header. Older versions send it in the session's first message, which Soniox stops accepting on 15 January 2027; upgrade if Soniox starts refusing a key that used to work.

### "Unknown Soniox region '...'"

`[soniox] region` (or `--soniox-region`, or `VOXTYPE_SONIOX_REGION`) must be one of `us`, `eu`, `jp` or `in`. Use the region the Soniox project that issued your key was created in.

### Soniox typed text occasionally diverges from spoken words (realtime mode)

Soniox occasionally revises tail tokens between non-final and final states (`tévedések,` → `tévedések.`, `fejeztem` → `fejezte`). Voxtype emits a `StreamingEvent::Replace { backspace, text }` in this case so the cursor is patched up — but the patch only works if a backspace-capable driver is in the chain. The current backspace path tries `wtype`, then `dotool` (via `dotoolc` if the daemon is running), then `ydotool`. `eitype` does not have a backspace implementation.

If you see persistent duplication or wrong tails:
1. Turn on [debug logging](#enable-verbose-logging) and check the log for `Soniox tail revision: backspace N chars, type … (lcp=N)` lines. If you see them, Replace is firing. These lines quote the revised text, so they are not logged by default.
2. If you also see `Streaming replace: no backspace-capable backend available; skipping backspace and accepting cursor artifact`, none of wtype/dotool/ydotool was usable — the original tail stayed at the cursor and the corrected text appended. Install at least one of them (`pacman -S wtype` or `pacman -S dotool` on Arch).
3. Disable partial typing entirely: `[soniox] type_partials = false`. Finals are still typed, but no live cursor feedback. Trade-off: feels slower, zero divergence risk.

### Notifications spam during dictation (transient tray icon flicker on KDE)

If your KDE Plasma panel briefly shows an icon and re-layouts every ~150ms during streaming, the cause is usually the `eitype` driver. eitype connects via the XDG RemoteDesktop portal on each call, and KDE's security indicator briefly registers in the system tray.

**Fix:** prefer `dotool` (or `ydotool`, layout-permitting) ahead of `eitype` in `[output] driver_order`. dotool uses kernel uinput directly — no portal, no tray.

```toml
[output]
driver_order = ["dotool", "ydotool", "eitype", "clipboard"]
```

### Streaming is unusably slow (each typed segment takes ~1 second)

You're hitting dotool's uinput init cost (~700ms) on every output call. With 60+ partials per session this stacks into 40+ seconds.

**Fix:** run `dotoold` once at login. When there is no per-call XKB hint,
voxtype auto-detects its FIFO and routes through `dotoolc`, paying the init
cost once for the daemon's lifetime instead of per call. Sub-10ms per typed
segment.

See [Streaming performance: dotoold fast path](CONFIGURATION.md#streaming-performance-dotoold-fast-path) in CONFIGURATION.md for the systemd user unit template.

To verify the fast path is active after dictation:
```bash
journalctl --user -u voxtype --since "5 min ago" | grep "typed via"
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "5 min ago" | grep "typed via"
```
- `Text typed via dotoolc (N chars)` — fast path
- `Text typed via dotool (N chars)` — direct path; daemon not running or voxtype had a per-call XKB hint

### Wrong keyboard layout when dotoold is running

dotool's layout setting applies to **the daemon, not the client**. When voxtype
has no per-call XKB hint, commands routed through `dotoolc` use whatever
layout dotoold inherited from its own environment.

**Fix for one fixed layout:** set `DOTOOL_XKB_LAYOUT` in dotoold's startup
environment and leave voxtype's dotool XKB fields unset:

```bash
# In your systemd user unit:
Environment=DOTOOL_XKB_LAYOUT=hu

# Then:
systemctl --user daemon-reload && systemctl --user restart dotoold
```

For per-language layouts or variants, configure `language_to_layout` /
`language_to_variant` in voxtype. `dotoolc` does not work with variants and
cannot receive voxtype's per-call XKB hints, so voxtype will use direct
`dotool` instead of `dotoolc` for those calls. You still need to switch the
active desktop layout to the same language/variant before dictating.

### PTT auto-promoted to toggle every time you start the daemon

Expected when `[soniox] streaming = true` (the default for the realtime backend). Live cursor typing while the PTT key is still held breaks libinput's held-key state tracking on Hyprland/Sway/River. Voxtype auto-promotes to toggle for the running session and warns.

To use Soniox with **real** push-to-talk, choose one of:
- `[soniox] streaming = false` — one-shot WebSocket on key release, no live partials
- `[soniox] async_api = true` — async REST API, slower but higher accuracy
- `[hotkey] mode = "toggle"` — accept toggle activation (silences the warning)

### Async API job stuck or "Soniox async: job ... did not complete within Ns"

The async API processing took longer than `async_max_wait_secs` (default 120). For very long recordings or during Soniox capacity spikes, bump the timeout:

```toml
[soniox]
async_api = true
async_max_wait_secs = 300
```

### Post-stop "Streaming Error: Soniox server error (408): Request timeout"

This notification used to appear when you released the hotkey and Soniox's server-side timer fired before the connection fully closed. Voxtype now suppresses 408s that arrive **after** you've signalled end-of-audio, so this should be silent. If you still see it, your build predates the fix (any release v0.7.5 or later includes it).

---

## Media Does Not Pause While Recording (Omarchy Quattro)

**Symptom:** `pause_media = true` is set, but music keeps playing while you
dictate. Common on Omarchy Quattro, whose upgrade removes `playerctl` while the
shipped Voxtype config still enables media pausing.

**Cause:** Voxtype v0.7.5 and earlier paused players by shelling out to
`playerctl`. When that binary is absent the pause silently does nothing.

**Fix:** Upgrade to Voxtype v1.0.0 or newer. Media pausing now speaks MPRIS over
D-Bus directly and needs no external binary, so it works on a playerctl-free
system.

If you must stay on v0.7.5, either install `playerctl` or turn the feature off:

```toml
[audio]
pause_media = false
```

**Check which players Voxtype can see:**

```bash
busctl --user list | grep org.mpris.MediaPlayer2
```

An empty list means no MPRIS-capable player is running, which is a different
problem from a missing `playerctl`. Some players (notably certain browsers)
report unreliable MPRIS status; skip those with
`pause_media_ignored_players`.

## OSD looks different after upgrading, or styles do nothing

Check which frontend is running and why:

```bash
voxtype setup osd --status
```

- **The OSD changed look after an upgrade.** Configs without `[osd] frontend` now default to `auto`, which prefers the Quickshell OSD when it can run. To get the old look back: `voxtype config set osd.frontend gtk4`, then `systemctl --user restart voxtype`.
- **Picking a style in Settings changes nothing.** Quickshell styles only apply to the Quickshell OSD. If `Active:` says `gtk4` or `native`, either your config names that frontend (switch with `voxtype setup osd --use-auto`) or Quickshell can't run here. The `Reason:` line says which: `voxtype-osd-quickshell is not installed`, Quickshell is not on PATH (install the `quickshell` package), or no Wayland session.
- **`Reason:` says `gtk4 OSD installed but libgtk4-layer-shell is missing`.** Your distro doesn't package gtk4-layer-shell (Ubuntu 24.04, for example), so voxtype uses the native OSD instead. Install gtk4-layer-shell if your distro has it, or install Quickshell.
- **A notification says "Voxtype: OSD frontend '...' unavailable".** The frontend your config names can't run, so another one is standing in. The notification and the `Reason:` line say why: its binary is not installed, or, for `quickshell`, Quickshell is not on PATH (install the `quickshell` package).
- **`Active: none`.** No OSD frontend is installed, or none can reach a display. Install a frontend, or set `[osd] enabled = false` to silence the warning.
- **The Settings panel keeps saying "Changes take effect after restart".** It clears after any daemon restart, including `systemctl --user restart voxtype` and `voxtype setup model`, within about five seconds while the panel is open, or the next time you open it.

The choice is made when the daemon starts, so restart it after installing Quickshell or gtk4-layer-shell, or after changing `osd.frontend`.

## Quickshell OSD Issues

These apply when `[osd] frontend = "quickshell"`. See the
[configuration guide](CONFIGURATION.md#quickshell-osd-customization) for the
full set of style, palette, layout, and recipe options.

### OSD exits with "OSD style '...' not found"

The `[osd] style` value names a package that isn't installed. The error lists
every directory that was searched. Install the style package into one of
those directories (typically `~/.config/voxtype/osd/<name>/`), or set
`style = "default"`. A package directory must contain a `voxtype-osd.toml`
manifest.

### OSD exits with "plugin_path ... is not an OSD package directory"

`[osd] plugin_path` must point at a directory containing `voxtype-osd.toml`.
Fix the path (a leading `~` is allowed) or remove `plugin_path` from
config.toml.

### OSD exits with "OSD package qml_entry not found"

The selected package's manifest names a `qml_entry` file that doesn't exist
in the package directory. Fix or remove `qml_entry` in the package's
`voxtype-osd.toml`; without it the package uses the built-in renderer.

### Custom package selected but the default card renders instead

If a package's custom QML fails to load, the OSD logs a warning and falls
back to the built-in surface rather than rendering nothing. Run the launcher
in the foreground to see the QML error:

```bash
voxtype-osd-quickshell --no-daemonize
```

### OSD looks or behaves like an older version after a binary upgrade

The Quickshell frontend is two parts: the launcher binary and the QML tree it
runs (installed to `~/.local/share/voxtype/quickshell/`). Package upgrades
keep them in sync through `/usr/share/voxtype/quickshell`, but a manual
binary swap upgrades only the launcher, and the stale QML keeps rendering old
behavior. The launcher's manifest check catches missing files, not outdated
ones. After any manual binary upgrade, re-sync the tree:

```bash
voxtype setup quickshell --force
```

### Style or recipe changes don't show up

The launcher resolves `[osd]` config once at startup and writes the result to
`$XDG_RUNTIME_DIR/voxtype/quickshell-style.json`. After editing config.toml
or a package manifest, restart the daemon (or the OSD) to re-resolve:

```bash
systemctl --user restart voxtype
```

Omarchy theme switches are different: when the palette is `omarchy` (the
default when no package chooses one), the launcher watches the active theme
and rewrites the style file, and the OSD applies the new colors without a
restart. If a theme switch doesn't recolor the OSD, check that `palette`
isn't set to `fallback`, `package`, or `custom`, and restart the daemon if
the launcher was started before the theme directory existed.

### Shell Line or Shell Dots doesn't match the shell's corners or border

Both shell presets need a launcher and QML tree that support `layout = "shell"`.
If the layout is rejected or its shape is unchanged, upgrade both parts as
described above. The radius comes from `hyprctl -j getoption decoration:rounding`;
without working Hyprland IPC the design deliberately uses square corners.
The popup border/text and font/spacing tokens come from the active theme's
`shell.toml`, with `~/.config/omarchy/shell.toml` overrides. Missing or invalid
TOML uses shell defaults. Keep `palette = "omarchy"` for live token reloads.
Other layouts do not inherit these shell-specific settings.

### OSD shows no clock, countdown, or result

The elapsed time, the countdown before `max_duration_secs`, the engine and
model chip, and the result shown after a session all come from
`recording.json` and `outcome.json` in `$XDG_RUNTIME_DIR/voxtype/`. They are
missing when:

- the daemon predates these files (the QML tree was updated but the daemon
  binary wasn't), or
- `state_file = "disabled"`, which turns these files off along with the
  state file.

Check while recording:

```bash
cat "$XDG_RUNTIME_DIR/voxtype/recording.json"
cat "$XDG_RUNTIME_DIR/voxtype/outcome.json"   # after a session ends
```

No result is shown for cancelled recordings, accidental taps, or
`record start --no-osd` sessions; that is expected.

---

## Meeting Audio and Post-Meeting Command

### The other side shows up under You on speakers

On speakers your microphone hears what the speakers play, so the other side of the call, or a video playing during the meeting, can be transcribed as You. Voxtype removes it with PipeWire's echo canceller unless the meeting's output is headphones (`[meeting.audio] echo_cancel = "auto"`, the default).

Check which echo handling the meeting used:

```bash
voxtype meeting status --json      # "echo_cancel": "webrtc", "gtcrn" or "none"
journalctl --user -u voxtype | grep -i "echo handling"
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service | grep -i "echo handling"
```

The log line gives the reason when it is not `webrtc`:

- **"the output is headphones"**: voxtype read your default output as headphones (its active port, its form factor, or Bluetooth). If sound is in fact coming from speakers, set `echo_cancel = "webrtc"` to use the canceller anyway.
- **"Echo cancellation could not start"**: the canceller needs PipeWire's echo-cancel module and its WebRTC plugin. On Arch and Omarchy they come with `libpipewire` and `pipewire-audio`; elsewhere, check that `libpipewire-module-echo-cancel.so` and `aec/libspa-aec-webrtc.so` exist under your PipeWire library directories. PipeWire releases too old for the canceller's monitor mode also fall back. `$XDG_RUNTIME_DIR/voxtype/meeting-aec.log` has the canceller's own error.
- **"the mic is a monitor of the output the canceller would cancel"**: your default microphone (or `mic_device`) is a monitor of the output whose sound the canceller removes, so cancelling would leave nothing. This happens with a virtual mic fed from the speakers. Pick your real microphone as the default, or name it in `mic_device`.
- **"The echo canceller's mic has been silent for 3s"**: the canceller started but delivered nothing but digital silence, so the meeting switched to recording the microphone directly with GTCRN (the status then says `gtcrn`). Either the canceller is not receiving the microphone, or your microphone is a monitor of your default output (a virtual mic fed from the speakers), whose every sound the canceller treats as echo. Check that `pactl list short sources` shows your real microphone and that it is not muted; `$XDG_RUNTIME_DIR/voxtype/meeting-aec.log` has the canceller's own output. `echo_cancel = "gtcrn"` skips the canceller.
- **"mic_device is not a PipeWire source"**: `meeting.audio.mic_device` names an ALSA device. Set it to a PipeWire source name from `pactl list short sources`, or to `default`.
- **"the default source is already an echo canceller"**: you run your own echo canceller as the default microphone, and voxtype records from it rather than adding a second one. If echo still gets through, check that canceller's setup, or name your real microphone in `mic_device` to have voxtype's canceller instead.
- **`none`**: `echo_cancel` is `disabled`, or it is `gtcrn` and the GTCRN model is missing.

With a filter as your default microphone (Camera Effects, a noise-suppression chain), the log also says "The default source … is a filter over …": the canceller records the real microphone under the filter, so the filter's effects are not in the meeting's mic track. That is expected.

The simplest fix is headphones: with nothing coming out of speakers there is no echo to remove, and voxtype skips the canceller on its own.

### The recording announcement only played on my speakers

With `announce_recording` on, `voxtype meeting status` says where the announcement went. "played on your speakers only; the call did not hear it" comes with the reason:

- **"no call app opened the microphone within 120 s"**: voxtype found no call. It recognizes calls the way meeting detection does, so run `voxtype meeting suggest --dry-run` during the call to see what it sees. A browser call (Meet, Teams on the web) needs Hyprland for its window title; on other compositors only Zoom and the Teams app are found. Add another app under `[meeting.detect]`, as in [Meeting Detection](MEETING_MODE.md#meeting-detection).
- **"pw-play is not installed"** or **"cannot run pw-dump"**: the announcement needs PipeWire's command-line tools (`pw-play`, `pw-link`, `pw-dump`; the `pipewire` package on Arch, `pipewire-bin` on Debian and Ubuntu).
- **"could not link into ...'s mic stream"**: PipeWire refused the link. `journalctl --user -u voxtype | grep -i announcement` shows `pw-link`'s error (use `journalctl _SYSTEMD_USER_UNIT=voxtype.service | grep -i announcement` when [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)).

When it says "played into ...'s microphone", PipeWire carried it into the call app's mic stream. The others still will not hear it if you are muted in the call app, or if the app's noise suppression removes it; unmute before the meeting starts, or start it with `--no-announce` and tell them yourself.

### The announcement used the built-in voice instead of Jessica

With `announcement_voice = "jessica"`, a status line ending "Note: the Jessica voice could not be downloaded (...), so the built-in voice played" means the one-time download failed and the meeting announced in the built-in Kokoro voice instead. The reason is in the brackets:

- **"timed out after 15 s"** or a curl error: voxtype could not reach `models.voxtype.io`. Download it beforehand, when you are online, with `voxtype meeting announcement --voice jessica` (it waits up to two minutes).
- **"rejected the download"** or **"does not match the published clip"**: what arrived was not the published file, and voxtype never plays a clip that fails its sha256 check. Run `voxtype meeting announcement --voice jessica` again. A proxy that rewrites downloads can cause this.

`voxtype meeting announcement` shows the voice in use and whether Jessica has been downloaded.

### Meeting audio is saved as WAV instead of Opus

With `[meeting] retain_audio = true`, voxtype encodes the audio with
`ffmpeg`. When `ffmpeg` is not on the daemon's `PATH`, or was built without
libopus, the daemon logs one of these and keeps `mic.wav`, `loopback.wav` and
`mixed.wav` instead (about 115 MB per hour per file, against about 10 MB for
Opus):

```
ffmpeg not found; keeping meeting audio as WAV ...
/usr/bin/ffmpeg has no libopus encoder; keeping meeting audio as WAV ...
```

Install `ffmpeg` (`sudo pacman -S ffmpeg`, `sudo apt install ffmpeg`,
`sudo dnf install ffmpeg-free`) and check it has the encoder:

```bash
ffmpeg -hide_banner -encoders | grep libopus
```

Then restart the daemon (`systemctl --user restart voxtype`); the check runs
once per daemon. Meetings already saved as WAV stay WAV. To convert one:

```bash
cd ~/.local/share/voxtype/meetings/<meeting>
for t in mic loopback mixed; do
  [ -f $t.wav ] && ffmpeg -i $t.wav -c:a libopus -b:a 24k $t.opus && rm $t.wav
done
```

`meeting.json` lists whichever files are there the next time it is rewritten.

### `meeting import` or `meeting retranscribe` fails

- **"need ffmpeg and ffprobe"**: install ffmpeg; ffprobe comes with it.
- **"Cannot read ..."**: ffprobe could not find an audio stream. Check the
  file plays (`ffprobe file`); a folder is only accepted when it is an Omarchy
  Meeting Recorder meeting folder.
- **"Invalid --channels ..."**: name one side per channel, in order. The
  message says how many channels the file has; `ffprobe -show_streams file`
  shows the layout. Leave `--channels` out to mix everything.
- **"recorded without keeping audio"**: `meeting retranscribe` needs the
  meeting's audio. Meetings recorded with `retain_audio = true`, and every
  imported meeting, have it.
- **A model or engine is refused**: the same checks as `meeting start
  --model` apply; download the model first (`voxtype setup model`).
- **Speaker names look swapped after transcribing again with `ml`
  diarization**: the speaker numbers come from a new clustering. Rename them
  with `voxtype meeting label`.

### `meeting play` fails or plays nothing

- **"needs ffmpeg and one of pw-play (PipeWire), paplay (PulseAudio) or
  aplay (ALSA)"**: install what the message names as missing. `pw-play`
  comes with PipeWire (`pipewire` or `pipewire-bin`), `paplay` with
  `pulseaudio-utils` or `libpulse`, `aplay` with `alsa-utils`.
- **"This meeting was recorded without keeping audio"**: the meeting was
  recorded with `retain_audio` off. Turn it on for future meetings; there is
  nothing to play for this one.
- **It says it is playing but you hear nothing**: playback goes to your
  default output. Check it with `wpctl status` (PipeWire) or
  `pactl get-default-sink`, and look at
  `$XDG_RUNTIME_DIR/voxtype/meeting_playback.log` for errors from ffmpeg or
  the player.

### The post-meeting command did not run, or did not finish

`post_meeting_command` runs in the background once the meeting's folder is
final, and its outcome is in the daemon log:

```bash
journalctl --user -u voxtype | grep post_meeting_command
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service | grep post_meeting_command
```

- **"failed ... (exit status: 127)"**: the shell could not find the program.
  The daemon's `PATH` under systemd is shorter than your terminal's; use an
  absolute path.
- **"ran past 300s and was killed"**: raise `post_meeting_timeout_secs`, or
  have the command start its long work with `systemd-run --user` so it is no
  longer the command's child.
- **Nothing logged**: check that `post_meeting_command` is under `[meeting]`
  and the daemon was restarted after you set it (`voxtype config` shows it).

The command runs in the meeting folder with `VOXTYPE_MEETING_ID`,
`VOXTYPE_MEETING_DIR` and `VOXTYPE_MEETING_MANIFEST` set, and without a
terminal: anything that prompts for input fails.

### A meeting shows as interrupted, with no summary

This is what the "A meeting didn't finish" notification is about. The
daemon stopped while the meeting recorded: it crashed, was killed, or the
computer lost power. Everything saved until then is in the meeting's folder,
but nothing finished the meeting, so it has no end time or summary and
`post_meeting_command` did not run. Finish it from what is on disk, or
delete it:

```bash
voxtype meeting recover                  # list them
voxtype meeting recover --finish <id>    # end it, summarize, run the command
voxtype meeting recover --discard <id>   # delete it and its folder
```

With `retain_audio` on, finishing transcribes the audio kept after the last
transcribed chunk too (that needs ffmpeg and the meeting's model). Without
it, what was said after the last chunk (up to `chunk_duration_secs`, 30
seconds by default) is lost. If the daemon is crashing, its log has the
reason: `journalctl --user -u voxtype -b`, or
`journalctl _SYSTEMD_USER_UNIT=voxtype.service -b` when
[the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries).

## Meetings: The Remote Side Shows Up Under "You"

With speakers instead of headphones, your microphone hears the call. Voxtype removes that echo from your lines, but what is left can still show up:

- **Long passages of the remote's words under You.** Check that loopback capture is running (`voxtype meeting status`); every echo check compares against the loopback track, including the guard's. Check `[meeting.audio] echo_guard` is on (the default). Keep `echo_cancel = "auto"`, or set up PipeWire's `echo-cancel` module, or use headphones. A line that mixes your voice with the echo stays yours, echo included: the guard only removes lines that are nothing but echo.
- **Short garbled fragments ("Omarky problem.", "5.").** These are misheard echo. The guard drops them when they are at most `echo_fuzzy_max_words` words and match the remote line spoken at the same time. Raise `echo_fuzzy_max_words` a little for longer fragments.
- **"Thank you." under You while you said nothing.** Speech engines write a few stock phrases ("Thank you.", "Thanks for watching.") for faint sound, such as the echo canceller's residual. The guard drops them when the remote is talking and your microphone held no voice of yours at the time. One that comes while the call is silent stays: delete it with `voxtype meeting delete-line`.
- **One sentence repeated over and over in a line.** On faint audio, such as the echo of the other side, a speech engine can get stuck and write the same phrase until the audio ends ("I don't know what you're talking about." six times). Voxtype keeps one copy of any phrase of three or more words that comes three or more times in a row, for every engine, and logs "Collapsed a repeated phrase" with the number of words before and after (the line itself at debug level). Two repeats ("Yes. Yes.") and short words said over and over ("No, no, no.") are left as they are. If the collapsed line was echo, the guard then drops it when it is at most eight words, opens with three words in a row from the other side's line at the same time, and your microphone held no voice of yours. A line that keeps repeating after this was most likely said that way, or is long enough to be left: delete it with `voxtype meeting delete-line`.
- **"Thank you." or "you" from a remote speaker nobody heard.** The same stock phrases, written for silence or a join sound on the call's side. The guard drops them when the loopback held no speech at the time. A tone or chime is sound, so a phrase written over one can stay: delete it with `voxtype meeting delete-line`.
- **A remote line shows up twice, once under the remote speaker from your microphone.** The guard moved an echo line to the remote side because the loopback's transcript did not have its words. Delete the copy with `voxtype meeting delete-line`.
- **A line of yours went missing or moved to the remote.** The guard keeps a line when your microphone is louder than the echo explains. A reply mumbled very quietly over a loud call can still read as echo. Speak up or lower the call volume; move a line back with `voxtype meeting reassign <meeting> <line> You`; or set `echo_guard = false` to keep only exact-word matching.

The log counts what the guard did per chunk ("Echo guard: dropped N microphone line(s) ..., moved N ...", "Removed N bleed-through word(s) via dedup"); at debug level it names each line:

```bash
systemctl --user stop voxtype
RUST_LOG=voxtype::meeting=debug voxtype 2>&1 | grep -iE "echo"
```

---

## No Meeting Window Appears

When a meeting starts, the daemon opens the view `[meeting] view` resolves to (the Quickshell view opens itself), and `voxtype meeting view` opens it by hand. Run `voxtype meeting view` first: when nothing can open it says why.

- **"No meeting view to open (auto: no Quickshell; voxtype-meetings-gtk4 is not installed)".** Install the deb/rpm, which ship the window in `/usr/lib/voxtype/`, or watch in a terminal with `voxtype meeting watch`.
- **"…voxtype-meetings-gtk4 is installed but libgtk-4 is missing".** Install GTK 4: `libgtk-4-1` (Debian, Ubuntu), `gtk4` (Fedora, Arch).
- **"no display".** The command ran without `WAYLAND_DISPLAY` or `DISPLAY` (over SSH, or from a service started before the desktop). Run `voxtype meeting watch` there instead.
- **Quickshell is installed but you want the window.** `voxtype config set meeting.view gtk4`.
- **On Quickshell, nothing happens.** The Quickshell view opens only in a running Omarchy shell or voxtype's standalone shell (`voxtype setup quickshell`).
- **`voxtype meeting view` works, but nothing opens when a meeting starts.** Check `voxtype config get meeting.show_view_on_start` (it must be `true`), then the daemon's log, which says what it decided for each meeting: `journalctl --user -u voxtype | grep "Meeting view on start"` (or `journalctl _SYSTEMD_USER_UNIT=voxtype.service | grep "Meeting view on start"` when [the user journal has no voxtype entries](#the-user-journal-has-no-voxtype-entries)). "no display" there means the service has no `WAYLAND_DISPLAY` or `DISPLAY`: import them into the user session (`systemctl --user import-environment WAYLAND_DISPLAY DISPLAY`, which most compositors' autostart does) and restart the daemon.
- **Both the Quickshell view and the window open.** The Quickshell view opens on its own unless `[meeting] view` is `gtk4`, `terminal` or `none`; set one of those, and run `omarchy-restart-shell` (or restart your Quickshell shell) once after upgrading so it reads the setting.
- **The window closed when the daemon restarted.** A window the daemon opened runs as part of the voxtype service and stops with it. Open it again with `voxtype meeting view`, which starts it on its own.

## Meeting Assistant: Summaries Don't Appear

Start with what the assistant sees:

```bash
voxtype meeting assistant status
journalctl --user -u voxtype --since "10 minutes ago" | grep -i summar
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "10 minutes ago" | grep -i summar
```

**No summary after a meeting stops.** The summary at stop is off until an agent is allowed, unless `[meeting.summary] auto = true`. Allow one with `voxtype meeting assistant allow claude`, or summarize by hand with `voxtype meeting summarize latest`. A meeting with no transcript is never summarized. `auto = false` turns it off.

**"Meeting transcripts have not been allowed to go to Claude".** Consent is asked once per agent and the daemon cannot ask. Run `voxtype meeting assistant allow <agent>`, or use `--local` / `local_only = true` to keep everything on the machine.

**The CLI finds the agent but the daemon does not.** A systemd user service often has a shorter PATH than your shell (no `~/.local/bin`, no mise or asdf shims). `meeting assistant allow` records where the agent was found and the daemon falls back to that path, so allow it again after moving or reinstalling the agent. `[meeting.assistant] agent` only names the agent; it does not change where it is looked for.

**"Claude failed: Not logged in" or similar.** The agent runs with your account, as you. Sign in once in a terminal (`claude`, then `/login`; `codex login`) and retry with `voxtype meeting summarize latest --regenerate`.

**"did not answer within 180s".** Long meetings take a while to summarize. Raise `[meeting.assistant] timeout_secs`, or lower `max_transcript_chars` so less is sent verbatim.

**"Could not read the summary the model wrote".** The model did not reply with the JSON the summary asks for. Small local models do this most; try a larger Ollama model or an agent, then `--regenerate`.

**Codex is listed as not usable: "needs bubblewrap".** Codex keeps a command tool that cannot be turned off, so voxtype only runs it inside a bubblewrap jail that hides your home. Install `bubblewrap` (`pacman -S bubblewrap`, `apt install bubblewrap`). If bwrap is installed but fails with a namespace error, unprivileged user namespaces are disabled on the system; use Claude or the local model instead.

**Codex worked, then fails with an authentication error.** In the jail codex can read `~/.codex/auth.json` but not rewrite it, so it cannot save a refreshed login token there. Run `codex` once in a terminal to refresh the login, then retry.

**Hermes, OpenCode or another agent is my Omarchy default.** Only `claude`, `codex` (jailed) and `pi` can be run without tools that reach your files or the network, so voxtype skips other agents and uses `claude` or `pi` from PATH, or the local model. Set `[meeting.assistant] agent` to choose explicitly.

**Answers and summaries need an agent or a local model.** No supported agent was found and Ollama did not answer at `ollama_url`. Install Claude Code, Codex or Pi, or start Ollama and `ollama pull llama3.2`.

**"No Omarchy text model is running" in the log, and the summary came from my agent or Ollama.** `[meeting.summary] backend = "omarchy"` uses Omarchy's text model only while one is running, and voxtype never starts one. Start it with `omarchy models start <recipe>` or from the local models panel (Super + Ctrl + Alt + X); `omarchy models list` shows which models fit your machine, and many machines are offered no text model at all. `voxtype doctor` shows what voxtype sees under "Omarchy local AI". With `omarchy_model` set, that exact recipe must be running.

**"Omarchy (...) refused the API key (HTTP 401)".** Voxtype reads `~/.config/omarchy/agent-local/key` for each request. If Omarchy set the model up again, the running model may still hold the old key: restart it from the local models panel.

**Ask through Pi fails when Pi's default model is an Omarchy one.** Voxtype runs Pi with `--no-extensions`, and loads Omarchy's `~/.pi/agent/extensions/omarchy-local.ts` back in only when `defaultProvider` in `~/.pi/agent/settings.json` is `omarchy-local` (or `defaultModel` is `omarchy-local/<recipe>`). A default on a paired machine (`omarchy-local-<host>`) is not supported this way. `[meeting.summary] backend = "omarchy"` reaches the same model without Pi.

The meeting itself never depends on the summary: if summarizing fails, the transcript is saved as usual and the meeting socket reports an `error` event naming the meeting.

---

## Action Items: Todoist and Reminders

**`meeting actions` shows nothing.** Action items come from the summary. A meeting without one has none; run `voxtype meeting summarize latest` (or `--regenerate` for an older summary).

**`todoist connect` says every port is in use.** Todoist's answer comes back to `http://localhost` on port 47815, 47816, 47817, 47818 or 47819. Find what holds them with `ss -ltnp | grep 4781`, free one, and connect again.

**The browser shows a Todoist error about the redirect address, or `connect` says Todoist would not accept a local redirect address.** Todoist did not accept voxtype's `http://localhost` return address. Please report it on GitHub with the message. Meanwhile, a personal API token works: see `todoist_token_file` in [CONFIGURATION.md](CONFIGURATION.md#todoist_token_file).

**Sending says Todoist refused the connection.** The access was revoked in Todoist or the tokens no longer renew. Run `voxtype meeting todoist connect` again.

**`--send reminder` is not available.** Reminders use `omarchy-reminder`, which comes with Omarchy. Elsewhere, send to Todoist instead.

## Meeting Titles: The Calendar Title Was Not Used

Start with what voxtype sees:

```bash
voxtype meeting calendar status
journalctl --user -u voxtype --since "15 minutes ago" | grep -i calendar
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "15 minutes ago" | grep -i calendar
```

A meeting takes its title from the calendar only when it started without a title you chose, and an event was in progress or started within `[meeting.calendar] window_mins` of the start.

- **"Last fetch failed: The calendar server (...) answered 404"** (or 401, 403, 410): the link was reset or unpublished. Copy the secret iCal address again and run `voxtype meeting calendar set-ics -`.
- **"Could not reach the calendar server"**: no network when the daemon fetched. It tries again within 15 minutes and uses the last good fetch meanwhile.
- **"did not return a calendar"**: the link points at a web page, often a sign-in page, rather than the iCal feed. Use the secret or published iCal address, not the calendar's ordinary web address.
- **"No event in the calendar link within 10 min"**: the event was further from the start than the window, or was an all-day event, which never names a meeting. Raise `window_mins`, or set the title with `voxtype meeting title latest "<title>"`.
- **A calendar command: "ran past 3 s"**: raise `[meeting.calendar] timeout_secs`. **"printed something other than the calendar JSON"**: run the command yourself (`bash -lc '<command>'`) and check it prints one JSON object and nothing else; a login profile that prints a greeting breaks it.

Voxtype never logs the iCal link itself, only its server, so these lines are safe to share in a bug report.

## Systemd Service Issues

### Service fails to start

```bash
# Check status
systemctl --user status voxtype

# View logs
journalctl --user -u voxtype -n 50
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service -n 50

# Common issues:
# - Built-in hotkey can't read the keyboard (use a compositor keybinding
#   and [hotkey] enabled = false; the daemon keeps running either way)
# - Model not downloaded
# - ydotool not running
```

### Service starts but doesn't work

**Symptom:** A notification at startup titled "Voxtype: no display in its environment", or text is never typed when voxtype runs as a service but works from a terminal.

**Cause:** The systemd user manager does not have `WAYLAND_DISPLAY` (or `DISPLAY` on X11), so the voxtype service doesn't either, and wtype, wl-copy and friends cannot reach your desktop. This happens when the compositor does not import its environment into systemd.

**Solution:** `voxtype doctor` reports whether `WAYLAND_DISPLAY` or `DISPLAY` is set in the systemd user environment (your terminal's environment says nothing about the service's). If neither is, import them from a terminal in your desktop session and restart the service:
```bash
systemctl --user import-environment WAYLAND_DISPLAY DISPLAY
systemctl --user restart voxtype
```

To make it permanent, run the import when your compositor starts, before voxtype:
```
# Hyprland (hyprland.conf)
exec-once = systemctl --user import-environment WAYLAND_DISPLAY DISPLAY

# Sway (config)
exec systemctl --user import-environment WAYLAND_DISPLAY DISPLAY
```

`voxtype setup systemd` also checks the user manager's environment and prints this fix when the variables are missing.

### Service doesn't start on login

```bash
# Enable the service
systemctl --user enable voxtype

# Check if it's enabled
systemctl --user is-enabled voxtype

# Check startup targets
systemctl --user list-dependencies default.target
```

---

## Debug Mode

### Enable verbose logging

```bash
# Debug level: each step of a dictation, and the transcribed text
voxtype -v

# Trace level (most verbose)
voxtype -vv

# Or via environment
RUST_LOG=debug voxtype
RUST_LOG=voxtype=trace voxtype
```

Stop the service first (`systemctl --user stop voxtype`) when you run the daemon in a terminal. To get debug logs from the service instead:

```bash
systemctl --user edit voxtype
#   [Service]
#   Environment=RUST_LOG=voxtype=debug,warn
systemctl --user restart voxtype
journalctl --user -u voxtype -f
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service -f
```

`RUST_LOG` takes precedence over `-v` and `-q` on the command line. An empty `RUST_LOG` counts as unset. Remove the line and restart the service when you are done.

### The user journal has no voxtype entries

On some systems the per-user journal has no entries for the daemon. `journalctl --user -u voxtype` then prints `-- No entries --`, or stops at an old date, while the service is running and logging. Match on the unit name instead, which reads every journal your account can open:

```bash
journalctl _SYSTEMD_USER_UNIT=voxtype.service -f
```

It takes the same options (`-f`, `-n 50`, `-b`, `--since "5 min ago"`) and pipes into `grep` the same way, so it can stand in for `journalctl --user -u voxtype` in any command in this documentation. If it prints nothing either, your account may not be allowed to read the system journal: add it to the `systemd-journal` group, or run the command with `sudo`.

`voxtype info accel` reads the user journal too, so on such a system it can report "unknown" after a dictation.

### Dictated text in the log

At the default level the log does not contain what you say: a dictation is logged as its length and its engine (`Transcribed 42 chars (whisper engine)`). To see what the engine heard, which is what a report about wrong or missing words needs, turn on debug logging as above and look for these lines:

```bash
journalctl --user -u voxtype --since "5 min ago" | grep -E "Transcribed:|text:|After text processing|Post-processed result"
# If that prints nothing: journalctl _SYSTEMD_USER_UNIT=voxtype.service --since "5 min ago" | grep -E "Transcribed:|text:|After text processing|Post-processed result"
```

- `Transcribed: "..."`: the text the engine returned
- `Cohere transcription text: "..."` (`Transcription text:` for Whisper, and so on for the other engines): the same, from the engine itself
- `After text processing: "..."`: after replacements, spoken punctuation and the other `[text]` steps
- `Post-processed result: "..."`: after the `[output.post_process]` command

With debug logging on, the journal keeps every dictation until it rotates out, so turn it off again afterwards. A debug log contains your dictations: read it before you attach it to an issue, and cut out anything you do not want to publish.

### Debug specific components

```bash
# Audio capture issues
RUST_LOG=voxtype::audio=debug voxtype

# Hotkey issues
RUST_LOG=voxtype::hotkey=debug voxtype

# Whisper issues
RUST_LOG=voxtype::transcribe=debug voxtype

# Output issues
RUST_LOG=voxtype::output=debug voxtype
```

### Log to file

```bash
voxtype -vv 2>&1 | tee voxtype.log
```

### Check system logs

```bash
# Kernel input messages
dmesg | grep -i input

# Audio system
journalctl --user -u pipewire -n 20
journalctl --user -u pulseaudio -n 20
```

---

### Trained LARD cleanup is not running

- It is off by default. Enable `text.lard_disfluency`. It then runs only when
  the dictation is known to be English: the engine's language is set to
  English, the model is English-only, or you declared English with
  `text.language`.
- Run `voxtype doctor`. With the setting on and nothing to run, the **Text
  cleanup** check fails and names the cause. The same problem is in the
  daemon's "Voxtype: N problems need attention" startup notification and in
  the log as `LARD cleanup unavailable; preserving text`.
- "it only runs on English, and nothing says ... dictation with ... is
  English": the model handles several languages and detects them
  automatically, so Voxtype does not know what language the text is in. This
  is the usual case with Parakeet v3, Whisper on `auto`, SenseVoice on `auto`,
  Dolphin, Omnilingual and Soniox, and with any model given as a path. If you
  dictate in English, declare it and restart the daemon:
  `voxtype config set text.language en`. If you dictate in several languages,
  leave it and turn the stage off instead.
- "it only runs on English, and whisper.language is fr" (or another engine's
  `language` key): the engine was told to write another language, and that
  wins over `text.language`. If you do dictate in English, change that key:
  `voxtype config set whisper.language en`.
- "it only runs on English, and whisper.language is limited to fr, de" (or
  `soniox.language_hints`): detection is limited to a set without English, so
  a declared `en` would contradict it. Add `en` to that list.
- "it only runs on English, and ... only writes ja": the model writes one
  language and it is not English. No setting changes that. Pick another model
  or turn the stage off.
- "it only runs on English, and text.language is ...": you declared another
  language, or a value that is not a language code (`english` rather than
  `en`). Set it to `en`.
- It runs for normal dictation but not with `--model` or the secondary-model
  hotkey: an override recording is checked on its own, and the override model
  is not known to write English. Declare `text.language en`, or use an
  English-only override model.
- "LARD disfluency cleanup is on but its model is not installed": the setting
  is on and the model file is not there. Turning the setting on does not
  download it. Run `voxtype setup lard`, then restart the daemon. Or turn the
  stage off: `voxtype config set text.lard_disfluency false`.
- `voxtype setup lard` fails with "answered HTTP ..." or "could not reach":
  the download did not complete. Nothing was installed and an existing model
  is untouched. Check your connection and run it again, or install from a copy
  of the file with
  `voxtype setup lard --source /path/to/lard-disfluency-v1.vxdf`.
- `voxtype setup lard` fails with "checksum mismatch": what was downloaded (or
  the file given to `--source`) is not the pinned model. The message shows the
  size and SHA-256 it got and the ones it expected. Nothing was installed. A
  captive portal or a proxy that rewrites downloads is the usual cause.
- Run `voxtype setup lard --status` to see the installed file and whether the
  stage is running for your configuration.
- An explicit `--url` must be HTTPS and return the exact pinned artifact.
- The model is licensed CC BY-SA 4.0, separately from Voxtype's MIT code. See
  the [model card](https://models.voxtype.io/text/lard-disfluency-v1.MODEL_CARD.md).
- Check `text.lard_model_path` and the XDG data directory used by the daemon.
  Restart after changing configuration. Missing or invalid models skip only
  this stage; startup never tries a download.
- Streaming partials stay raw. Cleanup runs only at finalization and can revise
  already typed partials only when backspace output succeeds. For cross-segment
  cleanup, use batch or file output. LLM post-processing is still batch-only.

### Profile vocabulary does not correct a term

- Start or toggle the recording with the profile that owns the vocabulary:
  `voxtype record start --profile code`.
- Restart the daemon after changing `terms` or `terms_file`; vocabulary files
  are loaded at startup.
- Run `voxtype -vv` and look for `Profile vocabulary disabled`. The warning
  names unreadable files and JSON that is not an array of strings.
- Use `[text.replacements]` for short terms, distant spellings, or an exact
  correction. Vocabulary intentionally leaves uncertain matches unchanged.
- Profile terms are appended to an explicit Whisper prompt or Soniox terms
  list, not substituted for it. Enable `-vv` logging to inspect which profile
  is active without logging private term values.
- Streaming correction applies only to finalized segments. A provisional word
  can remain misspelled until the backend marks that segment final.

## Getting Help

If you're still having issues:

1. **Run the health check:** `voxtype doctor` (attach `voxtype doctor --json` to bug reports)
2. **Gather debug logs:** `voxtype -vv 2>&1 | tee debug.log`. Debug logs include the text of what you dictate while they are on, so read the file before you attach it
3. **Check system info:**
   ```bash
   uname -a
   groups
   pactl info
   systemctl --user status ydotool
   ```
4. **Open an issue:** https://github.com/peteonrails/voxtype/issues

Include:
- Voxtype version (`voxtype --version`)
- Linux distribution and version
- Wayland compositor
- Debug log output
- Steps to reproduce

---

## Feedback

We want to hear from you! Voxtype is a young project and your feedback helps make it better.

- **Something not working?** If Voxtype doesn't install cleanly, doesn't work on your system, or is buggy in any way, please [open an issue](https://github.com/peteonrails/voxtype/issues). I actively monitor and respond to issues.
- **Like Voxtype?** I don't accept donations, but if you find it useful, a star on the [GitHub repository](https://github.com/peteonrails/voxtype) would mean a lot!
