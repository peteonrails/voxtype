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

Re-running `makepkg -s` fetches the latest `dev` and rebuilds it. To rebuild the
same revision, use `makepkg --holdver -f`. Once published on AUR, users can also
use their AUR helper's development-package update option, such as
`paru -Syu --devel` or `yay -Syu --devel`.

## Included builds

- Whisper CPU (`voxtype-native`, active by default) and Vulkan (`voxtype-vulkan`).
- ONNX CPU (`voxtype-onnx`): Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin,
  Omnilingual, and Cohere. It loads the optional system `onnxruntime>=1.24.0` at
  runtime, so the package builds the same engines with or without that library
  installed on the build machine. Enable it with `sudo voxtype setup onnx --enable`.
- OSD launcher, GTK4 and native frontends, Quickshell launcher and QML files,
  and the audio bridge. The GTK4 and native frontends include the waveform code.
- Configuration, systemd user service, configure desktop entry, completions,
  documentation, and generated man pages.

The inference binaries use whisper.cpp's native CPU optimization, like the
stable source package; build them on the machine where they will run. CUDA,
MIGraphX, and OpenVINO variants are not built by this recipe. The optional ONNX
Runtime and OSD runtime dependencies are listed separately in the PKGBUILD.

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
