# macOS Release Plan

Status: In Progress
Branch: feature/macos-release
Target: Merge to main, Homebrew distribution, then signed distribution

---

## Phase 1: Rebase and Linux Validation

### 1.1 Rebase onto main (v0.5.3)
- [ ] `git fetch origin`
- [ ] `git rebase origin/main`
- [ ] Resolve any conflicts
- [ ] Verify build after rebase

### 1.2 Validate Linux compilation
- [ ] Run `cargo check` (quick syntax/type check)
- [ ] Run `cargo build --release` on Linux (via Docker or remote)
- [ ] Run `cargo test` to verify no regressions
- [ ] Confirm macOS-specific code is properly gated with `#[cfg(target_os = "macos")]`

---

## Phase 2: macOS Build and Homebrew

### 2.1 Build macOS binary
- [ ] `cargo build --release` on macOS
- [ ] Verify binary works: `./target/release/voxtype --version`
- [ ] Test basic functionality (record, transcribe, output)

### 2.2 Build SwiftUI Setup App
- [ ] `cd macos/VoxtypeSetup && ./build-app.sh`
- [ ] Test setup wizard flow
- [ ] Test preferences panel

### 2.3 Create Homebrew formula
- [ ] Create formula in homebrew-voxtype tap
- [ ] Test `brew install --build-from-source`
- [ ] Test `brew install` from bottle (if available)

---

## Phase 3: Signed Distribution

### 3.1 Apple Developer Setup
- [ ] Ensure Apple Developer account is active
- [ ] Create/verify Developer ID Application certificate
- [ ] Create/verify Developer ID Installer certificate (for pkg)
- [ ] Set up notarization credentials (app-specific password or API key)

### 3.2 Code Signing
- [ ] Sign voxtype binary with Developer ID
- [ ] Sign VoxtypeSetup.app with Developer ID
- [ ] Verify signatures: `codesign -dv --verbose=4`

### 3.3 Notarization
- [ ] Submit for notarization: `xcrun notarytool submit`
- [ ] Wait for approval
- [ ] Staple ticket: `xcrun stapler staple`

### 3.4 Distribution Package (choose one or both)

#### Option A: DMG Installer
- [ ] Create DMG with app bundle and symlink to /Applications
- [ ] Sign DMG
- [ ] Notarize DMG
- [ ] Test fresh install on clean Mac

#### Option B: Mac App Store (more restrictive)
- [ ] Create App Store Connect record
- [ ] Add required entitlements
- [ ] Sandbox compliance (may require significant changes)
- [ ] Submit for review

**Recommendation:** Start with DMG. App Store sandboxing may conflict with:
- Accessibility permission requirements
- Input monitoring
- LaunchAgent installation
- Calling external binaries

---

## Current State

### Completed
- [x] Basic macOS daemon functionality
- [x] LaunchAgent for auto-start
- [x] Hotkey detection via rdev
- [x] Audio capture via cpal
- [x] Text output via Accessibility API
- [x] Notifications
- [x] SwiftUI Setup App scaffolded (needs testing)

### In Progress
- [ ] Rebase onto v0.5.3

### Blocked
- [ ] Signed distribution (needs Phase 1-2 complete)

---

## Commands Reference

```bash
# Rebase
git fetch origin && git rebase origin/main

# Linux check (Docker)
docker run --rm -v $(pwd):/src -w /src rust:latest cargo check

# macOS build
cargo build --release

# SwiftUI app build
cd macos/VoxtypeSetup && ./build-app.sh

# Sign binary
codesign --force --options runtime --sign "Developer ID Application: YOUR NAME" target/release/voxtype

# Notarize
xcrun notarytool submit app.zip --apple-id EMAIL --team-id TEAM --password APP_PASSWORD --wait

# Staple
xcrun stapler staple Voxtype.app
```

---

## Notes

- SwiftUI app requires macOS 13+
- Homebrew formula should handle both Intel and Apple Silicon
- DMG is simpler for initial release; App Store can come later
- Keep CLI setup as fallback for power users / Homebrew installs
