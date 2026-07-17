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

        voxtype = import ./nix/packages.nix {
          inherit pkgs pkgsUnfree;
          src = self;
        };

      in {
        packages = voxtype.packages;

        # Development shell with all dependencies
        devShells.default = pkgs.mkShell {
          inputsFrom = [ voxtype.packages.voxtype-unwrapped ];

          LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          packages = with pkgs; [
            rust-analyzer
            rustfmt
            clippy
          ] ++ voxtype.runtimeDeps;
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
