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
          wl-clipboard  # Wayland clipboard (wl-copy, wl-paste)
          ydotool       # Alternative typing backend (X11 and Wayland)
          xdotool       # X11 typing fallback
          xclip         # X11 clipboard fallback
          libnotify     # Desktop notifications
          pciutils      # GPU detection (lspci)
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

        # Base derivation for voxtype (unwrapped)
        mkVoxtypeUnwrapped = { pname ? "voxtype", features ? [], extraNativeBuildInputs ? [], extraBuildInputs ? [] }:
          pkgs.rustPlatform.buildRustPackage {
            inherit pname;
            version = "0.4.9";

            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

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

            # Install shell completions and systemd service
            postInstall = ''
              # Shell completions
              install -Dm644 packaging/completions/voxtype.bash \
                $out/share/bash-completion/completions/voxtype
              install -Dm644 packaging/completions/voxtype.zsh \
                $out/share/zsh/site-functions/_voxtype
              install -Dm644 packaging/completions/voxtype.fish \
                $out/share/fish/vendor_completions.d/voxtype.fish

              # Systemd user service
              install -Dm644 packaging/systemd/voxtype.service \
                $out/lib/systemd/user/voxtype.service

              # Default config
              install -Dm644 config/default.toml \
                $out/share/voxtype/default-config.toml
            '';

            meta = with pkgs.lib; {
              description = "Push-to-talk voice-to-text for Linux";
              longDescription = ''
                Voxtype is a push-to-talk voice-to-text daemon for Linux.
                Hold a hotkey while speaking, release to transcribe and output
                text at your cursor position. Fully offline using whisper.cpp.
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

        # Build the ROCm/HIP variant for AMD GPUs (unwrapped)
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

      in {
        packages = {
          # Wrapped packages (ready to use, runtime deps in PATH)
          # Use these for Home Manager module and direct installation
          default = wrapVoxtype (mkVoxtypeUnwrapped {});
          vulkan = wrapVoxtype vulkanUnwrapped;
          rocm = wrapVoxtype rocmUnwrapped;

          # Unwrapped packages (for custom wrapping scenarios)
          voxtype-unwrapped = mkVoxtypeUnwrapped {};
          voxtype-vulkan-unwrapped = vulkanUnwrapped;
          voxtype-rocm-unwrapped = rocmUnwrapped;
        };

        # Development shell with all dependencies
        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.voxtype-unwrapped ];

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
