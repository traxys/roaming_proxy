{
  description = "A basic flake with a shell";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.rust-overlay.url = "github:oxalica/rust-overlay";
  inputs.pacparser.url = "github:traxys/pacparser-rs";

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
    pacparser,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;

        overlays = [(import rust-overlay)];
      };
      rust = pkgs.rust-bin.stable.latest.default.override {
        extensions = ["llvm-tools-preview" "rust-src"];
      };
    in {
      devShell = pacparser."${system}".devShell;
    });
}
