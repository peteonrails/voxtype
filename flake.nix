{
  description = "Push-to-talk voice-to-text for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Separate pkgs instance with allowUnfree for CUDA-dependent packages.
        # legacyPackages doesn't support config overrides, so consumer flakes
        # can't pass allowUnfree=true through. CUDA has a non-free license
        # (CUDA EULA) that requires this. See: https://github.com/peteonrails/voxtype/issues/135
        pkgsUnfree = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
        };

        # Common build inputs for all variants
        commonNativeBuildInputs = with pkgs; [
          cmake
          pkg-config
          clang
          llvmPackages.libclang
          git  # Required by whisper.cpp cmake
        ];

        commonBuildInputs = with pkgs; [
          alsa-lib
          openssl
        ];

        # Cargo.lock contains four crates from the same pinned openvino-rs Git
        # revision. Nix requires an explicit fixed-output hash for every Git
        # dependency before it can vendor the lockfile.
        commonCargoLock = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "openvino-finder-0.11.0" = "sha256-nQWeHNdLlRk+owh3B6VpArAG2dr66HeBI2RHfDhyvvU=";
            "openvino-genai-0.11.0" = "sha256-nQWeHNdLlRk+owh3B6VpArAG2dr66HeBI2RHfDhyvvU=";
            "openvino-genai-sys-0.11.0" = "sha256-nQWeHNdLlRk+owh3B6VpArAG2dr66HeBI2RHfDhyvvU=";
            "openvino-sys-0.11.0" = "sha256-nQWeHNdLlRk+owh3B6VpArAG2dr66HeBI2RHfDhyvvU=";
          };
        };

        # Runtime dependencies wrapped into PATH
        runtimeDeps = with pkgs; [
          wtype         # Wayland typing
          dotool        # Universal typing backend via uinput
          wl-clipboard  # Wayland clipboard (wl-copy, wl-paste)
          ydotool       # Alternative typing backend (X11 and Wayland)
          xdotool       # X11 typing fallback
          xclip         # X11 clipboard fallback
          libnotify     # Desktop notifications
          pciutils      # GPU detection (lspci)
        ];

        # Tools the meeting features shell out to. These go on the wrapper
        # PATH with --suffix rather than --prefix: pactl and parec talk to the
        # user's running sound server, so a system copy matching it wins and
        # these are only the fallback.
        #
        # pipewire (pw-play, pw-dump, pw-link) is deliberately left out. Those tools
        # only work against a running PipeWire, and NixOS's PipeWire module
        # already puts the package in environment.systemPackages whenever the
        # service is on. Bundling it would add ~300 MiB to the closure.
        # Playback falls back to paplay from pulseaudio below.
        meetingRuntimeDeps = with pkgs; [
          ffmpeg-headless  # retained meeting audio and playback decoding
          pulseaudio       # parec, pactl (loopback capture), paplay
          xdg-utils        # xdg-open from the meeting view
          bubblewrap       # bwrap, the meeting assistant's codex jail
        ];

        # Basics voxtype shells out to outside the meeting features. NixOS
        # always has them, but Nix elsewhere and the build sandbox may not,
        # so they are fallbacks on the same --suffix PATH.
        baseRuntimeDeps = with pkgs; [
          curl   # model downloads (voxtype setup) and the announcement clip
          which  # output driver probes (wtype, dotool, ydotool, ...)
        ];

        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

        # Every release binary compiles the GGUF client. The features gate
        # client code only; transcribe.cpp itself lives in voxtype-gguf-worker.
        ggufClientFeatures = [ "gguf" "cohere-gguf" "parakeet-gguf" ];

        # ONNX engine feature sets
        # All ONNX engines: Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual
        onnxCpuFeatures = [
          "parakeet-load-dynamic"
          "moonshine"
          "sensevoice"
          "paraformer"
          "dolphin"
          "omnilingual"
          "cohere"
        ];

        onnxCudaFeatures = [
          "parakeet-load-dynamic"
          "parakeet-cuda"
          "moonshine-cuda"
          "sensevoice-cuda"
          "paraformer-cuda"
          "dolphin-cuda"
          "omnilingual-cuda"
          "cohere-cuda"
        ];

        # Only Parakeet has AMD GPU support (via MIGraphX); other engines run on CPU
        onnxMigraphxFeatures = [
          "parakeet-load-dynamic"
          "parakeet-migraphx"
          "moonshine"
          "sensevoice"
          "paraformer"
          "dolphin"
          "omnilingual"
        ];

        runtimePathArgs = ''
          --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps} \
          --suffix PATH : ${pkgs.lib.makeBinPath (meetingRuntimeDeps ++ baseRuntimeDeps)}'';

        # Wrap a package with runtime dependencies
        wrapVoxtype = pkg: pkgs.symlinkJoin {
          name = "${pkg.pname or "voxtype"}-wrapped-${pkg.version}";
          paths = [ pkg ];
          buildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/voxtype \
              ${runtimePathArgs}
          '';
          inherit (pkg) meta;
        };

        # Wrap an ONNX package with runtime dependencies and ORT_DYLIB_PATH
        # ONNX engines need ONNX Runtime at runtime for inference
        libExt = if pkgs.stdenv.isDarwin then "dylib" else "so";
        wrapOnnx = { onnxruntime ? pkgs.onnxruntime, pkg, extraWrapperArgs ? "" }: pkgs.symlinkJoin {
          name = "${pkg.pname or "voxtype"}-wrapped-${pkg.version}";
          paths = [ pkg ];
          buildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/voxtype \
              ${runtimePathArgs} \
              --set ORT_DYLIB_PATH "${onnxruntime}/lib/libonnxruntime.${libExt}" \
              --prefix LD_LIBRARY_PATH : "${onnxruntime}/lib" \
              ${extraWrapperArgs}
          '';
          inherit (pkg) meta;
        };

        # Extra wrapper args for MIGraphX (ROCm) to set cache directory
        migraphxWrapperArgs = ''
          --run '
            : "''${ORT_MIGRAPHX_MODEL_CACHE_PATH:=''${XDG_CACHE_HOME:-$HOME/.cache}/voxtype/migraphx}"
            export ORT_MIGRAPHX_MODEL_CACHE_PATH
            mkdir -p "$ORT_MIGRAPHX_MODEL_CACHE_PATH"
          '
        '';

        # ONNX Runtime variants for different GPU backends
        onnxruntimeCuda = pkgsUnfree.onnxruntime.override { cudaSupport = true; };
        onnxruntimeRocm = pkgs.onnxruntime.override { rocmSupport = true; };


        # GGUF models (Cohere, Parakeet) run out of process in
        # voxtype-gguf-worker, which links transcribe.cpp. It is its own
        # workspace crate because transcribe.cpp and whisper-rs each vendor a
        # ggml exporting the same symbols (see crates/gguf-worker/Cargo.toml),
        # so it is built as its own derivation and linked in beside the
        # voxtype binary, where voxtype looks for it first.
        mkGgufWorker = { pname ? "voxtype-gguf-worker", features ? [], extraNativeBuildInputs ? [], extraBuildInputs ? [] }:
          pkgs.rustPlatform.buildRustPackage {
            inherit pname version;

            src = ./.;
            cargoLock = commonCargoLock;

            nativeBuildInputs = with pkgs; [ cmake pkg-config git ] ++ extraNativeBuildInputs;
            buildInputs = extraBuildInputs;

            cargoBuildFlags = [ "-p" "voxtype-gguf-worker" ];
            buildFeatures = features;

            # The worker's tests dev-depend on the whole voxtype crate, which
            # the voxtype derivations already build and test.
            doCheck = false;

            # Same ISA floor as voxtype. .cargo/config.toml carries the
            # matching TRANSCRIBE_CMAKE_ARGS for transcribe.cpp's ggml.
            RUSTFLAGS = pkgs.lib.optionalString (system == "x86_64-linux")
              "-C target-cpu=x86-64-v3";

            preBuild = ''
              export CMAKE_BUILD_PARALLEL_LEVEL=$NIX_BUILD_CORES
            '';

            meta = with pkgs.lib; {
              description = "Out-of-process GGUF (transcribe.cpp) worker for voxtype";
              homepage = "https://voxtype.io";
              license = licenses.mit;
              maintainers = [];
              platforms = [ "x86_64-linux" "aarch64-linux" ];
              mainProgram = "voxtype-gguf-worker";
            };
          };

        # CPU worker, paired with the CPU Whisper package like the release's
        # baseline/avx2/avx512 binaries.
        ggufWorkerCpu = mkGgufWorker {};

        # Vulkan worker, paired with the GPU and ONNX packages like the
        # release's vulkan and onnx-* binaries. gguf_backend = "auto" falls
        # back to the CPU when no Vulkan device is present.
        ggufWorkerVulkan = mkGgufWorker {
          pname = "voxtype-gguf-worker-vulkan";
          features = [ "vulkan" ];
          extraNativeBuildInputs = with pkgs; [ shaderc vulkan-headers ];
          extraBuildInputs = with pkgs; [ vulkan-headers vulkan-loader spirv-headers ];
        };

        # Base derivation for voxtype (unwrapped)
        mkVoxtypeUnwrapped = { pname ? "voxtype", features ? [], ggufWorker ? ggufWorkerCpu, extraNativeBuildInputs ? [], extraBuildInputs ? [] }:
          pkgs.rustPlatform.buildRustPackage {
            inherit pname version;

            src = ./.;
            cargoLock = commonCargoLock;

            nativeBuildInputs = commonNativeBuildInputs ++ extraNativeBuildInputs;
            buildInputs = commonBuildInputs ++ extraBuildInputs;
            # The meeting assistant tests resolve codex only when bwrap is on
            # PATH, as it is at runtime through meetingRuntimeDeps.
            # curl: the announcement fetch tests run it against file:// URLs.
            nativeCheckInputs = with pkgs; [ dbus python3 bubblewrap curl ];

            # This test runs codex inside the real bwrap jail, which bind-mounts
            # the host's /usr. The Nix build sandbox has no /usr.
            checkFlags = [
              "--skip=meeting::assistant::tests::codex_runs_in_a_jail_that_hides_home"
            ];

            # Required for whisper-rs bindgen
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            # Build with specified features
            buildFeatures = ggufClientFeatures ++ features;

            # Ensure reproducible builds targeting AVX2-capable CPUs (x86-64-v3)
            # This matches the portable AVX2 binaries we ship for other distros
            RUSTFLAGS = pkgs.lib.optionalString (system == "x86_64-linux")
              "-C target-cpu=x86-64-v3";

            # whisper.cpp cmake needs some help in sandbox
            preBuild = ''
              export CMAKE_BUILD_PARALLEL_LEVEL=$NIX_BUILD_CORES
            '';

            # Install shell completions and default config
            # Note: systemd service is NOT installed here because it contains
            # hardcoded FHS paths (/usr/bin/voxtype) that don't work on NixOS.
            # Use the Home Manager module with service.enable = true instead,
            # which generates a service with the correct Nix store path.
            postInstall = ''
              # Shell completions
              install -Dm644 packaging/completions/voxtype.bash \
                $out/share/bash-completion/completions/voxtype
              install -Dm644 packaging/completions/voxtype.zsh \
                $out/share/zsh/site-functions/_voxtype
              install -Dm644 packaging/completions/voxtype.fish \
                $out/share/fish/vendor_completions.d/voxtype.fish

              # Default config
              install -Dm644 config/default.toml \
                $out/share/voxtype/default-config.toml

              # GGUF worker beside the binary: voxtype resolves
              # /proc/self/exe and looks in that directory first.
              ln -s ${ggufWorker}/bin/voxtype-gguf-worker $out/bin/voxtype-gguf-worker

              # Quickshell QML tree (OSD and the standalone meeting view), laid
              # out as scripts/package.sh does. `voxtype setup quickshell` and
              # the voxtype-osd-quickshell launcher find it at
              # <binary>/../share/voxtype/quickshell.
              mkdir -p $out/share/voxtype/quickshell
              tar -cf - \
                --exclude='.git' \
                --exclude='.gitignore' \
                --exclude='.*.swp' \
                --exclude='*~' \
                --exclude='.DS_Store' \
                -C quickshell . | tar -xf - -C $out/share/voxtype/quickshell
              find $out/share/voxtype/quickshell -type f -exec chmod 644 {} \;
              find $out/share/voxtype/quickshell -type d -exec chmod 755 {} \;

              # Omarchy Settings plugin, staged by the script every package
              # uses. Unused without Omarchy, shipped for parity.
              bash scripts/stage-omarchy-plugin.sh omarchy-plugin \
                $out/share/voxtype/omarchy-plugin

              # Desktop entries, the launcher the configure entry runs, and
              # the hicolor icon both entries name.
              install -Dm755 packaging/scripts/voxtype-configure-launcher \
                $out/bin/voxtype-configure-launcher
              install -Dm644 packaging/voxtype-configure.desktop \
                $out/share/applications/voxtype-configure.desktop
              install -Dm644 packaging/voxtype-meetings.desktop \
                $out/share/applications/voxtype-meetings.desktop
              install -Dm644 assets/icon.svg \
                $out/share/icons/hicolor/scalable/apps/voxtype.svg
              install -Dm644 assets/icon.png \
                $out/share/icons/hicolor/128x128/apps/voxtype.png
            '';

            meta = with pkgs.lib; {
              description = "Push-to-talk voice-to-text for Linux";
              longDescription = ''
                Voxtype is a push-to-talk voice-to-text daemon for Linux.
                Hold a hotkey while speaking, release to transcribe and output
                text at your cursor position. Supports Whisper, Parakeet,
                SenseVoice, Moonshine, Paraformer, Dolphin, and Omnilingual engines.
              '';
              homepage = "https://voxtype.io";
              license = licenses.mit;
              maintainers = []; # Add NixOS maintainers when upstreaming
              platforms = [ "x86_64-linux" "aarch64-linux" ];
              mainProgram = "voxtype";
            };
          };

        # Build the Vulkan variant (unwrapped)
        vulkanUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-vulkan";
            ggufWorker = ggufWorkerVulkan;
            features = [ "gpu-vulkan" ];
            extraNativeBuildInputs = with pkgs; [
              shaderc
              vulkan-headers
              vulkan-loader
            ];
            extraBuildInputs = with pkgs; [
              vulkan-headers
              vulkan-loader
            ];
          };
        in pkg.overrideAttrs (old: {
          # Help cmake find Vulkan SDK components
          preBuild = (old.preBuild or "") + ''
            export CMAKE_BUILD_PARALLEL_LEVEL=$NIX_BUILD_CORES
            export VULKAN_SDK="${pkgs.vulkan-loader}"
            export Vulkan_INCLUDE_DIR="${pkgs.vulkan-headers}/include"
            export Vulkan_LIBRARY="${pkgs.vulkan-loader}/lib/libvulkan.so"
          '';
        });

        # Build the ROCm/HIP variant for AMD GPUs (unwrapped, Whisper only)
        rocmUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-rocm";
            ggufWorker = ggufWorkerVulkan;
            features = [ "gpu-hipblas" ];
            extraNativeBuildInputs = with pkgs; [
              rocmPackages.clr
              rocmPackages.hipblas
              rocmPackages.rocblas
            ];
            extraBuildInputs = with pkgs; [
              rocmPackages.clr
              rocmPackages.hipblas
              rocmPackages.rocblas
            ];
          };
        in pkg.overrideAttrs (old: {
          # Help cmake find ROCm/HIP components
          preBuild = (old.preBuild or "") + ''
            export CMAKE_BUILD_PARALLEL_LEVEL=$NIX_BUILD_CORES
            export HIP_PATH="${pkgs.rocmPackages.clr}"
            export ROCM_PATH="${pkgs.rocmPackages.clr}"
          '';
        });

        # Build the ONNX variant (CPU, all engines)
        # Uses load-dynamic for Parakeet, ort for other engines
        onnxUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-onnx";
            ggufWorker = ggufWorkerVulkan;
            features = onnxCpuFeatures;
            extraBuildInputs = with pkgs; [ onnxruntime ];
          };
        in pkg.overrideAttrs (old: {
          # Tell ort-sys where to find ONNX Runtime (avoids sandbox download)
          ORT_LIB_LOCATION = "${pkgs.onnxruntime}/lib";
        });

        # Build the ONNX + CUDA variant for NVIDIA GPUs
        # Uses pkgsUnfree because CUDA has a non-free license (CUDA EULA)
        onnxCudaUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-onnx-cuda";
            ggufWorker = ggufWorkerVulkan;
            features = onnxCudaFeatures;
            extraNativeBuildInputs = [ pkgsUnfree.cudaPackages.cuda_nvcc ];
            extraBuildInputs = [
              onnxruntimeCuda
              pkgsUnfree.cudaPackages.cudatoolkit
              pkgsUnfree.cudaPackages.cudnn
            ];
          };
        in pkg.overrideAttrs (old: {
          ORT_LIB_LOCATION = "${onnxruntimeCuda}/lib";
        });

        # Build the ONNX + MIGraphX variant for AMD GPUs
        # Only Parakeet gets AMD GPU acceleration; other engines run on CPU
        onnxMigraphxUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-onnx-migraphx";
            ggufWorker = ggufWorkerVulkan;
            features = onnxMigraphxFeatures;
            extraNativeBuildInputs = with pkgs; [
              rocmPackages.clr
            ];
            extraBuildInputs = [
              onnxruntimeRocm
              pkgs.rocmPackages.clr
              pkgs.rocmPackages.rocblas
            ];
          };
        in pkg.overrideAttrs (old: {
          ORT_LIB_LOCATION = "${onnxruntimeRocm}/lib";
        });

        # OSD frontend packages. The launcher binary (`voxtype-osd`) ships
        # with every main voxtype package, these provide the GUI frontend
        # the launcher execs into.
        osdNative = pkgs.rustPlatform.buildRustPackage {
          pname = "voxtype-osd-native";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = ./.;
          cargoLock = commonCargoLock;

          nativeBuildInputs = commonNativeBuildInputs ++ [ pkgs.makeWrapper ];
          buildInputs = commonBuildInputs ++ [ pkgs.libxkbcommon ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          buildFeatures = [ "osd-native" ];
          cargoBuildFlags = [ "--bin" "voxtype-osd-native" ];

          # Skip running the full lib test suite, this package only ships the OSD bin.
          doCheck = false;

          # wgpu dlopens libvulkan / libwayland-client at runtime.
          postFixup = ''
            wrapProgram $out/bin/voxtype-osd-native \
              --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath (with pkgs; [
                vulkan-loader
                wayland
              ])}"
          '';

          meta = with pkgs.lib; {
            description = "Native (Wayland + wgpu + egui) on-screen display frontend for voxtype";
            homepage = "https://voxtype.io";
            license = licenses.mit;
            maintainers = [];
            platforms = [ "x86_64-linux" "aarch64-linux" ];
            mainProgram = "voxtype-osd-native";
          };
        };

        osdGtk4 = pkgs.rustPlatform.buildRustPackage {
          pname = "voxtype-osd-gtk4";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = ./.;
          cargoLock = commonCargoLock;

          nativeBuildInputs = commonNativeBuildInputs ++ [ pkgs.wrapGAppsHook4 ];
          buildInputs = commonBuildInputs ++ [ pkgs.gtk4-layer-shell ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          buildFeatures = [ "osd-gtk4" ];
          cargoBuildFlags = [ "--bin" "voxtype-osd-gtk4" ];

          # Skip running the full lib test suite, this package only ships the OSD bin.
          doCheck = false;

          meta = with pkgs.lib; {
            description = "GTK4 on-screen display frontend for voxtype";
            homepage = "https://voxtype.io";
            license = licenses.mit;
            maintainers = [];
            platforms = [ "x86_64-linux" "aarch64-linux" ];
            mainProgram = "voxtype-osd-gtk4";
          };
        };

        # The meetings window (`voxtype meeting view` on desktops without
        # Quickshell). Plain GTK4 in a normal window, so no layer-shell.
        # The daemon finds it on PATH, so install it beside any voxtype
        # package (home.packages, environment.systemPackages).
        meetingsGtk4 = pkgs.rustPlatform.buildRustPackage {
          pname = "voxtype-meetings-gtk4";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = ./.;
          cargoLock = commonCargoLock;

          nativeBuildInputs = commonNativeBuildInputs ++ [ pkgs.wrapGAppsHook4 ];
          buildInputs = commonBuildInputs ++ [ pkgs.gtk4 ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

          buildFeatures = [ "meetings-gtk4" ];
          cargoBuildFlags = [ "--bin" "voxtype-meetings-gtk4" ];

          # Skip running the full lib test suite, this package only ships the window.
          doCheck = false;

          meta = with pkgs.lib; {
            description = "GTK4 meetings window for voxtype";
            homepage = "https://voxtype.io";
            license = licenses.mit;
            maintainers = [];
            platforms = [ "x86_64-linux" "aarch64-linux" ];
            mainProgram = "voxtype-meetings-gtk4";
          };
        };

      in {
        packages = {
          # Wrapped packages (ready to use, runtime deps in PATH)
          # Use these for Home Manager module and direct installation
          default = wrapVoxtype (mkVoxtypeUnwrapped {});
          vulkan = wrapVoxtype vulkanUnwrapped;
          rocm = wrapVoxtype rocmUnwrapped;

          # ONNX variants (all ONNX engines: Parakeet, Moonshine, SenseVoice,
          # Paraformer, Dolphin, Omnilingual)
          onnx = wrapOnnx { pkg = onnxUnwrapped; };
          onnx-cuda = wrapOnnx { onnxruntime = onnxruntimeCuda; pkg = onnxCudaUnwrapped; };
          onnx-migraphx = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxMigraphxUnwrapped; extraWrapperArgs = migraphxWrapperArgs; };

          # Backwards-compatible aliases (parakeet → onnx, rocm → migraphx)
          parakeet = wrapOnnx { pkg = onnxUnwrapped; };
          parakeet-cuda = wrapOnnx { onnxruntime = onnxruntimeCuda; pkg = onnxCudaUnwrapped; };
          parakeet-migraphx = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxMigraphxUnwrapped; extraWrapperArgs = migraphxWrapperArgs; };
          # Legacy: rocm → migraphx (drop in v0.8.0)
          onnx-rocm = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxMigraphxUnwrapped; extraWrapperArgs = migraphxWrapperArgs; };
          parakeet-rocm = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxMigraphxUnwrapped; extraWrapperArgs = migraphxWrapperArgs; };

          # OSD frontends (installable alongside any voxtype package)
          osd-native = osdNative;
          osd-gtk4 = osdGtk4;

          # The GTK4 meetings window (installable alongside any voxtype package)
          meetings-gtk4 = meetingsGtk4;

          # GGUF workers. Every voxtype package already links the matching
          # one into its bin/; these are for custom wrapping.
          gguf-worker = ggufWorkerCpu;
          gguf-worker-vulkan = ggufWorkerVulkan;

          # Unwrapped packages (for custom wrapping scenarios)
          voxtype-unwrapped = mkVoxtypeUnwrapped {};
          voxtype-vulkan-unwrapped = vulkanUnwrapped;
          voxtype-rocm-unwrapped = rocmUnwrapped;
          voxtype-onnx-unwrapped = onnxUnwrapped;
          voxtype-onnx-cuda-unwrapped = onnxCudaUnwrapped;
          voxtype-onnx-migraphx-unwrapped = onnxMigraphxUnwrapped;

          # Backwards-compatible aliases
          voxtype-parakeet-unwrapped = onnxUnwrapped;
          voxtype-parakeet-cuda-unwrapped = onnxCudaUnwrapped;
          voxtype-parakeet-migraphx-unwrapped = onnxMigraphxUnwrapped;
          # Legacy
          voxtype-onnx-rocm-unwrapped = onnxMigraphxUnwrapped;
          voxtype-parakeet-rocm-unwrapped = onnxMigraphxUnwrapped;
        };

        # Development shell with all dependencies
        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.voxtype-unwrapped ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          packages = with pkgs; [
            rust-analyzer
            rustfmt
            clippy
          ] ++ runtimeDeps;
        };
      }) // {
        # Home Manager module for declarative user-level configuration
        # This is the recommended way to use Voxtype on NixOS
        homeManagerModules.default = import ./nix/home-manager-module.nix;

        # NixOS module for system-level configuration
        # Provides typing backend selection, input group management, and ydotool daemon
        nixosModules.default = import ./nix/nixos-module.nix;
      };
}
