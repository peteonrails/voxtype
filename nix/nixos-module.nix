# NixOS module for VoxType
#
# This module provides system-level configuration for VoxType.
# For user-level configuration with full options, use the Home Manager module.
#
# Usage in your configuration.nix or flake:
#
#   imports = [ voxtype.nixosModules.default ];
#
#   programs.voxtype = {
#     enable = true;
#     package = voxtype.packages.${system}.vulkan;
#   };
#
# For evdev hotkey support, add your user to the input group:
#   users.users.yourname.extraGroups = [ "input" ];
#
# For ydotool backend, enable the NixOS ydotool module:
#   programs.ydotool.enable = true;
#
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.voxtype;
in {
  options.programs.voxtype = {
    enable = lib.mkEnableOption "VoxType voice-to-text";

    package = lib.mkOption {
      type = lib.types.package;
      description = ''
        The VoxType package to install. Use the flake's wrapped packages:
        - packages.default: CPU-only
        - packages.vulkan: Vulkan GPU acceleration
        - packages.rocm: ROCm/HIP acceleration (AMD)

        These packages include all runtime dependencies (wtype, ydotool, etc.)
        in their PATH.
      '';
      example = lib.literalExpression "voxtype.packages.\${system}.vulkan";
    };
  };

  config = lib.mkIf cfg.enable {
    # Install VoxType (runtime deps are already in the wrapped package's PATH)
    environment.systemPackages = [ cfg.package ];
  };
}
