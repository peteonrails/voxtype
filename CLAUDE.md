# Claude Code Guidelines for Voxtype

This document helps Claude Code (and human contributors) understand the voxtype codebase, make good architectural decisions, and submit PRs that align with project standards.

## Table of Contents

- [Project Principles](#project-principles)
- [Architecture Overview](#architecture-overview)
- [Key Design Decisions](#key-design-decisions)
- [Code Style Guide](#code-style-guide)
- [Best Practices](#best-practices)
- [Roadmap](#roadmap)
- [Git Commits](#git-commits)
- [Version Bumping](#version-bumping)
- [Building Release Binaries](#building-release-binaries)
- [AUR Packages](#aur-packages)
- [Release Notes and Website News](#release-notes-and-website-news)
- [Website](#website)
- [Development Notes](#development-notes)
- [Smoke Tests](#smoke-tests)

---

## Project Principles

These principles guide all development decisions:

1. **Dead simple user experience** - Voxtype should just work. Installation, configuration, and daily use should be straightforward.

2. **Backwards compatibility** - Never break existing installations. Config changes must have sensible defaults that preserve current behavior.

3. **Performance first** - Prioritize speed and responsiveness. On desktops this means fast transcription; on laptops this means battery efficiency.

4. **Excellent CLI help** - The `--help` output is documentation. Every option should be clear, with examples where helpful.

5. **Every option configurable everywhere** - Any setting should be configurable via CLI flag, environment variable, or config file.

6. **Documentation in the right places** - User-facing changes go in the user manual, troubleshooting guide, and configuration guide as appropriate.

---

## Architecture Overview

Voxtype is a Linux-native push-to-talk voice-to-text daemon. The architecture follows a modular, trait-based design with async event handling.

### High-Level Flow

```
Hotkey (compositor/evdev) → Audio Capture (cpal) → Transcription (whisper-rs) → Text Processing → Output (wtype/ydotool/clipboard)
```

### Core Components

| Component | Location | Purpose |
|-----------|----------|---------|
| CLI | `src/cli.rs` | Clap command definitions, also used by `build.rs` for man pages |
| Config | `src/config.rs` | TOML parsing, defaults, icon themes (~900 lines) |
| Daemon | `src/daemon.rs` | Main event loop with `tokio::select!`, state coordination |
| State | `src/state.rs` | State machine: Idle → Recording → Transcribing → Outputting |
| CPU | `src/cpu.rs` | SIGILL handler, CPU feature detection |
| Error | `src/error.rs` | `thiserror` types with user-friendly messages |

### Module Structure

```
src/
├── hotkey/           # Keyboard input detection
│   ├── mod.rs        # HotkeyListener trait, factory
│   └── evdev_listener.rs  # Kernel-level via evdev (fallback for X11)
├── audio/            # Audio I/O
│   ├── mod.rs        # AudioCapture trait, factory
│   ├── cpal_capture.rs   # PipeWire/PulseAudio/ALSA via cpal
│   └── feedback.rs   # Audio playback for cues
├── transcribe/       # Speech-to-text
│   ├── mod.rs        # Transcriber trait, factory, prepare() optimization
│   ├── whisper.rs    # Local in-process via whisper-rs
│   ├── remote.rs     # HTTP API (OpenAI-compatible)
│   ├── subprocess.rs # GPU isolation wrapper
│   └── worker.rs     # Child process entry point
├── output/           # Text delivery
│   ├── mod.rs        # TextOutput trait, factory, fallback chain
│   ├── wtype.rs      # Wayland-native (best Unicode support)
│   ├── ydotool.rs    # X11/TTY fallback
│   ├── clipboard.rs  # Universal fallback via wl-copy
│   ├── paste.rs      # Clipboard + Ctrl+V
│   └── post_process.rs   # LLM cleanup command
├── text/             # Text transformations
│   └── mod.rs        # Spoken punctuation, replacements
└── setup/            # Installation helpers
    ├── model.rs      # Model selection & download
    ├── gpu.rs        # GPU feature detection
    ├── waybar.rs     # Waybar config snippets
    ├── systemd.rs    # Service installation
    └── compositor.rs # Hyprland/Sway/River keybinding setup
```

### Trait-Based Extensibility

Each major component defines a trait allowing multiple implementations:

| Trait | Implementations | Extension Point |
|-------|----------------|-----------------|
| `HotkeyListener` | `EvdevListener` | Add libinput, compositor-specific listeners |
| `AudioCapture` | `CpalCapture` | Add JACK, direct ALSA support |
| `Transcriber` | `WhisperTranscriber`, `RemoteTranscriber`, `SubprocessTranscriber` | Add new ASR backends |
| `TextOutput` | `WtypeOutput`, `YdotoolOutput`, `ClipboardOutput` | Add X11, compositor-specific output |

---

## Key Design Decisions

Understanding why things are built a certain way helps you extend them correctly.

### Async Runtime (Tokio)

**Why:** Push-to-talk requires responsive hotkey detection while handling long I/O operations.

**Pattern:**
- Main loop uses `tokio::select!` to multiplex hotkey events, signals, and task completion
- Audio capture uses mpsc channels to stream data without blocking
- Transcription runs via `spawn_blocking` to avoid blocking the event loop
- Model loading is a background task hidden behind recording time

### Hotkey Detection

**Preferred:** Compositor keybindings (Hyprland, Sway, River) - native integration, no special permissions needed. Voxtype provides `voxtype record start/stop/toggle` commands for compositor bindings to call.

**Fallback:** evdev listener - works on X11 and as a universal fallback. Requires user to be in `input` group.

Set `[hotkey] enabled = false` when using compositor keybindings.

### GPU Memory and Performance

**Priority:** Performance is critical. Fast transcription on desktops, battery efficiency on laptops.

**Trade-off:** GPU memory isn't released after in-process transcription, which causes memory growth over time. The `gpu_isolation = true` option spawns a child process that exits after transcription, releasing GPU memory.

**Guidance:** Don't assume users want GPU isolation by default. Some users prioritize keeping the model loaded for faster subsequent transcriptions. Let users choose based on their hardware and usage patterns.

### Output Fallback Chain

**Why:** No single output method works everywhere (wtype needs Wayland, ydotool needs daemon).

**Chain:** wtype → ydotool → clipboard

Each method is probed before use; failures cascade to next method.

### Configuration Layering

**Priority (highest wins):**
1. CLI arguments
2. Environment variables (`VOXTYPE_*`)
3. Config file (`~/.config/voxtype/config.toml`)
4. Built-in defaults

This allows overriding any setting at any level without modifying config files.

### CPU Compatibility via SIGILL Handler

**Why:** Binaries built on modern CPUs can contain instructions that crash on older CPUs.

**Solution:** Install SIGILL handler via `.init_array` constructor (runs before `main()`). If triggered, displays helpful message instead of silent crash.

---

## Code Style Guide

### Rust Conventions

- Run `cargo fmt` before committing
- Run `cargo clippy` and address warnings
- Use `cargo test` to verify changes

### Naming

| Item | Convention | Example |
|------|-----------|---------|
| Modules | snake_case | `audio_capture`, `post_process` |
| Types/Structs | PascalCase | `AudioCapture`, `TextProcessor` |
| Functions/Methods | snake_case | `create_transcriber`, `start_recording` |
| Config fields | snake_case in TOML | `on_demand_loading`, `max_duration_secs` |

### Error Handling

Use `thiserror` with user-friendly messages that include remediation steps:

```rust
#[error("Cannot open input device '{0}'. Is the user in the 'input' group?\n  Run: sudo usermod -aG input $USER")]
DeviceAccess(String),
```

Group related errors into domain-specific types:
- `VoxtypeError` - top-level
- `HotkeyError` - with group/key setup instructions
- `AudioError` - with device listing hints
- `TranscribeError` - with model download suggestions
- `OutputError` - with setup instructions for each method

### Logging

Use `tracing` (not `log`):

```rust
use tracing::{info, debug, warn, error};

info!("Starting daemon");
debug!(device = %device_name, "Opening audio device");
warn!("Model not found, downloading...");
error!(?err, "Transcription failed");
```

Worker processes log to stderr only (stdout reserved for IPC).

### Module Organization

- Keep trait definitions in `mod.rs`
- Put implementations in separate files
- Factory functions go in `mod.rs`
- Tests go at the bottom of each file in a `#[cfg(test)]` module

### Comments

- Prefer self-documenting code over comments
- Add comments for non-obvious "why" decisions
- Use `///` doc comments for public APIs
- Avoid TODO comments; open issues instead

---

## Best Practices

### Backwards Compatibility

**This is critical.** Never break existing installations.

- New config fields must have defaults that preserve current behavior
- Removed fields should be silently ignored, not cause errors
- CLI changes must not break existing scripts or keybindings
- Test upgrades by running the new version with an old config file

### Adding a New Transcription Backend

1. Create `src/transcribe/your_backend.rs`
2. Implement the `Transcriber` trait
3. Add variant to the factory in `src/transcribe/mod.rs`
4. Add configuration fields to `src/config.rs` with sensible defaults
5. Add CLI flags in `src/cli.rs` with clear `--help` text
6. Document in `docs/CONFIGURATION.md`
7. Add tests

### Adding a New Output Method

1. Create `src/output/your_method.rs`
2. Implement the `TextOutput` trait
3. Add to fallback chain in `src/output/mod.rs` if appropriate
4. Consider whether it should be a fallback or explicit selection

### Modifying Configuration

- Add new fields with sensible defaults (backward compatible)
- Update `src/config.rs` default values
- Add corresponding CLI flags in `src/cli.rs`
- Update `docs/CONFIGURATION.md`
- If the field affects behavior significantly, mention in release notes

### Documentation Requirements

When adding user-facing features, update:
- `docs/USER_MANUAL.md` - How to use the feature
- `docs/CONFIGURATION.md` - Config file options
- `docs/TROUBLESHOOTING.md` - If there are failure modes users might hit
- CLI `--help` text - Via clap attributes in `src/cli.rs`

### Testing Changes

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output visible
cargo test -- --nocapture

# Test a specific module
cargo test text::

# Manual testing
cargo run -- -vv  # Verbose daemon
cargo run -- transcribe test.wav  # Test transcription
cargo run -- status --follow  # Watch state changes
```

### Performance Considerations

- Avoid allocations in the hot path (hotkey detection, audio streaming)
- Use `spawn_blocking` for CPU-intensive work
- The `prepare()` method on `Transcriber` allows hiding model load time behind recording time
- Prefer streaming over buffering where possible
- On laptops, battery efficiency matters as much as raw speed

### Avoid Over-Engineering

- Don't add abstraction layers until there are multiple implementations
- Don't add configuration for edge cases; handle them with sensible defaults
- Three similar lines of code are better than a premature abstraction
- Only validate at system boundaries (user input, external APIs)

---

## Roadmap

### Packaging Priority

Expanding distribution support is a current focus:

1. **NixOS** - Next priority for packaging
2. **Manjaro** - Sway/Hyprland ecosystem support
3. **Other Sway/Hyprland distros** - Expand reach to tiling WM users

Existing packages: Arch (AUR: `voxtype`, `voxtype-bin`), Debian (.deb), Fedora (.rpm)

### Feature Roadmap

Based on open issues and project direction:

**Near Term:**
- **Multi-model support** ([#81](https://github.com/peteonrails/voxtype/issues/81)) - Load multiple Whisper models and switch between them
- **Multiple post-processing profiles** ([#79](https://github.com/peteonrails/voxtype/issues/79)) - Different LLM prompts for different contexts
- **KDE Wayland compatibility docs** ([#85](https://github.com/peteonrails/voxtype/issues/85)) - Document wtype alternatives for KDE
- **Deterministic integration tests** - Automated smoke tests using pre-recorded audio files that can run in CI without LLM/human interaction

**Medium Term:**
- **Audio caching** ([#28](https://github.com/peteonrails/voxtype/issues/28)) - Save recordings for replay/re-transcription
- **Eager input processing** ([#70](https://github.com/peteonrails/voxtype/issues/70)) - Start transcription while still recording

**Exploratory:**
- **Nemotron Speech backend** ([#47](https://github.com/peteonrails/voxtype/issues/47)) - Alternative ASR engine
- **Foreign exception handling** ([#30](https://github.com/peteonrails/voxtype/issues/30)) - Investigate whisper.cpp crash recovery

### Non-Goals

- Windows/macOS support (Linux-first, Wayland-native)
- GUI configuration (CLI and config file are the interface)
- Continuous dictation mode (push-to-talk is the paradigm)

---

## Git Commits

- **NEVER commit without GPG signing.** All commits must be signed. Do not use `--no-gpg-sign` or skip signing for any reason.
- **Pull requests with unsigned commits will be rejected.** Every commit in a PR must be signed.
- If GPG signing fails, stop and inform the user rather than bypassing signing.

## Version Bumping

**When bumping the version in Cargo.toml, ALWAYS update Cargo.lock before committing.**

The AUR source package (`voxtype`) uses `cargo fetch --locked` and `cargo build --frozen`, which require Cargo.lock to exactly match Cargo.toml. If the version in Cargo.lock doesn't match Cargo.toml, the build fails.

```bash
# Correct version bump process:
# 1. Edit Cargo.toml to set new version
# 2. Run cargo build to update Cargo.lock
cargo build
# 3. Verify Cargo.lock was updated
grep -A2 'name = "voxtype"' Cargo.lock  # Should show new version
# 4. Commit BOTH files together
git add Cargo.toml Cargo.lock
git commit -S -m "Bump version to X.Y.Z"
```

**Never commit a version bump to Cargo.toml without also committing the updated Cargo.lock.**

This caused the v0.4.6 incident where users building from source got:
```
error: the lock file Cargo.lock needs to be updated but --locked was passed to prevent this
```

## Building Release Binaries

### Why Docker Builds Matter

Building on modern CPUs (Zen 4, etc.) can leak AVX-512/GFNI instructions into binaries via system libstdc++, even with RUSTFLAGS set correctly. This causes SIGILL crashes on older CPUs (Zen 3, Haswell). Docker with Ubuntu 22.04 provides a clean toolchain without AVX-512 optimizations.

### Build Strategy

| Binary | Build Location | Why |
|--------|---------------|-----|
| AVX2 | Docker (Ubuntu 22.04) | Clean toolchain, no AVX-512 contamination |
| Vulkan | Docker on remote pre-AVX-512 server | GPU build on CPU without AVX-512 |
| AVX512 | Local machine | Requires AVX-512 capable host |

### GPU Feature Flags

GPU acceleration is enabled via Cargo features:

| Feature | Backend | Use Case |
|---------|---------|----------|
| `gpu-vulkan` | Vulkan | AMD GPUs, Intel GPUs, cross-platform |
| `gpu-cuda` | CUDA | NVIDIA GPUs |
| `gpu-hipblas` | ROCm/HIP | AMD GPUs (alternative to Vulkan) |
| `gpu-metal` | Metal | macOS (not applicable for Linux builds) |

```bash
# Build with Vulkan GPU support
cargo build --release --features gpu-vulkan

# Build with CUDA GPU support
cargo build --release --features gpu-cuda

# Build CPU-only (no GPU feature)
cargo build --release
```

### Remote Docker Context

A remote server with a pre-AVX-512 CPU is ideal for building binaries that must be clean of AVX-512 instructions. Configure a Docker context pointing to this server.

See `CLAUDE.local.md` for local infrastructure details (this file is gitignored).

```bash
# Switch to remote Docker context for AVX2/Vulkan builds
docker context use <your-remote-context>

# Build AVX2 and Vulkan binaries (safe, no AVX-512)
VERSION=0.4.3 docker compose -f docker-compose.build.yml up avx2 vulkan

# Switch back to local for AVX-512 build
docker context use default
```

### Full Release Build Process

**CRITICAL: Always use `--no-cache` for release builds to prevent stale binaries.**

Docker caches build layers aggressively. Without `--no-cache`, you may upload binaries with old version numbers even after updating Cargo.toml. This has caused AUR packages to ship v0.4.1 binaries labeled as v0.4.5.

```bash
# Set version
export VERSION=0.4.5

# 1. Build AVX2 + Vulkan on remote server (no AVX-512 contamination)
docker context use <your-remote-context>
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan
docker compose -f docker-compose.build.yml up avx2 vulkan

# 2. Copy binaries from containers
docker cp voxtype-avx2-1:/output/. releases/${VERSION}/
docker cp voxtype-vulkan-1:/output/. releases/${VERSION}/

# 3. Build AVX-512 locally (requires AVX-512 capable CPU)
docker context use default
cargo clean && cargo build --release
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512

# 4. VERIFY VERSIONS before uploading (critical!)
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx2 --version
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512 --version
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-vulkan --version

# 5. Validate instruction sets and package
./scripts/package.sh --skip-build ${VERSION}
```

### Version Verification Checklist

**Before uploading any release, verify ALL binaries report the correct version:**

```bash
# All three should print the same version matching $VERSION
releases/${VERSION}/voxtype-*-avx2 --version
releases/${VERSION}/voxtype-*-avx512 --version
releases/${VERSION}/voxtype-*-vulkan --version
```

If versions don't match, the Docker cache is stale. Rebuild with `--no-cache`.

### Validating Binaries (AVX-512 Detection)

Use `objdump` to verify binaries don't contain forbidden instructions:

```bash
# Check for AVX-512 instructions (should be 0 for AVX2/Vulkan builds)
objdump -d releases/0.4.3/voxtype-0.4.3-linux-x86_64-avx2 | grep -c zmm
objdump -d releases/0.4.3/voxtype-0.4.3-linux-x86_64-vulkan | grep -c zmm

# Check for GFNI instructions (should be 0 for AVX2/Vulkan builds)
objdump -d releases/0.4.3/voxtype-0.4.3-linux-x86_64-avx2 | grep -cE 'vgf2p8|gf2p8'

# Verify AVX-512 build DOES have AVX-512 (should be >0)
objdump -d releases/0.4.3/voxtype-0.4.3-linux-x86_64-avx512 | grep -c zmm
```

What to look for:
- `zmm` registers = 512-bit AVX-512 registers (forbidden in AVX2/Vulkan)
- `vpternlog`, `vpermt2`, `vpblendm` = AVX-512 specific instructions
- `{1to4}`, `{1to8}`, `{1to16}` = AVX-512 broadcast syntax
- `vgf2p8`, `gf2p8` = GFNI instructions (not on Zen 3)

### Packaging Deb and RPM

After binaries are built and validated:

```bash
# Full build + package (builds binaries if missing)
./scripts/package.sh 0.4.3

# Package only (use existing binaries)
./scripts/package.sh --skip-build 0.4.3

# Deb only
./scripts/package.sh --deb-only --skip-build 0.4.3

# RPM only
./scripts/package.sh --rpm-only --skip-build 0.4.3
```

Packages are output to `releases/${VERSION}/`:
- `voxtype_${VERSION}-1_amd64.deb`
- `voxtype-${VERSION}-1.x86_64.rpm`

Requirements: `fpm` (gem install fpm), `rpmbuild` for RPM

## AUR Packages

- AUR repos are nested git repos in `packaging/arch/` and `packaging/arch-bin/`
- These directories are ignored by the main repo (in `.gitignore`)
- To publish to AUR: `cd packaging/arch && git add -A && git commit -m "message" && git push`
- GPG signing key for AUR repos: `E79F5BAF8CD51A806AA27DBB7DA2709247D75BC6`

### AUR Versioning: pkgver vs pkgrel

**For the `voxtype-bin` package, always bump `pkgver`, never just `pkgrel` when binaries change.**

The binary download URLs include `pkgver` but not `pkgrel`:
```
https://github.com/peteonrails/voxtype/releases/download/v$pkgver/voxtype-$pkgver-linux-x86_64-avx2
```

When only `pkgrel` is bumped, the URL stays the same. AUR helpers like yay cache PKGBUILDs and see "same URL = same file," causing checksum failures when binaries have actually changed.

**When to use each:**

| Scenario | Action |
|----------|--------|
| New binary release | Bump `pkgver`, reset `pkgrel` to 1, create new GitHub release |
| Fix PKGBUILD only (deps, install script) | Bump `pkgrel` |
| Binaries were wrong/corrupted | **Release new version** (bump `pkgver`), don't try to fix in place |

**Never do this:**
- Re-upload different binaries to an existing GitHub release
- Bump only `pkgrel` when binary content has changed

This caused the v0.4.5 incident where users had cached PKGBUILDs with old checksums that didn't match re-uploaded binaries.

### Post-Install Message

When updating the AUR packages, also update the post-upgrade message in `packaging/arch-bin/voxtype-bin.install` to reflect the current release highlights.

The `post_upgrade()` function displays a message to users after they upgrade. This should summarize what's new in the version they just installed, not old releases.

```bash
# Check current message
cat packaging/arch-bin/voxtype-bin.install

# Update the post_upgrade() message with current version highlights
# Then commit with the PKGBUILD changes
```

## Release Notes and Website News

**Every GitHub release must have a corresponding news article on the website.**

When publishing a release to GitHub, also add a matching article to `website/news/index.html`. The content should mirror the GitHub release notes.

### Capturing All Features

Before writing release notes, review all commits since the last release to ensure nothing is missed:

```bash
git log --oneline v0.4.14..HEAD  # Replace with previous version tag
```

Check for:
- New features and configuration options
- Bug fixes
- Performance improvements
- Deprecations
- Contributors to credit

Don't just document the most recent work - capture everything that shipped since the last release.

### Style Guide (follow v0.4.10 and v0.4.11 as examples)

**Avoid AI writing patterns:**
- No em-dashes (—). Use regular dashes, colons, or separate sentences instead.
- No "delve", "leverage", "utilize", "streamline", "robust", "seamless"
- No excessive hedging ("It's worth noting that...", "Interestingly...")
- No formulaic transitions ("Let's dive in", "Without further ado")
- No punchy one-liner endings to paragraphs ("And that's the point.", "Simple as that.", "No thoughts, just vibes.")
- No sentence fragments for dramatic effect ("The result? Faster builds.", "The fix? Simple.")
- Write plainly and directly. The existing news posts are the voice to match.

**GitHub Release Notes (Markdown):**
- Version and headline in title: "v0.4.11: Remote Whisper, Cancel Transcription, Output Mode Override"
- Brief intro paragraph summarizing the release
- `###` sections for each major feature
- **"Why use it:"** callouts explaining the user benefit
- Code blocks with examples (config snippets, CLI commands)
- Bug fixes as a bullet list
- Downloads table and checksums at the end

**Website News Article (HTML):**
- Add new article at the top of the articles list in `website/news/index.html`
- Use the `id` attribute for anchor links (e.g., `id="v0411"`)
- `article-meta` with date and `<span class="article-tag">Release</span>`
- Same h2 title as GitHub release
- h3 subsections matching the GitHub structure
- **Why use it:** in `<strong>` tags
- Code blocks wrapped in `<div class="code-block">` with optional `<div class="code-header">` for labels

**Example structure:**
```html
<article class="news-article" id="v0412">
    <div class="article-meta">
        <time datetime="2026-01-15">January 15, 2026</time>
        <span class="article-tag">Release</span>
    </div>
    <h2>v0.4.12: Feature Summary Here</h2>
    <div class="article-body">
        <p>Intro paragraph...</p>

        <h3>Feature Name</h3>
        <p>Description of what it does.</p>
        <p><strong>Why use it:</strong> User benefit explanation.</p>

        <div class="code-block">
            <div class="code-header"><span>config.toml</span></div>
            <pre><code>[section]
option = "value"</code></pre>
        </div>
    </div>
</article>
```

**Checklist for releases:**
1. Create GitHub release with notes following the style above
2. Add matching article to `website/news/index.html`
3. Update `packaging/arch-bin/voxtype-bin.install` post_upgrade() message with current version highlights
4. Commit and push website changes
5. Push AUR package updates

## Website

The website at voxtype.io is hosted via GitHub Pages. It deploys automatically when changes to `website/` are merged to main. No separate deployment step is needed.

## Development Notes

### Killing the Daemon

When using `pkill voxtype` or manually killing the daemon, Waybar status followers (`voxtype status --follow`) will also be terminated. After restarting the daemon:

```bash
# Either reload Waybar entirely
pkill -SIGUSR2 waybar

# Or the followers will reconnect on next Waybar restart
```

The systemd unit restart (`systemctl --user restart voxtype`) handles this gracefully, but manual kills require Waybar attention.

### Binary Location Priority

The PATH typically has `~/.local/bin` before `/usr/local/bin`. When testing new builds:

```bash
# Check which binary is active
which voxtype

# Remove stale local copy if needed
rm ~/.local/bin/voxtype
hash -r  # Clear shell's command cache
```

## Smoke Tests

Run these tests after installing a new build to verify core functionality.

### Basic Verification

```bash
# Version and help
voxtype --version
voxtype --help
voxtype daemon --help
voxtype record --help
voxtype setup --help

# Show current config
voxtype config

# Check status
voxtype status
```

### Recording Cycle

```bash
# Basic record start/stop
voxtype record start
sleep 3
voxtype record stop

# Toggle mode
voxtype record toggle  # starts recording
sleep 3
voxtype record toggle  # stops and transcribes

# Cancel recording (should not transcribe)
voxtype record start
sleep 2
voxtype record cancel
# Verify no transcription in logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i transcri
```

### CLI Overrides

```bash
# Output mode override (use --clipboard, --type, or --paste)
voxtype record start --clipboard
sleep 2
voxtype record stop
# Verify clipboard has text: wl-paste

# Model override (requires model to be downloaded)
# Note: --model flag is on the main command, not record subcommand
voxtype --model base.en record start
sleep 2
voxtype record stop
```

### GPU Isolation Mode

Tests subprocess-based GPU memory release (for laptops with hybrid graphics):

```bash
# 1. Enable gpu_isolation in config.toml:
#    [whisper]
#    gpu_isolation = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for subprocess spawning:
journalctl --user -u voxtype --since "1 minute ago" | grep -i subprocess

# 5. Verify GPU memory is released after transcription:
#    (AMD) watch -n1 "cat /sys/class/drm/card*/device/mem_info_vram_used"
#    (NVIDIA) nvidia-smi
```

### On-Demand Model Loading

Tests loading model only when needed (reduces idle memory):

```bash
# 1. Enable on_demand_loading in config.toml:
#    [whisper]
#    on_demand_loading = true

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Check memory before recording (model not loaded):
systemctl --user status voxtype | grep Memory

# 4. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 5. Check logs for model load/unload:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Loading|Unloading"
```

### Model Switching

```bash
# Download a different model if not present
voxtype setup model  # Interactive selection

# Or specify directly
voxtype setup model small.en

# Test with different models (edit config.toml or use --model flag)
```

### Remote Transcription

```bash
# 1. Configure remote backend in config.toml:
#    [whisper]
#    backend = "remote"
#    remote_endpoint = "http://your-server:8080"

# 2. Restart and test
systemctl --user restart voxtype
voxtype record start && sleep 3 && voxtype record stop

# 3. Check logs for remote transcription:
journalctl --user -u voxtype --since "1 minute ago" | grep -i remote
```

### Output Drivers

```bash
# Test wtype (Wayland native)
# Should work by default on Wayland

# Test ydotool fallback (unset WAYLAND_DISPLAY or rename wtype)
sudo mv /usr/bin/wtype /usr/bin/wtype.bak
voxtype record start && sleep 2 && voxtype record stop
journalctl --user -u voxtype --since "30 seconds ago" | grep ydotool
sudo mv /usr/bin/wtype.bak /usr/bin/wtype

# Test clipboard mode
# Edit config.toml: mode = "clipboard"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
wl-paste  # Should show transcribed text

# Test paste mode
# Edit config.toml: mode = "paste"
systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop
```

### Delay Options

```bash
# Test type delays (edit config.toml):
#    type_delay_ms = 50       # Inter-keystroke delay
#    pre_type_delay_ms = 200  # Pre-typing delay

systemctl --user restart voxtype
voxtype record start && sleep 2 && voxtype record stop

# Check debug logs for delay application:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "delay|sleeping"
```

### Audio Feedback

```bash
# Enable audio feedback in config.toml:
#    [audio.feedback]
#    enabled = true
#    theme = "default"
#    volume = 0.5

systemctl --user restart voxtype
voxtype record start  # Should hear start beep
sleep 2
voxtype record stop   # Should hear stop beep
```

### Compositor Hooks

```bash
# Verify hooks run (check Hyprland submap changes):
voxtype record start
hyprctl submap  # Should show voxtype_recording
sleep 2
voxtype record stop
hyprctl submap  # Should show empty (reset)
```

### Transcribe Command (File Input)

```bash
# Transcribe a WAV file directly (useful for testing without mic)
voxtype transcribe /path/to/audio.wav

# With model override
voxtype transcribe --model large-v3-turbo /path/to/audio.wav
```

### Multilingual Model Verification

Tests that non-.en models load correctly and detect language:

```bash
# Use a multilingual model (without .en suffix)
voxtype --model small record start
sleep 3
voxtype record stop

# Check logs for language auto-detection:
journalctl --user -u voxtype --since "30 seconds ago" | grep "auto-detected language"

# Verify model menu shows multilingual options:
echo "0" | voxtype setup model  # Should show tiny, base, small, medium (multilingual)
```

### Invalid Model Rejection

Verify bad model names warn and fall back to default:

```bash
# Should warn, send notification, and fall back to default model
voxtype --model nonexistent record start
sleep 2
voxtype record cancel

# Expected behavior:
# 1. Warning logged: "Unknown model 'nonexistent', using default model 'base.en'"
# 2. Desktop notification via notify-send
# 3. Recording proceeds with the default model

# Check logs for warning:
journalctl --user -u voxtype --since "30 seconds ago" | grep -i "unknown model"

# The setup --set command should still reject invalid models:
voxtype setup model --set nonexistent
# Expected: error about model not installed
```

### GPU Backend Switching

Test transitions between CPU and Vulkan backends:

```bash
# Interactive GPU setup
voxtype setup gpu

# Check current backend in logs after restart:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "backend|Vulkan|CPU"
```

### Waybar JSON Output

Test the status follower with JSON format for Waybar integration:

```bash
# Should output JSON status updates (Ctrl+C to stop)
timeout 3 voxtype status --follow --format json || true

# Expected output format:
# {"text":"idle","class":"idle","tooltip":"Voxtype: idle"}

# Test during recording:
voxtype record start &
sleep 1
timeout 2 voxtype status --follow --format json || true
voxtype record cancel
```

### Single Instance Enforcement

Verify only one daemon can run at a time:

```bash
# With daemon already running via systemd, try starting another:
voxtype daemon
# Should fail with error about existing instance / PID lock

# Check PID file:
cat ~/.local/share/voxtype/voxtype.pid
ps aux | grep voxtype
```

### Post-Processing Command

Tests LLM cleanup if configured:

```bash
# 1. Configure post-processing in config.toml:
#    [output]
#    post_process_command = "your-llm-cleanup-script"

# 2. Restart daemon
systemctl --user restart voxtype

# 3. Record and transcribe
voxtype record start && sleep 3 && voxtype record stop

# 4. Check logs for post-processing:
journalctl --user -u voxtype --since "1 minute ago" | grep -i "post.process"
```

### Config Validation

Verify malformed config files produce clear errors:

```bash
# Backup current config
cp ~/.config/voxtype/config.toml ~/.config/voxtype/config.toml.bak

# Test with invalid TOML syntax
echo "invalid toml [[[" >> ~/.config/voxtype/config.toml
voxtype config  # Should show parse error with line number

# Test with unknown field (should warn but continue)
echo 'unknown_field = "value"' >> ~/.config/voxtype/config.toml
voxtype config

# Restore config
mv ~/.config/voxtype/config.toml.bak ~/.config/voxtype/config.toml
```

### Signal Handling

Test direct signal control of the daemon:

```bash
# Get daemon PID
DAEMON_PID=$(cat ~/.local/share/voxtype/voxtype.pid)

# Start recording via SIGUSR1
kill -USR1 $DAEMON_PID
voxtype status  # Should show "recording"
sleep 2

# Stop recording via SIGUSR2
kill -USR2 $DAEMON_PID
voxtype status  # Should show "transcribing" then "idle"

# Check logs:
journalctl --user -u voxtype --since "30 seconds ago" | grep -E "USR1|USR2|signal"
```

### Rapid Successive Recordings

Stress test with quick start/stop cycles:

```bash
# Run multiple quick recordings in succession
for i in {1..5}; do
    echo "Recording $i..."
    voxtype record start
    sleep 1
    voxtype record cancel
done

# Verify daemon is still healthy
voxtype status
journalctl --user -u voxtype --since "1 minute ago" | grep -iE "error|panic"
```

### Long Recording

Test recording near the max_duration_secs limit:

```bash
# Check current max duration
voxtype config | grep max_duration

# Start a long recording (default max is 60s)
# The daemon should auto-stop at the limit
voxtype record start
echo "Recording... will auto-stop at max_duration_secs"
# Wait or manually stop before limit:
sleep 10
voxtype record stop

# To test auto-cutoff, set max_duration_secs = 5 in config and record longer
```

### Service Restart Cycle

Test systemd service restarts:

```bash
# Multiple restart cycles
for i in {1..3}; do
    echo "Restart cycle $i..."
    systemctl --user restart voxtype
    sleep 2
    voxtype status
done

# Verify clean restarts in logs:
journalctl --user -u voxtype --since "1 minute ago" | grep -E "Starting|Ready|shutdown"
```

### Quick Smoke Test Script

```bash
#!/bin/bash
# quick-smoke-test.sh - Run after new build install

set -e
echo "=== Voxtype Smoke Tests ==="

echo -n "Version: "
voxtype --version

echo -n "Status: "
voxtype status

echo "Recording 3 seconds..."
voxtype record start
sleep 3
voxtype record stop
echo "Done."

echo ""
echo "Check logs:"
journalctl --user -u voxtype --since "30 seconds ago" --no-pager | tail -10
```
