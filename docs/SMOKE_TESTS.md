# Smoke Tests

Run these tests after installing a new build to verify core functionality.
Each test lives in its own file under `docs/smoke_tests/` so you can read
and review one feature at a time without scrolling through the whole
catalogue.

For automated regression testing, use the `/regression-test` skill, which
covers unit tests, CLI commands, config validation, and binary variant
verification.

## Core

| Test | What it covers |
|------|----------------|
| [Basic Verification](smoke_tests/basic-verification.md) | `--version`, `--help`, daemon startup, and config defaults |
| [Recording Cycle](smoke_tests/recording-cycle.md) | Idle to Recording to Transcribing to Outputting state transitions |
| [Service Restart Cycle](smoke_tests/service-restart-cycle.md) | `systemctl --user restart voxtype` leaves the daemon in a healthy state |
| [Single Instance Enforcement](smoke_tests/single-instance-enforcement.md) | A second `voxtype daemon` invocation refuses to start |
| [Signal Handling](smoke_tests/signal-handling.md) | SIGTERM and SIGINT shut the daemon down cleanly |
| [Config Validation](smoke_tests/config-validation.md) | Invalid config produces a useful error, not a panic |

## CLI and file I/O

| Test | What it covers |
|------|----------------|
| [CLI Overrides](smoke_tests/cli-overrides.md) | `--model`, `--engine`, `--device`, and other per-invocation flags |
| [File Output](smoke_tests/file-output.md) | `--file`, append vs overwrite, config-based paths, directory creation |
| [Transcribe Command](smoke_tests/transcribe-command-file-input.md) | `voxtype transcribe <wav>` for offline file transcription |

## Recording behaviour

| Test | What it covers |
|------|----------------|
| [Smart Auto-Submit](smoke_tests/smart-auto-submit.md) | Auto-stop on silence, CLI/env/config layering, negative cases |
| [Recording Queue](smoke_tests/recording-queue.md) | FIFO queueing while previous batch recordings transcribe or output |
| [Rapid Successive Recordings](smoke_tests/rapid-successive-recordings.md) | Back-to-back recordings don't leak state |
| [Long Recording](smoke_tests/long-recording.md) | Multi-minute recordings transcribe cleanly without OOM |

## Voice activity detection

| Test | What it covers |
|------|----------------|
| [Voice Activity Detection](smoke_tests/voice-activity-detection.md) | Energy VAD, Whisper VAD, auto selection, threshold tuning, transcribe-command integration |

## Engines and models

| Test | What it covers |
|------|----------------|
| [Multi-Engine Transcription](smoke_tests/multi-engine-transcription.md) | Quick smoke per engine, daemon integration, error handling, perf comparison |
| [Model Switching](smoke_tests/model-switching.md) | Swap models at runtime without restarting the daemon |
| [Engine Switching via Model Selection](smoke_tests/engine-switching-via-model-selection.md) | Picking a model from another engine flips the engine automatically |
| [Multilingual Model Verification](smoke_tests/multilingual-model-verification.md) | Non-English models load and transcribe |
| [Invalid Model Rejection](smoke_tests/invalid-model-rejection.md) | Bogus model names fail loudly with a remediation hint |
| [Whisper CLI Backend](smoke_tests/whisper-cli-backend.md) | Out-of-process Whisper via subprocess |
| [Parakeet with Preloaded Model](smoke_tests/parakeet-with-preloaded-model.md) | Eager preload removes load latency on first recording |
| [Parakeet Backend Switching](smoke_tests/parakeet-backend-switching.md) | Switching between Parakeet variants at runtime |
| [Remote Transcription](smoke_tests/remote-transcription.md) | OpenAI-compatible HTTP backend |
| [On-Demand Model Loading](smoke_tests/on-demand-model-loading.md) | Lazy load reduces idle memory; load latency hides behind recording |
| [Eager Processing](smoke_tests/eager-processing.md) | Eager preload paths warm caches before the first recording |

## GPU

| Test | What it covers |
|------|----------------|
| [GPU Isolation Mode](smoke_tests/gpu-isolation-mode.md) | `gpu_isolation = true` releases VRAM after each transcription |
| [GPU Backend Switching](smoke_tests/gpu-backend-switching.md) | Switching between CUDA, Vulkan, and CPU at runtime |
| [Multi-GPU Selection](smoke_tests/multi-gpu-selection.md) | `CUDA_VISIBLE_DEVICES` and explicit device pinning |

## Output

| Test | What it covers |
|------|----------------|
| [Output Drivers](smoke_tests/output-drivers.md) | wtype, dotool, ydotool, clipboard selection |
| [dotool Fallback](smoke_tests/dotool-fallback.md) | dotool path probes and falls back when uinput is unavailable |
| [dotool Keyboard Layout](smoke_tests/dotool-keyboard-layout.md) | `DOTOOL_XKB_LAYOUT` handles non-US layouts |
| [ydotool Fallback](smoke_tests/ydotool-fallback.md) | ydotool path probes and falls back when the daemon is missing |
| [X11 Session Clipboard](smoke_tests/x11-session-clipboard-xclip-xsel.md) | xclip/xsel clipboard output on X11 |
| [Output Chain Verification](smoke_tests/output-chain-verification.md) | Full fallback chain wtype, dotool, ydotool, clipboard |
| [Delay Options](smoke_tests/delay-options.md) | `--delay` and per-driver inter-keystroke delays |
| [Post-Processing Command](smoke_tests/post-processing-command.md) | LLM cleanup command runs on the transcript before output |

## Integrations

| Test | What it covers |
|------|----------------|
| [Audio Feedback](smoke_tests/audio-feedback.md) | Start, stop, and completion cues play through the configured device |
| [Compositor Hooks](smoke_tests/compositor-hooks.md) | `voxtype record start/stop/toggle` from Hyprland, Sway, River |
| [Waybar JSON Output](smoke_tests/waybar-json-output.md) | `voxtype status --follow` emits valid JSON for Waybar |

## Meeting mode

| Test | What it covers |
|------|----------------|
| [Meeting Mode](smoke_tests/meeting-mode.md) | Lifecycle, list, show, export, delete, speaker labels, AI summary, error handling, dual audio, diarization backends |

## v0.7.6 release verification

Tests for bug fixes and tweaks introduced in v0.7.6.

| Test | Issue |
|------|-------|
| [TUI Audio/VAD float serialization](smoke_tests/v076/tui-audio-vad-float-serialization.md) | [#451](https://github.com/peteonrails/voxtype/issues/451) |
| [Engine vs binary mismatch detection](smoke_tests/v076/engine-binary-mismatch-detection.md) | [#450](https://github.com/peteonrails/voxtype/issues/450) |
| [Setup retry-hint](smoke_tests/v076/setup-retry-hint.md) | [#449](https://github.com/peteonrails/voxtype/issues/449) |
| [Streaming + non-streaming model fail-fast](smoke_tests/v076/streaming-model-fail-fast.md) | [#442](https://github.com/peteonrails/voxtype/issues/442) |
| [Model downloads via Cloudflare R2](smoke_tests/v076/models-cdn-r2.md) | (CDN rollout) |

## v0.6.6 release verification

Tests for bug fixes and enhancements introduced in v0.6.6.

| Test | Issue |
|------|-------|
| [Text Replacements with Spoken Punctuation](smoke_tests/v066/text-replacements-with-spoken-punctuation.md) | [#172](https://github.com/peteonrails/voxtype/issues/172) |
| [Remote Backend initial_prompt](smoke_tests/v066/remote-backend-initial-prompt.md) | [#278](https://github.com/peteonrails/voxtype/issues/278) |
| [Ydotool Socket Detection](smoke_tests/v066/ydotool-socket-detection.md) | [#306](https://github.com/peteonrails/voxtype/issues/306) |
| [Eitype in Paste Mode](smoke_tests/v066/eitype-in-paste-mode.md) | [#259](https://github.com/peteonrails/voxtype/issues/259) |
| [Duplicate Notification Fix](smoke_tests/v066/duplicate-notification-fix.md) | [#268](https://github.com/peteonrails/voxtype/issues/268) |
| [Xclip Clipboard Fallback on X11](smoke_tests/v066/xclip-clipboard-fallback-on-x11.md) | [#256](https://github.com/peteonrails/voxtype/issues/256) |
| [KDE Plasma Compositor Docs](smoke_tests/v066/kde-plasma-compositor-docs.md) | [#296](https://github.com/peteonrails/voxtype/issues/296) |
| [Audio Feedback on Transcription Completion](smoke_tests/v066/audio-feedback-on-transcription-completion.md) | [#258](https://github.com/peteonrails/voxtype/issues/258) |
| [MPRIS Media Player Pause](smoke_tests/v066/mpris-media-player-pause.md) | [#249](https://github.com/peteonrails/voxtype/issues/249) |
| [Post-Process trim and fallback_on_empty](smoke_tests/v066/post-process-trim-and-fallback-on-empty.md) | [#270](https://github.com/peteonrails/voxtype/issues/270) |

## Quick reference

| Test | What it covers |
|------|----------------|
| [Quick Smoke Test Script](smoke_tests/quick-smoke-test-script.md) | A compressed script that hits the most-broken-things-first paths |

## Adding a new test

Drop a new file under `docs/smoke_tests/` (or `docs/smoke_tests/<release>/`
for release-themed verification batches) and add a row to the appropriate
table above. Each file is its own document: open with an `# H1` heading,
then the test body. There's no enforced template, but most tests follow
the same shape: a one-line description, unit-test commands if there are
any, a structural grep with expected counts, then a runtime checklist
with "Expected:" lines.
