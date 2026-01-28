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

        # ONNX engine feature sets
        # All ONNX engines: Parakeet, Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual
        onnxCpuFeatures = [
          "parakeet-load-dynamic"
          "moonshine"
          "sensevoice"
          "paraformer"
          "dolphin"
          "omnilingual"
        ];

        onnxCudaFeatures = [
          "parakeet-load-dynamic"
          "parakeet-cuda"
          "moonshine-cuda"
          "sensevoice-cuda"
          "paraformer-cuda"
          "dolphin-cuda"
          "omnilingual-cuda"
        ];

        # Only Parakeet has ROCm support; other engines run on CPU
        onnxRocmFeatures = [
          "parakeet-load-dynamic"
          "parakeet-rocm"
          "moonshine"
          "sensevoice"
          "paraformer"
          "dolphin"
          "omnilingual"
        ];

        # Wrap a package with runtime dependencies
        wrapVoxtype = pkg: pkgs.symlinkJoin {
          name = "${pkg.pname or "voxtype"}-wrapped-${pkg.version}";
          paths = [ pkg ];
          buildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/voxtype \
              --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps}
          '';
          inherit (pkg) meta;
        };

        # Wrap an ONNX package with runtime dependencies and ORT_DYLIB_PATH
        # ONNX engines need ONNX Runtime at runtime for inference
        libExt = if pkgs.stdenv.isDarwin then "dylib" else "so";
        wrapOnnx = { onnxruntime ? pkgs.onnxruntime, pkg }: pkgs.symlinkJoin {
          name = "${pkg.pname or "voxtype"}-wrapped-${pkg.version}";
          paths = [ pkg ];
          buildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/voxtype \
              --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps} \
              --set ORT_DYLIB_PATH "${onnxruntime}/lib/libonnxruntime.${libExt}" \
              --prefix LD_LIBRARY_PATH : "${onnxruntime}/lib"
          '';
          inherit (pkg) meta;
        };

        # ONNX Runtime variants for different GPU backends
        onnxruntimeCuda = pkgsUnfree.onnxruntime.override { cudaSupport = true; };
        onnxruntimeRocm = pkgs.onnxruntime.override { rocmSupport = true; };

        # Base derivation for voxtype (unwrapped)
        mkVoxtypeUnwrapped = { pname ? "voxtype", features ? [], extraNativeBuildInputs ? [], extraBuildInputs ? [] }:
          pkgs.rustPlatform.buildRustPackage {
            inherit pname;
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "ort-2.0.0-rc.11" = "sha256-3v6wRi3mU/Fbd3fuiGxTRAXHj+VnUTsahU/oc7eiw18=";
              };
            };

            nativeBuildInputs = commonNativeBuildInputs ++ extraNativeBuildInputs;
            buildInputs = commonBuildInputs ++ extraBuildInputs;

            # Required for whisper-rs bindgen
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";

            # Build with specified features
            buildFeatures = features;

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

        # Build the ONNX + ROCm variant for AMD GPUs
        # Only Parakeet gets ROCm acceleration; other engines run on CPU
        onnxRocmUnwrapped = let
          pkg = mkVoxtypeUnwrapped {
            pname = "voxtype-onnx-rocm";
            features = onnxRocmFeatures;
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
          onnx-rocm = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxRocmUnwrapped; };

          # Backwards-compatible aliases (parakeet → onnx)
          parakeet = wrapOnnx { pkg = onnxUnwrapped; };
          parakeet-cuda = wrapOnnx { onnxruntime = onnxruntimeCuda; pkg = onnxCudaUnwrapped; };
          parakeet-rocm = wrapOnnx { onnxruntime = onnxruntimeRocm; pkg = onnxRocmUnwrapped; };

          # Unwrapped packages (for custom wrapping scenarios)
          voxtype-unwrapped = mkVoxtypeUnwrapped {};
          voxtype-vulkan-unwrapped = vulkanUnwrapped;
          voxtype-rocm-unwrapped = rocmUnwrapped;
          voxtype-onnx-unwrapped = onnxUnwrapped;
          voxtype-onnx-cuda-unwrapped = onnxCudaUnwrapped;
          voxtype-onnx-rocm-unwrapped = onnxRocmUnwrapped;

          # Backwards-compatible aliases
          voxtype-parakeet-unwrapped = onnxUnwrapped;
          voxtype-parakeet-cuda-unwrapped = onnxCudaUnwrapped;
          voxtype-parakeet-rocm-unwrapped = onnxRocmUnwrapped;
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
        # This is the recommended way to use VoxType on NixOS
        homeManagerModules.default = import ./nix/home-manager-module.nix;

        # NixOS module for system-level configuration
        # Provides typing backend selection, input group management, and ydotool daemon
        nixosModules.default = import ./nix/nixos-module.nix;
      };
}
