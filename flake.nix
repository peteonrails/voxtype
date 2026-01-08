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

        # Base derivation for voxtype
        mkVoxtype = { pname ? "voxtype", features ? [], extraNativeBuildInputs ? [], extraBuildInputs ? [] }:
          pkgs.rustPlatform.buildRustPackage {
            inherit pname;
            version = "0.4.7";

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

      in {
        packages = {
          # Default: CPU-only build (AVX2 baseline on x86_64)
          default = mkVoxtype {};

          # Vulkan GPU acceleration
          vulkan = mkVoxtype {
            pname = "voxtype-vulkan";
            features = [ "gpu-vulkan" ];
            extraNativeBuildInputs = with pkgs; [
              shaderc
              vulkan-headers
            ];
            extraBuildInputs = with pkgs; [
              vulkan-loader
            ];
          };
        };

        # Development shell with all dependencies
        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];

          packages = with pkgs; [
            rust-analyzer
            rustfmt
            clippy
            # Optional runtime deps for testing
            wtype
            ydotool
            wl-clipboard
          ];
        };
      }) // {
        # NixOS module for system-level configuration
        nixosModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.voxtype;
          in {
            options.services.voxtype = {
              enable = lib.mkEnableOption "voxtype voice-to-text service";

              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
                description = "The voxtype package to use";
              };
            };

            config = lib.mkIf cfg.enable {
              environment.systemPackages = [ cfg.package ];

              # Users need input group access for evdev
              # Note: Users must add themselves to the input group
              # or configure appropriate udev rules
            };
          };
      };
}
