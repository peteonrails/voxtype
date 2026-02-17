# Home Manager module for VoxType
#
# Usage in your home.nix or flake-based home-manager config:
#
#   imports = [ voxtype.homeManagerModules.default ];
#
#   # Whisper example (default engine):
#   programs.voxtype = {
#     enable = true;
#     package = voxtype.packages.${system}.vulkan;
#     model.name = "base.en";
#     service.enable = true;
#     settings = {
#       hotkey.enabled = false;
#       whisper.language = "en";
#     };
#   };
#
#   # Parakeet example:
#   programs.voxtype = {
#     enable = true;
#     engine = "parakeet";
#     package = voxtype.packages.${system}.parakeet-cuda;
#     model.path = "/path/to/parakeet-tdt-1.1b";
#     service.enable = true;
#     settings = {
#       hotkey.enabled = false;
#     };
#   };
#
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.voxtype;
  tomlFormat = pkgs.formats.toml { };
  modelDefs = import ./models.nix;

  # Fetch model from HuggingFace if using declarative model management
  fetchedModel = lib.optionalAttrs (cfg.model.name != null) (
    let modelDef = modelDefs.${cfg.model.name}; in
    pkgs.fetchurl {
      url = modelDef.url;
      hash = modelDef.hash;
    }
  );

  # Resolve the model path (fetched or user-provided)
  resolvedModelPath =
    if cfg.model.path != null then cfg.model.path
    else if cfg.model.name != null then fetchedModel
    else null;

  # Build the config TOML from settings, injecting engine and model path
  configFile = tomlFormat.generate "voxtype-config.toml" (
    lib.recursiveUpdate
      (lib.filterAttrs (_: v: v != null) {
        engine = cfg.engine;
        ${cfg.engine} = lib.optionalAttrs (resolvedModelPath != null) {
          model = toString resolvedModelPath;
        };
      })
      cfg.settings
  );

in {
  options.programs.voxtype = {
    enable = lib.mkEnableOption "VoxType push-to-talk voice-to-text";

    engine = lib.mkOption {
      type = lib.types.enum [ "whisper" "parakeet" ];
      default = "whisper";
      description = ''
        Speech recognition engine to use.
        - whisper: Local transcription via whisper.cpp (default)
        - parakeet: NVIDIA Parakeet models via ONNX Runtime

        When using parakeet, set model.name to null and use model.path
        to point to your Parakeet model directory.
      '';
    };

    package = lib.mkOption {
      type = lib.types.package;
      description = ''
        The VoxType package to use. Use the flake's wrapped packages:

        Whisper packages:
        - packages.default: CPU-only Whisper (works everywhere)
        - packages.vulkan: Vulkan GPU acceleration (AMD/NVIDIA/Intel)
        - packages.rocm: ROCm/HIP acceleration (AMD only)

        Parakeet packages (set engine = "parakeet"):
        - packages.parakeet: CPU-only Parakeet
        - packages.parakeet-cuda: CUDA acceleration (NVIDIA)
        - packages.parakeet-rocm: ROCm acceleration (AMD)

        All packages include runtime dependencies (wtype, dotool, ydotool, etc.).
      '';
      example = lib.literalExpression "voxtype.packages.\${system}.vulkan";
    };

    model = {
      name = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum (builtins.attrNames modelDefs));
        default = null;
        description = ''
          Whisper model to download from HuggingFace. Only used when engine = "whisper".
          Set to null when using Parakeet or managing models manually.

          Available: tiny, tiny.en, base, base.en, small, small.en,
          medium, medium.en, large-v3, large-v3-turbo
        '';
      };

      path = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Path to a model file or directory.
          - For Whisper: path to a .bin model file
          - For Parakeet: path to the model directory containing ONNX files

          Overrides model.name when set.
        '';
        example = "/home/user/.local/share/voxtype/models/parakeet-tdt-1.1b";
      };
    };

    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      description = ''
        Configuration for voxtype. These settings are written to
        ~/.config/voxtype/config.toml. See the voxtype documentation
        for available options.
      '';
      example = lib.literalExpression ''
        {
          hotkey = {
            enabled = false;  # Use compositor keybindings
            key = "SCROLLLOCK";
          };
          # For Whisper engine:
          whisper = {
            language = "en";
            translate = false;
          };
          # For Parakeet engine:
          # parakeet = {
          #   model_type = "tdt";
          # };
          output = {
            mode = "type";
            fallback_to_clipboard = true;
          };
          text = {
            spoken_punctuation = true;
            replacements = { "vox type" = "voxtype"; };
          };
        }
      '';
    };

    service.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable the systemd user service for VoxType.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.model.name != null && cfg.model.path != null);
        message = "programs.voxtype: cannot set both model.name and model.path";
      }
      {
        assertion = !(cfg.engine == "parakeet" && cfg.model.name != null);
        message = "programs.voxtype: model.name is only for Whisper models. Use model.path for Parakeet.";
      }
    ];

    home.packages = [ cfg.package ];

    xdg.configFile."voxtype/config.toml".source = configFile;

    systemd.user.services.voxtype = lib.mkIf cfg.service.enable {
      Unit = {
        Description = "VoxType push-to-talk voice-to-text daemon";
        Documentation = "https://voxtype.io";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" "pipewire.service" "pipewire-pulse.service" ];
      };

      Service = {
        Type = "simple";
        ExecStart = "${cfg.package}/bin/voxtype daemon";
        Restart = "on-failure";
        RestartSec = 5;
      };

      Install = {
        WantedBy = [ "graphical-session.target" ];
      };
    };
  };
}
