{
  description = "A basic flake with a shell";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";
  inputs.pacparser.url = "github:traxys/pacparser-rs";

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    pacparser,
  }:
    flake-utils.lib.eachDefaultSystem (system: {
      devShell = pacparser.devShell."${system}";
    });
}
