# Home Manager module for VoxType
#
# Usage in your home.nix or flake-based home-manager config:
#
#   imports = [ voxtype.homeManagerModules.default ];
#
#   programs.voxtype = {
#     enable = true;
#     package = voxtype.packages.${system}.vulkan;
#     model.name = "base.en";
#     service.enable = true;
#
#     # All config options go in settings (converted to config.toml)
#     settings = {
#       hotkey.enabled = false;  # Use compositor keybindings instead
#       output.mode = "type";
#       whisper.language = "en";
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

  # Build the config TOML from settings, injecting model path
  configFile = tomlFormat.generate "voxtype-config.toml" (
    lib.recursiveUpdate
      (lib.optionalAttrs (resolvedModelPath != null) {
        whisper.model = toString resolvedModelPath;
      })
      cfg.settings
  );

in {
  options.programs.voxtype = {
    enable = lib.mkEnableOption "VoxType push-to-talk voice-to-text";

    package = lib.mkOption {
      type = lib.types.package;
      description = ''
        The VoxType package to use. Use the flake's wrapped packages:
        - packages.default: CPU-only (works everywhere)
        - packages.vulkan: Vulkan GPU acceleration (AMD/NVIDIA/Intel)
        - packages.rocm: ROCm/HIP acceleration (AMD only)

        These packages include all runtime dependencies (wtype, ydotool, etc.)
        in their PATH.
      '';
      example = lib.literalExpression "voxtype.packages.\${system}.vulkan";
    };

    model = {
      name = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum (builtins.attrNames modelDefs));
        default = "base.en";
        description = ''
          Whisper model to download from HuggingFace. Set to null to manage
          the model yourself via model.path or let voxtype download on first run.

          Available: tiny, tiny.en, base, base.en, small, small.en,
          medium, medium.en, large-v3, large-v3-turbo
        '';
      };

      path = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Path to a custom whisper model file (overrides model.name).";
        example = "/home/user/.local/share/voxtype/models/ggml-custom.bin";
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
          whisper = {
            language = "en";
            translate = false;
          };
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
