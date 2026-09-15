# Voxtype Settings — an Omarchy shell panel

A graphical settings panel for [voxtype](https://voxtype.io), running as a
panel plugin inside `omarchy-shell`. It renders whatever `voxtype config schema
--json` reports and writes changes back through `voxtype config set`, so the
panel, the CLI, and `voxtype configure` all agree about defaults and validation
without keeping a second copy of the schema.

## Requirements

- Omarchy 4 with the Quickshell shell (`omarchy-shell`)
- **voxtype 0.8 or newer.** The panel is built entirely on `voxtype config
  schema --json`, which earlier builds do not have. On an older voxtype the
  panel opens onto a card offering to install/update or to open the TUI
  instead.

## Install

```bash
omarchy plugin add https://github.com/peteonrails/omarchy-voxtype-settings.git --enable --yes
```

Or by hand, from a checkout of this directory (the one holding
`manifest.json`):

```bash
cp -r . ~/.config/omarchy/plugins/io.voxtype.settings
omarchy-shell shell rescanPlugins
omarchy plugin enable io.voxtype.settings
```

Add the microphone settings icon to the bar:

```bash
omarchy bar put io.voxtype.settings --section right
```

Click it to open the **Audio** section; click again to close the panel. The
tooltip reads **Voxtype settings · Audio**. The icon uses the native bar theme
and sizing, including vertical bars.

On builds that support `audio.keep_ready`, Audio begins with **Keep microphone
ready**. Enabling it keeps the microphone stream active between dictations to
reduce recording startup delay; idle samples are discarded. The microphone
indicator can stay on and power usage can increase. Use the panel's restart
banner to restart Voxtype after changing this setting.

Optionally add a **Setup → Dictation** row to the Omarchy menu:

```bash
./install/install-menu.sh            # add it
./install/install-menu.sh --remove   # take it back out
```

The script merges one key, `setup.dictation`, into
`~/.config/omarchy/extensions/omarchy-menu.jsonc` and leaves every other key
alone. Running it twice changes nothing. The shell watches that file, so the row
appears without a restart. If the file contains comments they are not
reproducible from parsed JSON, so the previous version is copied to
`omarchy-menu.jsonc.bak` first.

## Use

```bash
omarchy-shell shell toggle io.voxtype.settings '{}'
```

An optional payload opens the panel on a particular section:

```bash
omarchy-shell shell toggle io.voxtype.settings '{"section":"Hotkey"}'
```

The plugin also registers its own IPC target, which is the friendlier form for a
keybind or a script because it takes no JSON:

| Command | Does |
|---|---|
| `omarchy-shell io.voxtype.settings open` | open the panel (no-op if already open) |
| `omarchy-shell io.voxtype.settings close` | close it |
| `omarchy-shell io.voxtype.settings toggle` | open or close |
| `omarchy-shell io.voxtype.settings section Hotkey` | open on a section, or jump to it if already open |
| `omarchy-shell io.voxtype.settings restart` | restart the voxtype daemon without opening anything |
| `omarchy-shell io.voxtype.settings download <model>` | start the same download the model row's button starts |
| `omarchy-shell io.voxtype.settings cancelDownload` | stop the download in progress |

`section` answers `unknown section: <name>` rather than showing an empty pane.
There is no `status` verb: `voxtype status --format json` already answers that.

Both routes end up in the same place. The `shell toggle` form goes through the
shell's plugin registry, the plugin's own target goes straight to the panel and
then back through the shell, so its record of which panels are open stays right
either way.

Because a keybind can fire while the panel has never been opened, the manifest
sets `keepLoaded: true` — the panel is instantiated with the shell so its IPC
target exists from the start. Adding or removing that flag needs a full
`omarchy-restart-shell`; `rescanPlugins` alone will not re-instantiate it.

## Keybinding

Add this to `~/.config/hypr/bindings.lua`:

```lua
o.bind("SUPER + CTRL + ALT + V", "Voxtype settings", "omarchy-shell io.voxtype.settings toggle")
```

Omarchy's stock voxtype binds (`default/hypr/bindings/voxtype.lua`) are
**SUPER+CTRL+X** for toggle dictation and **F9** for push-to-talk, and
SUPER+CTRL+V is the clipboard manager, so SUPER+CTRL+ALT+V collides with none of
them — the SUPER+CTRL+ALT row is where Omarchy already keeps its second-tier
panels (SUPER+CTRL+ALT+D is the calendar).

Inside the panel:

- Under the title is a line of what is actually running, as opposed to what is
  configured to run: engine and model, the daemon's state, the systemd unit's
  `ActiveState` and main pid, and GPU memory. The unit is reported separately from
  the daemon because a unit sitting in `failed` reports no daemon status at all,
  and that state is shown in the urgent color rather than folded into "not
  running". Every item is measured: a reading the panel could not take is left
  out rather than guessed, so a missing item means "not known", never "zero". The
  whole line is one elided Text, so a long model name cuts the line off at the
  card's edge instead of pushing the later readings outside it.
- The title row carries an acceleration badge from `voxtype info accel --json`:
  **GPU · MIGraphX** tinted as confirmation, **CPU fallback** in the urgent colour,
  and a quiet plain **CPU** when the build has no GPU provider at all. The
  fallback state is the one that matters — it means acceleration was asked for and
  did not happen — so it is the loudest thing in the header. `unknown` and
  `not-running` draw nothing, as does a voxtype without the subcommand. It is
  fetched on open and after a restart, never on the status poll: it reads the
  journal and is far heavier than asking the daemon its state.
- The badge sits where the daemon-state dot used to be. The daemon's state is
  reported once, on the facts line; it used to appear in both places, so "Daemon
  idle" was on screen twice.
- GPU memory comes from `nvidia-smi` filtered to the daemon's pid where that
  works, otherwise from `rocm-smi` as the card total (ROCm's per-process
  accounting is not dependable), otherwise not at all. Which tool is used is
  decided by running it rather than by whether it is installed, because a machine
  can carry `nvidia-smi` with no driver behind it. On ROCm the poll can wake a
  GPU out of its low-power state every five seconds for as long as the panel is
  open.
- The engine switcher is pinned above the Engine section rather than scrolling
  with it, so it stays reachable while you are looking down the model list, and
  the engine in effect is stated in full instead of only being a dropdown's
  selected row.
- The panel re-reads the config file whenever it changes on disk, so an edit from
  the CLI, the TUI, or the Edit config button lands in an already-open panel
  without needing to close and re-summon it.
- The search box takes focus on open; typing searches every section at once.
- **Down** or **Tab** leaves the search box for the form, where **j/k** and the
  **arrow keys** move a cursor down the rows and **Enter** or **Space** acts on
  the row under it: a switch flips, a picker drops open, a text or number field
  takes the keyboard. **Escape** in a field hands the keyboard back to the row
  cursor rather than closing the panel.
- Switches and dropdowns write immediately; text and number fields write when
  you finish editing; sliders write 300ms after you stop moving them.
- A dot beside a control means the config file overrides the default. The button
  next to it removes the override (`voxtype config unset`).
- Settings for engines that are not compiled into your binary render dimmed, and
  the engine picker marks those engines "not in this build".
- Enums the schema marks `open` (an evdev key name, a language list, a sound
  theme that may be a path) carry a **Custom value…** row that turns the picker
  into a text field. Escape backs out of it without writing.
- Models that are not downloaded yet get a **Download** button, which downloads
  in the panel: the row swaps its button for a slim progress bar, a percentage,
  and a cancel. One download runs at a time, so the other Download buttons go
  flat while it does. A failure shows its message inline under the row; a
  cancellation shows nothing, since you asked for it. A download outlives
  dismissing the panel and switching sections — the process is owned at the top
  of the panel, not by the row that started it, so the row rebuilding does not
  interrupt it.
- Progress comes from `voxtype setup --download --model <name> --progress-format
  json`, read as NDJSON (`{"event":"progress","file":…,"pct":…}`,
  `{"event":"done"}`, `{"event":"error","message":…}`). A voxtype that rejects
  that flag gets one silent retry without it, and its human-readable output then
  drives an indeterminate bar labelled "working" rather than a percentage nobody
  reported.
- Progress is reported per file, not per model, so the bar runs 0→100 once for
  each file a multi-file model is made of. The row names the file in flight, so a
  bar starting over reads as "next file" instead of as progress going backwards.
- The catalog is what decides whether a model is on disk, and a dropped read of
  it would otherwise leave a downloaded model labelled "not downloaded" forever.
  Three things prevent that: a finished download marks its model installed
  immediately, a catalog read that arrives while one is already running is queued
  rather than dropped (and retried once if it fails), and the panel re-reads the
  catalog on a timer while it is open so stale state heals on its own.
- Voxtype reads its config at startup, so a banner appears after the first
  successful write and stays until a restart succeeds.
- The Advanced section leads with **Release GPU memory when idle**, one switch
  over two keys (`whisper.on_demand_loading` and `whisper.gpu_isolation`). Both
  are needed before the driver actually hands VRAM back between dictations, which
  is what someone who also games wants; the cost is that the model reloads on the
  next dictation. Both underlying rows still appear below it for anyone who wants
  them separately. The row only shows for the engine that has those keys.
- **Edit config** in the header opens `~/.config/voxtype/config.toml` in your
  editor (via `omarchy-launch-config-editor`) for the things a form is the wrong
  shape for.
- Escape or a click outside the card closes it.

## What it shells out to

| Action | Command |
|---|---|
| Read every setting | `voxtype config schema --json` |
| Write a setting | `voxtype config set <key> <value>` |
| Clear an override | `voxtype config unset <key>` |
| Model list per engine | `voxtype info models --json` |
| Which engines this build has | `voxtype info engines --json` |
| Input device list | `voxtype info devices --json` |
| Daemon liveness | `voxtype status --format json` |
| Unit state and main pid | `systemctl --user show voxtype -p ActiveState -p MainPID --value` |
| Whether the GPU is actually in use | `voxtype info accel --json` |
| GPU memory (NVIDIA) | `nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader` |
| GPU memory (AMD) | `rocm-smi --showmeminfo vram --json` |
| Restart the daemon | `systemctl --user restart voxtype` |
| Download a model | `voxtype setup --download --model <name> --progress-format json` (in panel) |
| Open the TUI | `voxtype configure` (floating terminal) |
| Edit the config file | `omarchy-launch-config-editor <config path>` |
| Install / update voxtype | `omarchy-voxtype-install` (floating terminal) |

## Development

Two things about this panel's environment are worth knowing before adding UI to
it:

- **Qt Quick Controls popups do not render here.** A `ToolTip` is a `Popup` and
  needs a window overlay, which a layer-shell surface does not provide, so the
  kit's own `Button` tooltips appear to be silently invisible in this panel. That
  is why dropdowns are routed through an explicit `overlayHost` item instead. A
  hover panel drawn as a plain `Rectangle` works fine — see `AccelBadge`.
- **A later sibling paints over an earlier one's children**, whatever `z` those
  children carry, because `z` only orders within one parent. Anything that hangs
  outside its own bounds — a hover panel below a header, say — needs a `z` on the
  component itself where it is used, not just on the overflowing child.

`components/VoxtypeCli.qml` owns every subprocess; the rest of the QML never
spawns anything. Two rules there are load-bearing:

- A download's `Process` lives in `VoxtypeCli`, never in the row whose button
  started it. Section switches, schema refetches, and dismissals all rebuild
  those rows, and a download has to outlive every one of them.
- Anything that opens its own window (the TUI, the installer, the config editor)
  is launched through `Panel.dismissThen`, which dismisses the panel first. This
  panel is a layer-shell surface on the overlay layer holding exclusive keyboard
  focus, so a terminal spawned while it is up opens *behind* it and never takes
  focus: the user clicks and sees nothing happen. That was the original reason
  model downloads moved into the panel. To drive a build that is not on `PATH`, drop a `dev.json` next
to `manifest.json`:

```json
{ "voxtypeBin": "/home/you/voxtype/target/release/voxtype" }
```

It is read synchronously at load, before the shell calls `open()`, so the first
schema fetch already uses it. The file is deliberately not committed.

`dev/sample-schema.json` is a fixture matching the `config schema --json`
contract — useful for eyeballing the layout, or as a stub:

```json
{ "voxtypeBin": "/path/to/fake-voxtype" }
```

where `fake-voxtype` is a script that `cat`s the fixture for `config schema
--json`.

Saving any file under `~/.config/omarchy/plugins/` hot-reloads plugin code, so
iterating is edit-and-resummon. `omarchy plugin validate ./omarchy-plugin`
checks the manifest against what the shell enforces.

Layout of the QML:

| File | Role |
|---|---|
| `BarWidget.qml` | native microphone icon opening the existing panel on Audio |
| `Panel.qml` | plugin entry point: state, the layer-shell window, IPC, CLI wiring |
| `components/SettingsCard.qml` | scrim + centered card + key catcher |
| `components/HeaderBar.qml` | title, accel badge, restart / TUI / edit config / close |
| `components/RuntimeFacts.qml` | the measured-facts line under the title |
| `components/AccelBadge.qml` | the GPU / CPU-fallback acceleration badge |
| `components/DownloadProgress.qml` | the in-row bar, percentage, and cancel |
| `components/SectionSidebar.qml` | search box + section list |
| `components/SchemaForm.qml` | the scrolling pane and what goes in it |
| `components/SettingDelegate.qml` | one form row, one control per schema type |
| `components/GpuIdlePresetRow.qml` | the "Release GPU memory when idle" preset |
| `components/EnginePicker.qml` | the pinned engine switch |
| `components/EngineModelCard.qml` | the model catalog for the engine in effect |
| `components/ReplacementsEditor.qml` | the `text.replacements` map |
| `components/RestartBanner.qml` | "changes take effect after restart" |
| `components/VoxtypeCli.qml` | every subprocess |

## Uninstall

```bash
./install/install-menu.sh --remove
omarchy plugin remove io.voxtype.settings
```

Or, for a hand-installed copy:

```bash
omarchy plugin disable io.voxtype.settings
rm -rf ~/.config/omarchy/plugins/io.voxtype.settings
```

Nothing else is left behind: the plugin stores no state of its own, and every
setting it changed lives in `~/.config/voxtype/config.toml`.
