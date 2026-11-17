# voxtype-git packaging

A development source package prepared for the upstream maintainer to publish
and maintain on AUR. Adding this directory to the GitHub repository does not
publish an AUR package.

The source tracks `dev`, where upstream merges contributions. `main` contains
stable releases and would not deliver unreleased fixes. `pkgver()` derives the
version from the latest reachable stable tag, commit distance, and commit hash.
The PKGBUILD does not need a version bump for every upstream commit.

## Build and install locally

With `base-devel` installed, run from this directory:

```bash
makepkg -s
sudo pacman -U voxtype-git-*.pkg.tar.zst
```

The package provides and conflicts with `voxtype`, so pacman will offer to
replace an installed stable package (including packages providing `voxtype`,
such as `voxtype-bin`). `/etc/voxtype/config.toml` is a backed-up configuration
file; personal configuration and models are not installed or removed by the
package. Restart the user service after switching packages or rebuilding.

`/usr/bin/voxtype` is a link to `voxtype-native` that the install script
creates; it is not a packaged file. `sudo voxtype setup gpu --enable` and
`sudo voxtype setup onnx --enable` point it at another variant, and an upgrade
leaves a link that still resolves alone. `voxtype-bin` manages the same path
from its own install script, so switching between `voxtype-bin` and this
package works in both directions. The stable source package `voxtype` ships
the link as a file, so going back to it needs `sudo pacman -R voxtype-git`
first.

Re-running `makepkg -s` fetches the latest `dev` and rebuilds it. To rebuild the
same revision, use `makepkg --holdver -f`. Once published on AUR, users can also
use their AUR helper's development-package update option, such as
`paru -Syu --devel` or `yay -Syu --devel`.

## Included builds

- Whisper CPU (`voxtype-native`, active by default) and Vulkan (`voxtype-vulkan`).
- ONNX CPU (`voxtype-onnx`): Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin,
  Omnilingual, and Cohere, plus the speaker-embedding diarization meetings use.
  It loads the optional system `onnxruntime>=1.24.0` at runtime, so the package
  builds the same engines with or without that library installed on the build
  machine. Enable it with `sudo voxtype setup onnx --enable`.
- GGUF workers for Parakeet and Cohere `.gguf` models, which every daemon above
  can drive: `voxtype-gguf-worker` (CPU, used by `voxtype-native` and
  `voxtype-onnx`) and `voxtype-gguf-worker-vulkan` (used by `voxtype-vulkan`).
- OSD launcher, GTK4 and native frontends, Quickshell launcher and QML files,
  OSD style packages and recipes, and the audio bridge. The GTK4 and native
  frontends include the waveform code.
- The GTK4 meetings window (`voxtype-meetings-gtk4`) and its launcher entry,
  when the checked-out tree has them (1.3.0 and later).
- The Omarchy Settings panel under `/usr/share/voxtype/omarchy-plugin`. Its
  QML sits in a directory named after the plugin manifest's version, and
  Omarchy reloads a panel only when that path changes. Two builds between
  releases share the version, so restart the Omarchy shell to see panel
  changes from an update.
- Configuration, systemd user service, configure desktop entry, icon,
  completions, documentation, and man pages.

CUDA, MIGraphX, and OpenVINO variants are not built by this recipe. The
optional ONNX Runtime, OSD and meeting-mode runtime dependencies are listed
separately in the PKGBUILD.

## CPU tuning

Build the package on the machine where it will run. The recipe sets
`GGML_NATIVE=ON`, so whisper.cpp in `voxtype-native` and `voxtype-vulkan` is
compiled with `-march=native` and may not start on a different CPU. It has to
ask for this: makepkg exports `SOURCE_DATE_EPOCH`, and ggml then turns native
tuning off and enables no other instruction set, which leaves whisper.cpp at
the x86-64 baseline (SSE2). In a test build that was about ten times slower:
5.3 s against 0.5 s to transcribe a 4.75 s clip with `base.en`.

To build a package for other machines, choose the instruction sets yourself.
For the AVX2 level the release binaries use:

```bash
GGML_NATIVE=OFF GGML_SSE42=ON GGML_AVX=ON GGML_AVX2=ON GGML_BMI2=ON GGML_FMA=ON GGML_F16C=ON makepkg -sC
```

`-C` matters when the package was built here before: Cargo does not rebuild
whisper.cpp because these variables changed, so an existing build directory
would keep the earlier tuning.

The GGUF workers do not follow `GGML_NATIVE`. Upstream's `.cargo/config.toml`
holds transcribe.cpp at the AVX2 level on every build, so GGUF models need a
CPU with AVX2 wherever the package was built. Rust code is built with
makepkg's flags, which target plain x86-64.

## Build directories

The CPU, Vulkan and ONNX daemons are built in separate Cargo target
directories (`target`, `target-vulkan`, `target-onnx`) inside the checkout.
Upstream requires that GPU feature sets never share build artifacts, and
separate directories keep that rule without a `cargo clean`, so an update
rebuilds only what changed. `makepkg -C` removes them for a build from nothing.

`prepare()` runs `cargo fetch`, which downloads every dependency in
`Cargo.lock`, including the OpenVINO sources behind an engine this recipe does
not build. Expect about 3 GB in `~/.cargo` and 8 GB of build tree.

A first build took 30 to 35 minutes on a 12-core machine. Half of that is one
file: clang needs 17 minutes for transcribe.cpp's `ggml-vulkan.cpp` in the
Vulkan GGUF worker. An update that only changes voxtype rebuilds each daemon
in about a minute and spends most of its time in `check()`.

`check()` runs upstream's test suite. One test runs `bwrap`, which needs
unprivileged user namespaces; where `bwrap` cannot create one (containers,
some build chroots, hardened kernels) the recipe skips that test.

## Until dev has 1.3.0

`dev` is still on 1.2, which has no GTK4 meetings window. The recipe checks
the tree for the `meetings-gtk4` feature and the meetings desktop entry
instead of assuming them. Both checks can go once 1.3.0 is merged back.

## Maintainer handoff

Before publication, build in a clean Arch chroot and inspect the result:

```bash
extra-x86_64-build
namcap PKGBUILD voxtype-git-*.pkg.tar.zst
makepkg --printsrcinfo > .SRCINFO
```

`extra-x86_64-build` is provided by `devtools`; install `namcap` separately.
The `aarch64` entry follows the stable source package and needs validation on
that architecture.

The AUR repository needs `PKGBUILD`, `.SRCINFO`, and `voxtype-git.install`.
Keep those files in sync when changing the recipe. If the install script
changes, regenerate its checksum with `updpkgsums` (from `pacman-contrib`)
before regenerating `.SRCINFO`. Only the moving Git source skips checksums.
