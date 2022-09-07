{
  description = "A basic flake with a shell";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.pacparser.url = "github:traxys/pacparser-rs";
  inputs.naersk.url = "github:nix-community/naersk";

  outputs = {
    self,
    flake-utils,
    pacparser,
    naersk,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import pacparser.inputs.nixpkgs {
        inherit system;

        overlays = [(import pacparser.inputs.rust-overlay)];
      };
      rust = pkgs.rust-bin.stable.latest.default;

      naersk' = pkgs.callPackage naersk {
        cargo = rust;
        rustc = rust;
      };
    in {
      devShell = pacparser.devShell."${system}";

      defaultPackage = naersk'.buildPackage {
        src = ./.;
        gitSubmodules = true;
        buildInputs = [pkgs.rustPlatform.bindgenHook];
      };
    });
}
