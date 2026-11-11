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

## 2. Ask

Ask only what the output can't tell you:

1. Which languages do you dictate in?
2. On a laptop: is battery life or speed the priority?
3. Is sending audio to a paid cloud service ever acceptable? (Default: no.)
4. Do you want words to appear while you are still speaking (streaming), or
   only after you release the key?

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
