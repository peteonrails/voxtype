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
│   ├── dotool.rs     # Keyboard layout support via uinput
│   ├── ydotool.rs    # X11/TTY fallback (requires daemon)
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
| `TextOutput` | `WtypeOutput`, `DotoolOutput`, `YdotoolOutput`, `ClipboardOutput` | Add X11, compositor-specific output |

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

**Why:** No single output method works everywhere (wtype needs Wayland, ydotool needs daemon, dotool needs uinput access).

**Chain:** wtype → dotool → ydotool → clipboard

- **wtype**: Wayland-native, best Unicode/CJK support, no daemon needed
- **dotool**: Works on X11/Wayland/TTY, supports keyboard layouts via `DOTOOL_XKB_LAYOUT`, no daemon needed
- **ydotool**: Works on X11/Wayland/TTY, requires ydotoold daemon
- **clipboard**: Universal fallback via wl-copy

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

### Crediting Contributors

When work builds on contributions from others, always include appropriate credit:

- **Use `Co-authored-by:` trailers** for commits that incorporate someone else's work, even if substantially modified
- **When in doubt, give credit.** It's better to over-attribute than to omit someone's contribution
- **Credit applies broadly:** code, ideas, bug reports, design feedback, and review comments all warrant acknowledgment
- **Check PR and issue history** to identify contributors whose work influenced the commit

Examples of when to add co-author credit:
- Cherry-picking or rebasing commits from a PR (even if you resolve conflicts or make changes)
- Implementing a feature based on someone's detailed issue or design proposal
- Fixing a bug that someone else identified and diagnosed
- Incorporating code snippets or approaches suggested in review comments

Format:
```
Co-authored-by: Name <email@example.com>
```

Multiple co-authors are fine when several people contributed to the work.

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

**Whisper Binaries:**

| Binary | Build Location | Why |
|--------|---------------|-----|
| AVX2 | Docker on remote pre-AVX-512 server | Clean toolchain, no AVX-512 contamination |
| Vulkan | Docker on remote pre-AVX-512 server | GPU build on CPU without AVX-512 |
| AVX512 | Local machine | Requires AVX-512 capable host |

**Parakeet Binaries (Experimental):**

| Binary | Build Location | Why |
|--------|---------------|-----|
| parakeet-avx2 | Docker on remote pre-AVX-512 server | Wide CPU compatibility |
| parakeet-avx512 | Local machine | Best CPU performance |
| parakeet-cuda | Docker on remote server with NVIDIA GPU | GPU acceleration |

Note: Parakeet binaries include bundled ONNX Runtime which contains AVX-512 instructions, but ONNX Runtime uses runtime CPU detection and falls back gracefully on older CPUs.

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
export VERSION=0.5.0

# 1. Build Whisper binaries (AVX2 + Vulkan) on remote server
docker context use <your-remote-context>
docker compose -f docker-compose.build.yml build --no-cache avx2 vulkan
docker compose -f docker-compose.build.yml up avx2 vulkan

# 2. Build Parakeet binaries on remote server
docker compose -f docker-compose.build.yml build --no-cache parakeet-avx2
docker compose -f docker-compose.build.yml up parakeet-avx2

# 3. Copy binaries from remote Docker volumes to local
mkdir -p releases/${VERSION}
docker run --rm -v $(pwd)/releases/${VERSION}:/test ubuntu:24.04 ls /test  # verify
# Use tar pipe to copy from remote Docker volume:
docker run --rm -v $(pwd)/releases/${VERSION}:/src ubuntu:24.04 tar -cf - -C /src . | tar -xf - -C releases/${VERSION}/

# 4. Build AVX-512 binaries locally (requires AVX-512 capable CPU)
docker context use default

# Whisper AVX-512
cargo clean && cargo build --release
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512

# Parakeet AVX-512
cargo clean && RUSTFLAGS="-C target-cpu=native" cargo build --release --features parakeet
cp target/release/voxtype releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-parakeet-avx512

# 5. VERIFY VERSIONS before uploading (critical!)
for bin in releases/${VERSION}/voxtype-*; do
  echo -n "$(basename $bin): "; $bin --version
done

# 6. Validate instruction sets and package
./scripts/package.sh --skip-build ${VERSION}
```

### Version Verification Checklist

**Before uploading any release, verify ALL binaries report the correct version:**

```bash
# Whisper binaries
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx2 --version
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-avx512 --version
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-vulkan --version

# Parakeet binaries (experimental)
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-parakeet-avx2 --version
releases/${VERSION}/voxtype-${VERSION}-linux-x86_64-parakeet-avx512 --version
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

### Parakeet Binary Instruction Leakage

**IMPORTANT: Parakeet binaries also need AVX-512 instruction checks**, even when built on pre-AVX-512 hardware.

The `ort` crate downloads prebuilt ONNX Runtime binaries that may contain AVX-512 instructions regardless of the build host's CPU. This is different from Whisper builds where the leakage comes from system libraries.

```bash
# Check Parakeet binaries for AVX-512 leakage
objdump -d voxtype-*-parakeet-avx2 | grep -c zmm
# If >0, the ONNX Runtime contains AVX-512 instructions
```

**Mitigation options:**
1. **Accept fallback behavior** - ONNX Runtime will fall back to non-AVX-512 code paths at runtime on unsupported CPUs (may cause slight performance penalty)
2. **Build ONNX Runtime from source** - Use `ORT_STRATEGY=build` to compile ONNX Runtime with specific CPU flags (significantly increases build time)
3. **Use `load-dynamic` feature** - Link against system ONNX Runtime instead of bundled (requires users to install ONNX Runtime separately)

For now, Parakeet binaries may contain AVX-512 instructions from ONNX Runtime but should still run on pre-AVX-512 CPUs via runtime fallback. Test on target hardware to verify.

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

See [docs/SMOKE_TESTS.md](docs/SMOKE_TESTS.md) for comprehensive manual testing procedures.

For automated regression testing, use the `/regression-test` skill which covers unit tests, CLI commands, config validation, and binary variant verification.
