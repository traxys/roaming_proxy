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

      rust = pkgs.rust-bin.stable.latest.default.override {
        targets = ["x86_64-pc-windows-gnu"];
      };

      naersk' = pkgs.callPackage naersk {
        cargo = rust;
        rustc = rust;
      };
    in {
      devShell = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          pkgsCross.mingwW64.stdenv.cc
          pkgsCross.mingwW64.windows.pthreads
          rust
        ];
      };

      packages.x86_64-pc-windows-gnu = naersk'.buildPackage {
        src = ./.;
        strictDeps = true;

        depsBuildBuild = with pkgs; [
          pkgsCross.mingwW64.stdenv.cc
          pkgsCross.mingwW64.windows.pthreads
        ];

        CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
      };

      defaultPackage = naersk'.buildPackage {
        src = ./.;
      };
    });
}
