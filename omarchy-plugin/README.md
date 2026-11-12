# Voxtype: an Omarchy shell panel

A graphical settings panel for [voxtype](https://voxtype.io), running as a
panel plugin inside `omarchy-shell`. It renders whatever `voxtype config schema
--json` reports and writes changes back through `voxtype config set`, so the
panel, the CLI, and `voxtype configure` all agree about defaults and validation
without keeping a second copy of the schema.

Omarchy lists it as **Voxtype**: the same panel holds the settings and the
live meeting view (`omarchy-shell io.voxtype.settings open` or `meetings`).

### Why the ID is still `io.voxtype.settings`

The plugin's ID predates the meeting view, so it says "settings" while the
panel does more. It stays `io.voxtype.settings` on purpose: the ID is the
directory name the packages install to
(`/usr/share/voxtype/omarchy-plugin/io.voxtype.settings`), the link
`voxtype setup omarchy` and the daemon create in `~/.config/omarchy/plugins`,
the IPC target the tray and scripts call, and what omarchy-pkgs packages.
Renaming it would leave every installed link pointing at a directory that no
longer exists, and break keybindings and scripts that summon it. Only the
display name changed. A rename, if it happens, belongs in 2.0 with a migration:
`voxtype setup omarchy` would link the new ID, remove the old link, and keep an
IPC alias for the old target for a release.

## Requirements

- Omarchy 4 with the Quickshell shell (`omarchy-shell`)
- **voxtype 0.8 or newer.** The panel is built entirely on `voxtype config
  schema --json`, which earlier builds do not have. On an older voxtype the
  panel opens onto a card offering to install/update or to open the TUI
  instead. If voxtype cannot be run at all, the same card names the binary it
  tried (and the `dev.json` that named it, if one did) instead of showing the
  last answers it got.

## Install

**Packaged (recommended).** The voxtype deb, rpm and AUR packages install this
plugin at `/usr/share/voxtype/omarchy-plugin/io.voxtype.settings`. Link it into
Omarchy with:

```bash
voxtype setup omarchy
```

The daemon also links it on first run. The link follows the packaged copy, so
each voxtype upgrade brings the matching panel. Packages lay the QML out under a
versioned directory (`<version>/Panel.qml`), so Omarchy loads the new panel
after an upgrade instead of a cached one.

**From git (development).** This installs the flat tree as a copy that does not
follow package upgrades. Use it to work on the panel, with `dev.json` naming the
voxtype binary to drive (see Development). Only one copy should be installed at
a time. Remove a git copy with `omarchy plugin remove io.voxtype.settings`
before switching to the packaged one.

```bash
omarchy plugin add https://github.com/peteonrails/omarchy-voxtype-settings.git --enable --yes
```

Or by hand, from a checkout of this directory (the one holding
`manifest.json`). `-L` matters: `components/meetings` is a link to the meeting
view in `../quickshell/voxtype-meetings`, shared with the standalone
Quickshell shell, and has to be copied as files:

```bash
cp -rL . ~/.config/omarchy/plugins/io.voxtype.settings
omarchy-shell shell rescanPlugins
omarchy plugin enable io.voxtype.settings
```

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
| `omarchy-shell io.voxtype.settings section Hotkey` | open on a section, or jump to it if already open (re-reading everything, as opening does) |
| `omarchy-shell io.voxtype.settings meetings` | open the live meeting view (no-op if already open) |
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

## Meetings

The meeting view's QML lives in `quickshell/voxtype-meetings/`, shared with
voxtype's standalone Quickshell shell; `components/meetings` here is a link to
it, `scripts/stage-omarchy-plugin.sh` copies it into packages, and
`components/OmarchyMeetingsHost.qml` adapts it to this plugin (the shell's
theme, `shell.summon`, and "Change in Settings" opening the Meeting section).
The view uses only QtQuick and Quickshell, never `qs.Commons` or `qs.Ui`, so it
runs under both hosts.

Tabs: **Transcript**, **Notes** (an autosaving pad over `voxtype meeting notes
--set-file`, re-read with `notes --json` on `notes_updated`), **Ask** (the
meeting's saved asks from `show --json`, then new questions streamed from
`voxtype meeting ask --json`, with quick chips; the answer's `citations` become
links that jump into the transcript) and, once the meeting stops, **Summary**
(the MeetingSummary from `show --json`, `summary_ready` or `voxtype meeting
summarize --format json [--regenerate]`, rendered with chapter links; a
Summarizing state from the `summarizing` event that names the meeting). Each
tab keeps its scroll position. Ask and Summary carry an agent chip
(`backend_name` from `voxtype meeting assistant status --json`) with a menu of
the agents found, their notes (codex's bwrap jail), Local model only, and
Change in Settings. `blocked: "consent_required"` opens the consent dialog
(one button per usable agent when `choice_required`). Transcript lines get
"Add note" on hover (`voxtype meeting annotate`, note and/or `--highlight`),
and annotated or highlighted lines a margin marker; Edit (inline, `voxtype
meeting edit`) and Delete (with a five-second Undo before `voxtype meeting
delete-line` runs; a deletion arrives as `segment_deleted` or as
`segment_updated` with `"deleted": true`, and both are handled) sit beside it.
The header's More menu exports through `voxtype meeting export` into
`$XDG_DOCUMENTS_DIR/Voxtype` (default `~/Documents/Voxtype`) and opens folders
with `xdg-open`. A question asked while the
view is closed (`voxtype meeting view --ask last-5m --no-open`) runs with
`--notify`; with `[meeting.assistant] open_command = "voxtype meeting view"`,
the notification's Open writes `$XDG_RUNTIME_DIR/voxtype/meeting-view.request`
(meeting and answer from `VOXTYPE_MEETING_ID`/`VOXTYPE_ASK_ID`), which every
host reads, and the view opens on that answer.

The view follows a meeting live: title, elapsed time, state
(recording, paused, finalizing), Pause/Resume and Stop, a chip per speaker
(click to rename, which runs `voxtype meeting label <meeting id> <key> <name>`;
two chips renamed alike merge), and the transcript in start-time order with a
timestamp per line. It scrolls with new lines unless you have scrolled up. With
no meeting it offers Start meeting with an optional title.

Clicking the speaker on a line opens a small chooser (a plain item, not a
Popup) to move just that line: the meeting's speakers as chips, a "New
speaker…" field, and Restore original for a line already moved. A pick runs
`voxtype meeting reassign <meeting id> <segment id> <speaker> --json` (or
`--reset`); the line moves at once, takes the segment the CLI prints, and goes
back with the CLI's stderr in the error banner if it is refused.
`segment_updated` events (tagged `event`, or `type` as its contract was first
written) apply live, so a change made elsewhere shows up too. A line is marked
"moved from …" when the segment carries `original_speaker_id` or
`reassigned: true`, or it was moved from this view.

A meeting that ends is kept on screen as **Ended** (title, length, end time)
with renames and reassignment still working, until a new meeting starts, Back
returns to the list, or the view is closed. A daemon that goes away
mid-meeting leaves it Ended the same way.

With no meeting on screen the view lists past meetings from `voxtype meeting
list --json` (50, then more on request), newest first. That list carries
metadata only, so each row reads its own folder (`storage_path`) for the
speaker count (transcript.json) and the Summary badge (metadata.json's
`summary`, or summary.md), and only for rows actually built. Opening one runs
`voxtype meeting show <id> --json` and shows it as Ended; edits go to the
stored meeting.

```bash
omarchy-shell io.voxtype.settings meetings
omarchy-shell shell toggle io.voxtype.settings '{"view":"meetings"}'
```

It is not the modal Settings overlay: it is a card on the right edge, on the Top
layer, without a scrim, reserving no space, and it takes the keyboard only when
clicked, so the call keeps focus. Escape (once it has focus) or its close button
dismisses it. Both views are one plugin, so the shell's open/close bookkeeping
covers whichever is up, and a `shell toggle` of either closes the other.

**Opening on meeting start.** The plugin is `keepLoaded`, so
`components/meetings/MeetingFeed.qml` stays connected to
`$XDG_RUNTIME_DIR/voxtype/meeting.sock` whether or not the view is open. The
daemon announces every start there, whatever started it (`voxtype meeting
start`, the detection prompt's Record button, the tray), so nothing has to be
wired into those paths. On a start the plugin reads `voxtype config get
meeting.show_view_on_start` (default `true`; a CLI without the key counts as
`true`) and, if it is on and the plugin is not already open, summons itself with
`{"view":"meetings"}`. A meeting already running when the shell starts is not a
start and opens nothing. An open Settings panel is never replaced.

While the daemon is away the feed retries with backoff (1, 2, 4, then every 8
seconds; each failed attempt logs one socket warning in the shell's log), and
opening the view retries at once. A drop or a daemon restart re-reads the
`hello` snapshot, so the transcript is rebuilt rather than duplicated.

Not there yet, because `meeting.sock` does not carry them: per-track level
meters and a countdown to the next chunk. The view says "Listening…" and how
long ago the last lines arrived instead. The title is not editable: no CLI
renames a running meeting.

**Seeing it after an install.** The shell does not reload a `keepLoaded`
plugin's QML when its files change, and a plugin rescan keeps the old instance.
After installing a build that adds or changes this view, restart the shell once
(`omarchy-restart-shell`) in an unlocked session. A packaged upgrade whose
manifest version changed loads the new versioned path on its own, but a test
build that kept the old version, or a git copy, keeps serving the old QML until
the shell restarts. The dictation daemon does not need a restart for a
panel-only change, and a meeting in progress is not affected: the restarted
plugin reconnects and reads the transcript so far from the `hello` snapshot.

## Text Cleanup

Open it with `omarchy-shell io.voxtype.settings section 'Text Cleanup'`.
The legacy `section Text` IPC request remains an alias. The canonical section
label comes from the executable's schema; older schemas labeled Text are displayed
as Text Cleanup without adding unsupported keys. A schema without
`text.lard_disfluency` cannot offer the model opt-in switch: update to a compatible
client and plugin together. A daemon restart alone does not add CLI schema keys.


The **Text Cleanup** section groups **Clean up text** as independent deterministic
switches: spoken punctuation, filler removal, restart collapse, sentence-break
repair, and number formatting. It preserves each stage's schema value; opening
the section does not enable anything. **LARD disfluency cleanup** is a separate
opt-in switch over `text.lard_disfluency`, with its own local-model path and
CC BY-SA 4.0 license link. It requires explicit English and separate model setup
(`voxtype setup lard --source /path/to/model.vxdf`). Missing or invalid models
leave deterministic cleanup available. A click on the license opens the browser;
there are no automatic network requests or downloads. Older schemas only show
the keys they actually support. Learned punctuation/casing restoration and
smart auto-submit remain under **Other text options**.

Regression test (Quickshell and a Wayland session required):

```bash
python3 omarchy-plugin/tests/test-cleanup-controls.py \
  --bin /path/to/new/voxtype --old-bin /path/to/older/voxtype \
  --evidence /path/to/evidence
```

The test drives the real panel's keyboard activation through SettingDelegate,
SchemaForm, and VoxtypeCli into a disposable config, then re-fetches the schema
and verifies independent toggles, exact writes, sibling values, the model
license affordance, and older-schema behavior. It never writes your live config
or restarts the daemon.

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
- Keys the schema flags `onnx_model_only` (today Parakeet's **Decoder type**) are
  hidden while the engine's model is a `.gguf` file, which carries its own
  architecture. A voxtype that predates the flag shows the key as before.
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
- A model whose catalog entry carries `license_url` (from `voxtype info models
  --json`, alongside an optional `license` name) shows a small "License: <name>"
  link under its row that opens the page in your browser. It is information
  only; nothing asks you to accept it. Entries without a URL, and older builds
  without the fields, show no link.
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
- When a better model is available for the one you run, the Engine section
  opens with a **model update** banner: each offer with its download size and
  an **Update** button (Parakeet Ultra first when it is the recommended one),
  plus **Keep current model**. Update runs `voxtype setup migrate --accept <id>
  --yes` with the same in-panel progress bar, cancel and inline error as a
  model Download; a failed or cancelled update leaves the current model in
  place and the button offers **Retry**. On success the CLI switches the model
  and restarts the daemon itself, so no restart banner follows. Keep current
  model records the decline through `voxtype setup migrate --decline` and then
  shows the 2.0 removal notice and the pin command for your install as
  selectable text (nothing is copied to the clipboard for you). The banner
  stays while offers exist, declined or not, so a kept model can still be
  updated later. After an update, **Remove old model files** runs `voxtype
  setup migrate --cleanup`. The offers come from `voxtype setup migrate
  --json`; a voxtype without that command shows no banner. The tray's "Model
  update available..." item opens the panel here.
- Voxtype reads its config at startup, so a banner appears after the first
  successful write and stays until a restart succeeds. A restart made outside
  the panel (`systemctl --user restart voxtype`, `voxtype setup model`) counts:
  the panel notes the daemon's main pid when the banner goes up and clears the
  banner once the unit reports a different one, on the next status poll or the
  next summon.
- When the OSD is set to GTK4 or native and the Quickshell OSD is available,
  a banner at the top of the OSD section (and of the Engine section, where model
  updates are offered) offers to **Switch**. Switch runs `voxtype setup osd
  --use-auto` and then the panel's own restart, with its usual validation and
  error banner. **Not now** runs `voxtype setup osd --dismiss-offer`; if that
  cannot be recorded the banner stays hidden until the shell restarts. Whether
  to offer is the CLI's answer (`offer` in `voxtype setup osd --status
  --json`); a voxtype without `--status` shows no banner.
- The OSD **Style** row says which frontend its styles belong to (the Quickshell
  OSD) and which OSD is in use. Picking a style on another frontend is never a
  silent no-op: with Quickshell available the panel also sets `osd.frontend` to
  `auto` and says so under the row; without it nothing is written, the picker
  goes back to the configured style, and the row says styles need the Quickshell
  OSD. A voxtype without `setup osd --status` writes the style as before.
- A number setting with no value in the config (Parakeet's **Threads**, for
  one) shows an empty field marked "default" rather than the spin box's
  minimum; the row's help states what the default is. Nothing is written until
  you enter a number, and **Reset to default** returns it to that state.
- If a voxtype command cannot be started at all (voxtype is not installed, or a
  `dev.json` names a binary that is gone), the panel says so: "The Voxtype panel
  could not run `<binary>`", plus the `dev.json` path when that is where the
  binary came from. It also drops everything it was showing from earlier
  answers (version, configured model, model list, daemon state, upgrade offers)
  rather than presenting them as current. The next summon or `section` call
  tries again, and the settings come back as soon as the binary answers.
- If `voxtype info engines`, `info devices` or `info styles` runs but fails, a
  red strip above the form names the command and its error, and that list is
  emptied. The rest of the panel keeps working.
- If this panel is a copy you installed by hand (with `omarchy plugin add` or
  `cp -r`) and your voxtype package ships its own, a banner offers to **Switch**
  to the packaged one, which updates with voxtype. The button runs `voxtype
  setup omarchy --replace --yes --restart-shell` in a terminal. A voxtype
  without `setup omarchy` shows no banner. **Not now** records the choice with
  `voxtype setup omarchy --dismiss-copy` so later opens do not ask again; if
  recording fails, the banner stays hidden until the shell restarts.
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
| Set a secret from the schema's `secrets` list (the calendar's iCal link) | its `set_command` (`voxtype meeting calendar set-ics - --json`), with the value on stdin, never in argv |
| Clear a secret, and read its status | its `clear_command` and `status_command` (`voxtype meeting calendar clear --json`, `status --json`) |
| Clear an override | `voxtype config unset <key>` |
| Model list per engine | `voxtype info models --json` |
| Which engines this build has | `voxtype info engines --json` |
| Input device list | `voxtype info devices --json` |
| OSD style list | `voxtype info styles --json` |
| Daemon liveness | `voxtype status --format json` |
| Unit state and main pid | `systemctl --user show voxtype -p ActiveState -p MainPID --value` |
| Whether the GPU is actually in use | `voxtype info accel --json` |
| GPU memory (NVIDIA) | `nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader` |
| GPU memory (AMD) | `rocm-smi --showmeminfo vram --json` |
| Restart the daemon | `systemctl --user restart voxtype` |
| Download a model | `voxtype setup --download --model <name> --progress-format json` (in panel) |
| Model upgrade offers | `voxtype setup migrate --json` |
| Accept / keep / clean up | `voxtype setup migrate --accept <id> --yes --progress-format json`, `--decline`, `--cleanup` (in panel) |
| Open the TUI | `voxtype configure` (floating terminal) |
| Edit the config file | `omarchy-launch-config-editor <config path>` |
| Install / update voxtype | `omarchy-install-dictation-voxtype`, or `omarchy-voxtype-install` on an Omarchy that predates it (floating terminal) |
| Is this the packaged copy of the panel | `voxtype setup omarchy --status --json` |
| Switch to the packaged copy | `voxtype setup omarchy --replace --yes --restart-shell` (floating terminal) |
| Not now on that banner | `voxtype setup omarchy --dismiss-copy` |
| OSD frontend, and whether to offer Quickshell | `voxtype setup osd --status --json` |
| Switch on the OSD offer | `voxtype setup osd --use-auto`, then the restart above |
| Not now on the OSD offer | `voxtype setup osd --dismiss-offer` |

## Development

### Model upgrade banner test

```bash
python3 omarchy-plugin/tests/test-upgrade-offers.py [--evidence DIR]
```

Runs the real panel offscreen (`QT_QPA_PLATFORM=offscreen`; the test copy
swaps the layer-shell window for a plain one) against
`tests/fake-voxtype-migrate.py`, so it needs Quickshell and the Omarchy shell
but no compositor, daemon, config or network. It refuses to run with
`WAYLAND_DISPLAY` or `DISPLAY` set; run it in a desktop-isolated sandbox. The
`link*` scenarios cover the "Switch to the packaged copy" banner.

### OSD offer, style picker, restart banner and unset number tests

```bash
python3 omarchy-plugin/tests/test-osd-panel.py [--evidence DIR] [--scenario NAME]
```

Same harness and isolation rules. The fake CLI answers `setup osd --status
--json`, `--use-auto` and `--dismiss-offer`, and `tests/fake-systemctl.py` is
put first on `PATH` as `systemctl`, so the unit's main pid and the panel's own
restart are under the test's control. Covers the offer (shown, hidden without
Quickshell, hidden once dismissed, hidden for an older CLI, Switch, a failed
Switch, Not now, a failed Not now), the Style row with and without Quickshell,
the restart banner clearing when the pid changes (on the poll and on reopen,
and not before), and an unset Threads row.

### CLI failure and section refresh tests

```bash
python3 omarchy-plugin/tests/test-cli-missing.py [--evidence DIR]
python3 omarchy-plugin/tests/test-section-refresh.py [--evidence DIR]
python3 omarchy-plugin/tests/test-onnx-model-only.py [--evidence DIR]
```

Same harness (`tests/panel_harness.py`) and the same isolation rules as the
upgrade banner test. `test-cli-missing.py` points `dev.json` at a binary that
does not exist, removes the binary between two summons, empties `PATH`, and
makes the `info` readers fail; it checks the error names the binary and that
nothing stale stays on screen. `test-section-refresh.py` changes the fake CLI's
answers behind an open panel and checks that `section` re-reads all of them.
`test-onnx-model-only.py` checks Decoder type is hidden for a GGUF model, shown
for an ONNX one, and shown by a schema without the flag.
Terminal launches are logged by a stand-in, never opened.

### Meeting view tests

```bash
python3 omarchy-plugin/tests/test-meeting-view.py [--evidence DIR] [--scenario NAME]
```

Same harness and isolation rules as the tests above, plus a private
`XDG_RUNTIME_DIR`. `tests/fake-meeting-sock.py` binds `meeting.sock` there and
speaks the daemon's NDJSON contract (a `hello` per connection, `segments`,
`speaker_renamed`, `status`, `error`), driven through a control file;
`tests/fake-voxtype-meeting.py` stands in for `voxtype meeting …` and
`voxtype config get meeting.show_view_on_start`, and tells the fake socket what
the daemon would do. The plugin's `shell` is a stand-in that records summons.
Scenarios cover the hello backlog, insertion by start time, speaker renames,
chip rename and merge, moving one line (to a speaker, to a new one, a refusal
rolled back, Restore original, `segment_updated` from elsewhere), editing after
the meeting ends, pause/resume/stop through finalizing, opening on a start
(on, off, an older CLI, an open Settings panel), reconnecting after a drop, a
socket that goes away and a daemon restart, the idle state and a refused start,
no daemon at all, and auto-scroll. `--evidence` also saves a screenshot of the
view in each state.

### Calendar link test

```bash
python3 omarchy-plugin/tests/test-calendar-link.py [--evidence DIR] [--scenario NAME]
REAL_VOXTYPE=target/debug/voxtype python3 omarchy-plugin/tests/test-calendar-link.py --scenario real
```

Same harness and isolation rules as the tests above. The schema's `secrets`
entry for the calendar's iCal link, shown as a row on the Meeting section
above the calendar keys: setting a link (a refused one stays in the field,
a good one empties it and the status line takes over), a 404 from the
server, a saved link whose last fetch failed, Clear, the keyboard cursor and
search, and a schema without `secrets`, which shows no row. Every scenario checks that the
link reached `set-ics -` on stdin and never appeared in argv or in the shell's
log. `real` (run only when `REAL_VOXTYPE` is set) hands `config schema` and
`meeting calendar` to that binary, which fetches a feed from a local HTTP
server.

### Schema boundary regression test

Run `python3 omarchy-plugin/tests/test-schema-boundary.py` from the repository
root with Quickshell, an Omarchy shell checkout, and a Wayland session available.
Use `--shell /path/to/omarchy/shell` for a nonstandard checkout. A headless
compositor may provide the Wayland session in automation.

The test instantiates the actual panel without opening it or changing settings.
It passes schema JSON through a QML `signal(var)` and checks section discovery,
engine choices, values, engine filtering, schema reloads, and empty/null keys.
This must run in QML: a JavaScript-only test misses Qt sequence wrappers that
retain length and indexed access but fail `Array.isArray`. JSON validation stays
in `VoxtypeCli` before the signal; consumers must accept the wrapped sequences.

After installing a panel change, verify it in the running shell. If a plugin
rescan still serves cached QML, `omarchy-restart-shell` loads the new code without
restarting the Voxtype daemon.

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

`components/VoxtypeCli.qml` owns every subprocess of the Settings view, and
`components/meetings/MeetingCli.qml` every one of the Meetings view; the rest
of the QML never spawns anything. Three rules there are load-bearing:

- Every subprocess is a `CliProcess`, started with `launch()`. Quickshell's
  `Process` emits neither `started` nor `exited` for a command that cannot
  start, so an `onExited` handler alone never learns about a missing binary.
  `CliProcess` turns that into `failedToStart()`, and each voxtype call reports
  it as `cliUnavailable`.
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

It is read synchronously when a command first resolves its executable, including
an `open()` queued before the host injects the manifest. Missing or malformed
files fall back to `voxtype` on `PATH`. The file is deliberately not committed.
For an installation that uses a launcher wrapper, point `voxtypeBin` at that
wrapper, not its resolved executable: the wrapper may establish model-cache or
runtime environment variables needed by its backend.

`dev/sample-schema.json` is a fixture matching the `config schema --json`
contract — useful for eyeballing the layout, or as a stub:

```json
{ "voxtypeBin": "/path/to/fake-voxtype" }
```

where `fake-voxtype` is a script that `cat`s the fixture for `config schema
--json`.

Saving a file under `~/.config/omarchy/plugins/` requests a plugin hot reload.
A rescan alone retains `keepLoaded` instances, and Qt can retain compiled QML
even across a full plugin reload. If the old code remains after resummoning,
restart the shell in an unlocked session; do not restart the dictation daemon
for a panel-only change. `omarchy plugin validate ./omarchy-plugin` checks the
manifest against what the shell enforces.

Layout of the QML:

| File | Role |
|---|---|
| `Panel.qml` | plugin entry point: state, the layer-shell window, IPC, CLI wiring |
| `components/SettingsCard.qml` | scrim + centered card + key catcher |
| `components/HeaderBar.qml` | title, accel badge, restart / TUI / edit config / close |
| `components/RuntimeFacts.qml` | the measured-facts line under the title |
| `components/AccelBadge.qml` | the GPU / CPU-fallback acceleration badge |
| `components/DownloadProgress.qml` | the in-row bar, percentage, and cancel |
| `components/UpgradeOfferBanner.qml` | model upgrade offers at the top of the Engine section |
| `components/SectionSidebar.qml` | search box + section list |
| `components/SchemaForm.qml` | the scrolling pane and what goes in it |
| `components/SettingDelegate.qml` | one form row, one control per schema type |
| `components/GpuIdlePresetRow.qml` | the "Release GPU memory when idle" preset |
| `components/EnginePicker.qml` | the pinned engine switch |
| `components/EngineModelCard.qml` | the model catalog for the engine in effect |
| `components/ReplacementsEditor.qml` | the `text.replacements` map |
| `components/RestartBanner.qml` | "changes take effect after restart" |
| `components/CliErrorBanner.qml` | an `info` reader that ran and failed |
| `components/PluginCopyBanner.qml` | "switch to the packaged copy" |
| `components/OsdOfferBanner.qml` | "switch to the Quickshell OSD" |
| `components/VoxtypeCli.qml` | every subprocess |
| `components/CliProcess.qml` | a `Process` that also reports a command that never started |
| `components/meetings/Meetings.qml` | the Meetings view: feed, commands, window, opening on a start |
| `components/meetings/MeetingFeed.qml` | the `meeting.sock` connection and the live meeting model |
| `components/meetings/MeetingCli.qml` | `voxtype meeting …` and the `show_view_on_start` read |
| `components/meetings/MeetingWindow.qml` | the non-modal layer-shell card the view lives in |
| `components/meetings/MeetingView.qml` | header, speaker chips, transcript, idle state |

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
