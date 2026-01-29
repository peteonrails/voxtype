# Home Manager module for VoxType
#
# Usage in your home.nix or flake-based home-manager config:
#
#   imports = [ voxtype.homeManagerModules.default ];
#
#   # Whisper example (default engine):
#   programs.voxtype = {
#     enable = true;
#     package = voxtype.packages.${pkgs.stdenv.hostPlatform.system}.vulkan;
#     model.name = "base.en";
#     service.enable = true;
#     settings = {
#       hotkey.enabled = false;
#       whisper.language = "en";
#     };
#   };
#
#   # Parakeet example (declarative model download):
#   programs.voxtype = {
#     enable = true;
#     engine = "parakeet";
#     package = voxtype.packages.${pkgs.stdenv.hostPlatform.system}.parakeet-cuda;
#     model.name = "parakeet-tdt-0.6b-v3";  # Automatically fetched
#     service.enable = true;
#     settings = {
#       hotkey.enabled = false;
#     };
#   };
#
#   # Parakeet example (custom model path):
#   programs.voxtype = {
#     enable = true;
#     engine = "parakeet";
#     package = voxtype.packages.${pkgs.stdenv.hostPlatform.system}.parakeet-cuda;
#     model.path = "/path/to/custom-parakeet-model";
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
  whisperModelDefs = import ./whisper-models.nix;
  parakeetModelDefs = import ./parakeet-models.nix;

  # Fetch Whisper model from HuggingFace (single .bin file)
  fetchedWhisperModel = lib.optionalAttrs 
    (cfg.engine == "whisper" && cfg.model.name != null) (
    let modelDef = whisperModelDefs.${cfg.model.name}; in
    pkgs.fetchurl {
      url = modelDef.url;
      hash = modelDef.hash;
    }
  );

  # Fetch Parakeet model from HuggingFace (directory with multiple ONNX files)
  # Parakeet models consist of multiple files (encoder, decoder, vocab, config)
  # that must be in a directory structure, so we use linkFarm to create that structure.
  # Whisper uses a single .bin file, so fetchurl is sufficient.
  fetchedParakeetModel = lib.optionalAttrs
    (cfg.engine == "parakeet" && cfg.model.name != null) (
    let 
      modelDef = parakeetModelDefs.${cfg.model.name};
      fileList = lib.mapAttrsToList (filename: fileSpec: {
        name = filename;
        path = pkgs.fetchurl {
          url = fileSpec.url;
          hash = fileSpec.hash;
        };
      }) modelDef.files;
    in
    pkgs.linkFarm cfg.model.name fileList
  );

  # Resolve the model path (fetched or user-provided)
  resolvedModelPath =
    if cfg.model.path != null then cfg.model.path
    else if cfg.engine == "whisper" && cfg.model.name != null then fetchedWhisperModel
    else if cfg.engine == "parakeet" && cfg.model.name != null then fetchedParakeetModel
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

        Both engines support declarative model management via model.name
        or manual management via model.path.
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

        All packages include runtime dependencies (wtype, ydotool, etc.).
      '';
      example = lib.literalExpression "voxtype.packages.\${pkgs.stdenv.hostPlatform.system}.vulkan";
    };

    model = {
      name = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum (
          (builtins.attrNames whisperModelDefs) ++ (builtins.attrNames parakeetModelDefs)
        ));
        default = null;
        description = ''
          Model to download from HuggingFace. Automatically fetched when set.

          Whisper models (engine = "whisper"):
            tiny, tiny.en, base, base.en, small, small.en,
            medium, medium.en, large-v3, large-v3-turbo

          Parakeet models (engine = "parakeet"):
            parakeet-tdt-0.6b-v2, parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8

          Set to null and use model.path for custom/manually managed models.
        '';
      };

      path = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Path to a model file or directory. Use this for custom models
          not available in model.name presets.

          - For Whisper: path to a .bin model file
          - For Parakeet: path to the model directory containing ONNX files

          Overrides model.name when set. Cannot be used together with model.name.
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
        message = ''
          programs.voxtype: cannot set both model.name and model.path.
          Choose one: use model.name for preset models, or model.path for custom models.
        '';
      }
      {
        assertion = !(cfg.engine == "whisper" && cfg.model.name != null && !(builtins.hasAttr cfg.model.name whisperModelDefs));
        message = ''
          programs.voxtype: '${cfg.model.name}' is not a valid Whisper model.
          Available Whisper models: ${lib.concatStringsSep ", " (builtins.attrNames whisperModelDefs)}
          Alternatively, use model.path to specify a custom model file.
        '';
      }
      {
        assertion = !(cfg.engine == "parakeet" && cfg.model.name != null && !(builtins.hasAttr cfg.model.name parakeetModelDefs));
        message = ''
          programs.voxtype: '${cfg.model.name}' is not a valid Parakeet model.
          Available Parakeet models: ${lib.concatStringsSep ", " (builtins.attrNames parakeetModelDefs)}
          Alternatively, use model.path to specify a custom model directory.
        '';
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
