---
name: voxtype-setup
description: Configure Voxtype (push-to-talk voice-to-text for Linux) for the best speed, accuracy and battery life on the user's own hardware. Use when the user wants to set up, tune or speed up Voxtype, pick a transcription engine or model, enable GPU acceleration, cut idle memory or battery drain, or fix slow or inaccurate dictation. Triggers on "set up voxtype", "configure voxtype", "make voxtype faster", "which voxtype model should I use", "voxtype GPU", "voxtype on my laptop", "voxtype battery".
---

# Configuring Voxtype for this machine

Voxtype runs many engines, models and binary variants. The right choice depends
on four things: the languages the user speaks, the GPU, whether it is a laptop
on battery, and how much memory there is. Work in this order: look, ask, plan,
confirm, change, verify.

## Ground rules

- **Look before you touch anything.** Every command in step 1 is read-only.
- **Show the plan and get a yes before changing anything.** Commands that need
  `sudo` (`setup gpu`, `setup onnx`, `setup npu`) swap system binaries, so name
  them explicitly when you ask.
- **Back up the config first:** `cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak`
- **Change config with `voxtype config set <key> <value>`**, not by editing the
  file. It type-checks the value, keeps comments, and rejects keys this binary
  doesn't support. `voxtype config schema` lists every key it accepts.
- **Don't guess flags.** If a command here fails, run it with `--help`. The
  `--help` output is Voxtype's documentation.
- **Never pick a cloud engine (Soniox, remote Whisper) without asking.** Every
  other engine keeps the audio on the machine.

## 1. Look

Run these and read the output:

```bash
voxtype --version
voxtype info variants          # CPU/GPU detected, installed variants, and a ★ recommendation
voxtype setup gpu --status     # active backend, every GPU, Vulkan runtime
voxtype info engines           # engines compiled into this binary, which one is active
voxtype info models            # downloadable and installed models per engine
voxtype config get --json      # current effective settings
voxtype setup migrate --json   # upgrade offers for the current model (GGUF, Parakeet Ultra)
voxtype setup check            # missing dependencies and permissions
```

And the hardware Voxtype doesn't report:

```bash
lscpu | grep -E '^(Model name|Core\(s\) per socket|Socket\(s\))'   # physical cores
free -h                                                                # memory
ls /sys/class/power_supply/ | grep -i '^BAT'                           # any output = laptop
```

The `★` lines in `voxtype info variants` are Voxtype's own recommendation for
this hardware. Start from them.

### Older installs are probably not optimal

Upgrading Voxtype never rewrites the user's `config.toml`, by design, so an
upgrade can't change how someone's dictation behaves. The catch is that a user
who installed an older version keeps that version's settings. Better defaults,
new engines, GPU paths and cleanup features reach new installs through the
config template, not existing users. Someone who has used Voxtype for a year
is probably missing things they would want.

So don't treat the existing config as a set of deliberate choices to protect.
Look for improvements and suggest them. Packages ship the current template at
`/etc/voxtype/config.toml`, and comparing against it shows what an older
config lacks:

```bash
voxtype --config /etc/voxtype/config.toml config get --json   # current template
voxtype config get --json                                     # this user
```

Most differences are personal choices: hotkey, engine, language, OSD style,
output mode. Leave those alone unless the user wants to revisit them. What is
worth raising:

- **Settings the template now turns on**, such as `vad.enabled` (skips
  transcribing silence and stops Whisper inventing text when the key is
  pressed by accident), `text.format_numbers` and `text.repair_sentence_breaks`
- **Model upgrade offers**: `voxtype setup migrate` lists them for the
  configured model
- **An OSD frontend pinned explicitly** (`gtk4` or `native`) when `auto`
  would pick a better one: `voxtype setup osd --status` says so
- **An ONNX engine or model** that has a Whisper or GGUF alternative, since
  2.0 removes ONNX Runtime
- **Settings tuned around an old bug or an old machine**, for example a
  thread count from a previous CPU, or a short `audio.max_duration_secs`

Present them as a short list: what each change does and why it helps. Apply
only the ones the user picks. If `/etc/voxtype/config.toml` is missing (a
manual or source install), compare against the recommendations in this skill
instead.

## 2. Ask

Ask only what the output can't tell you:

1. Which languages do you dictate in?
2. On a laptop: is battery life or speed the priority?
3. Is sending audio to a paid cloud service ever acceptable? (Default: no.)
4. Do you want words to appear while you are still speaking (streaming), or
   only after you release the key?
5. Where do you dictate most: terminals, chat apps or AI agents, editors,
   remote desktop sessions? Is your keyboard layout something other than US?

Questions 1 to 4 drive the hardware setup in steps 3 to 5. Question 5 drives
the output settings in step 6, which come after the basics work.

## 3. Plan

### Engine and model

| Situation | Engine and model |
|-----------|------------------|
| English, or one of the 25 European languages Parakeet covers | Parakeet in GGUF format. If `voxtype setup migrate` offers `parakeet-ultra` or `format-gguf`, take it, since that is the supported path forward |
| Any other language, or a mix that includes one | Whisper `large-v3-turbo` on a GPU, `small` on CPU only |
| English only, CPU only, and responsiveness matters most | Whisper `base.en` or `small.en` |
| Chinese, Japanese, Korean, Cantonese | Whisper `large-v3-turbo`. SenseVoice is smaller and faster, but it runs on ONNX Runtime (see below) |
| A language nothing else covers | Whisper, or Omnilingual (ONNX Runtime, see below) |
| Long recordings (meetings, files over a few minutes) | Parakeet. It splits long audio into windows. Whisper on very long files can get stuck repeating a sentence |

Voxtype 2.0 removes ONNX Runtime. Prefer Whisper and GGUF models (the `.gguf`
names in `voxtype info models`) over the ONNX model directories for anything you
set up now, so it keeps working after the upgrade. If an ONNX-only engine is the
best fit for the user's language, say so and let them choose.

### Acceleration

| GPU | What to do |
|-----|------------|
| AMD, Intel, or NVIDIA, for Whisper | `sudo voxtype setup gpu --enable`, which selects the Vulkan build |
| Any GPU, for a GGUF model | `voxtype config set <engine>.gguf_backend vulkan` (for example `parakeet.gguf_backend`). Only some installed variants ship a Vulkan-capable GGUF worker, so run `voxtype config validate-selection` afterwards and fall back to `auto` if it fails |
| No GPU | Stay on CPU, set threads to the physical core count (below) |
| Intel Core Ultra with an NPU | `voxtype setup npu --status`, then `--enable` if it reports a usable NPU |
| Two GPUs (an iGPU plus a discrete card) | `voxtype setup gpu --status` explains how to choose the device |

### CPU threads

Set threads to the **physical** core count from `lscpu` (cores per socket times
sockets), not the hyperthread count, for the engine in use:

```bash
voxtype config set whisper.threads 8      # or parakeet.threads, cohere.threads, gguf.threads
```

### Laptops and memory

On battery, or with little free RAM or VRAM, load the model only while
transcribing. Each recording then starts a little later, but nothing sits in
memory between dictations:

```bash
voxtype config set whisper.on_demand_loading true   # or <engine>.on_demand_loading
```

On a laptop with a discrete GPU using Whisper, also run
`voxtype config set whisper.gpu_isolation true`. Transcription then runs in a
subprocess that exits afterwards, so the discrete GPU can power down between
dictations. It costs almost nothing in latency. On a desktop, leave both off for the fastest
response.

### Hotkey

Compositor keybindings (Hyprland, Sway, River and others) need no special
permissions and are the recommended setup. Run
`voxtype setup compositor <name> --help` for the compositor in use. The
built-in evdev hotkey works everywhere but needs the user in the `input` group.

## 4. Change

After the user approves the plan:

1. Back up `config.toml`.
2. Download the model: `voxtype setup --download --model <name>`. Downloading
   does not select it.
3. Select it: `voxtype config set engine <engine>` and
   `voxtype config set <engine>.model <name>`, or accept a migrate offer with
   `voxtype setup migrate --accept <offer>`, which downloads, verifies and
   switches in one step.
4. Apply the acceleration, threads and memory settings from the plan.
5. Check the selection before restarting: `voxtype config validate-selection`
6. Restart the daemon: `systemctl --user restart voxtype`

## 5. Verify

1. Ask the user to dictate one sentence. Acceleration can't be confirmed until
   a model has loaded.
2. Run `voxtype info accel`. It should report the backend you planned.
3. Run `voxtype status --format json --extended` to confirm the engine and model.
4. If the daemon fails to start, read `journalctl --user -u voxtype -n 50`. To
   undo everything, restore `config.toml.bak` and restart.

Report what you changed, why each choice fits this machine, and the command to
undo it.

## 6. Beyond the basics

Once dictation works and is fast, go through these with the user. Each one is
optional. Explain the trade-off, recommend one, and let them choose. Every
`output.*` setting needs a daemon restart to take effect.

### How text reaches the cursor: type or paste

`voxtype config get output.mode` shows the current mode.

| Mode | What it does | Good for | Costs |
|------|--------------|----------|-------|
| `type` | Simulates keystrokes, character by character | Apps that block pasting, keeping the clipboard untouched, CJK text through `wtype` | Slower for long text. Can produce wrong characters on non-US layouts unless the driver knows the layout |
| `paste` | Copies to the clipboard, then sends the paste keystroke | Long dictations, non-US layouts (no layout mapping involved), apps where typing is slow | Overwrites the clipboard unless `output.restore_clipboard` is on. The paste keystroke has to match the app |
| `clipboard` | Copies only, the user pastes | Anything where typing into the focused window is unwanted | One extra keystroke every time |
| `file` | Appends or writes to `output.file_path` | Logging dictation somewhere instead of typing it | Nothing reaches the cursor |

Settings that matter for each:

- **Paste in terminals.** Many terminals don't paste on `ctrl+v`. Set
  `output.paste_keys` to `shift+insert` (works in terminals and most GUI apps)
  or `ctrl+shift+v`.
- **Keep the clipboard.** `voxtype config set output.restore_clipboard true`
  puts the previous clipboard back after pasting.
- **Typing on a non-US layout.** Set the layout for the driver in use, for
  example `voxtype config set output.dotool_xkb_layout de`. Without it, a German
  layout gets `y` and `z` swapped. Paste mode avoids the problem entirely.
- **Typing driver order.** `wtype` (wlroots compositors such as Hyprland and
  Sway, best Unicode), `eitype` (GNOME, KDE), `dotool`, `ydotool`. The order is
  `driver_order` under `[output]` in `config.toml`. For remote desktop clients
  such as `xfreerdp`, put `ydotool` first, because `wtype`'s keycodes don't
  match the remote session.
- **Chat apps and AI agents.** `output.shift_enter_newlines true` types line
  breaks as Shift+Enter so a multi-line dictation doesn't send early.
  `output.auto_submit true` presses Return after the text, which is handy when
  dictating prompts to an agent and risky everywhere else.

### The post-process command

`output.post_process.command` is a shell command that receives every finished
transcription on stdin. Whatever it prints on stdout is what gets typed. It is
Voxtype's general-purpose hook, and it is worth explaining to the user even if
they don't want it today, because it is usually the answer when they later ask
for something Voxtype doesn't do on its own:

- **Cleanup or rewriting with a local LLM**, for example
  `ollama run llama3.2:1b 'Clean up this transcription. Output only the cleaned text:'`
- **Translation** into another language before typing
- **Domain vocabulary**: fixing medical, legal or product terms the model gets
  wrong, with `sed` or a small script
- **Logging or side effects**: saving a copy of everything dictated, or sending
  it somewhere else as well (see history below)

What to tell the user about it:

- It adds latency. `sed` is instant, a local LLM takes seconds. Raise
  `output.post_process.timeout_ms` for an LLM (30000 to 60000).
- If the command fails, times out or prints nothing, the original
  transcription is typed instead, so a broken command never loses dictation.
- It does not run when streaming is enabled.
- Use instruct or chat models, not reasoning models, which print their
  thinking into the output.
- `[profiles.<name>]` sections can hold different commands, chosen by holding a
  modifier with the hotkey. `docs/CONFIGURATION.md` covers profiles.
- Check the built-in `[text]` options before reaching for an LLM:
  `text.filter_filler_words`, `text.spoken_punctuation` and
  `text.replacements.<spoken> <written>` handle the common cases with no latency.

### Dictation history (on and off)

Built-in transcript history is on the roadmap
([#209](https://github.com/peteonrails/voxtype/issues/209)). Check whether this
version already has it before doing anything else:

```bash
voxtype config schema | grep -i history
voxtype --help | grep -i history
```

If either shows a history setting or command, use that and skip the rest of
this section: read its `--help` or `voxtype config schema` entry to turn it on
and off, and tell the user where it stores transcripts.

Until then, the post-process command can keep a history. Tell the user it is a
stopgap that built-in history will replace, and that they can switch it off
once they upgrade. Ask first, and tell them plainly that the log will contain
everything they dictate, including anything sensitive.

To turn it on, install this script as `~/.local/bin/voxtype-history`:

```sh
#!/bin/sh
# Voxtype post-process command: log each transcription, pass it through unchanged.
log="${VOXTYPE_HISTORY_FILE:-$HOME/.local/share/voxtype/history.tsv}"
mkdir -p "$(dirname "$log")"
text=$(cat)
umask 077
printf '%s\t%s\n' "$(date -Iseconds)" "$(printf '%s' "$text" | tr '\t\n' '  ')" >> "$log"
printf '%s' "$text"
```

Then:

```bash
chmod +x ~/.local/bin/voxtype-history
voxtype config get output.post_process.command      # check for an existing command first
voxtype config set output.post_process.command "$HOME/.local/bin/voxtype-history"
systemctl --user restart voxtype
```

If a post-process command already exists, keep it and chain the logger after
it, for example `sed 's/uh, //g' | ~/.local/bin/voxtype-history`, so the log
records the text as typed.

Each line of `~/.local/share/voxtype/history.tsv` is a timestamp, a tab and
one dictation. The file is readable only by the user.

To turn it off, remove the command, or put back the user's previous command:

```bash
voxtype config unset output.post_process.command
systemctl --user restart voxtype
```

Turning it off keeps the existing log. Delete `history.tsv` only if the user asks.
