{
  description = "battery tester pc";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      crane
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # src = craneLib.cleanCargoSource ./.;
        src = nixpkgs.lib.cleanSourceWith {
          # inherit filter;
          src = ./.;
          name = "source";
        };

        nativeBuildInputs = with pkgs; [ rustToolchain pkg-config ];
        buildInputs = with pkgs; [
        ];

        commonArgs = {
          inherit src buildInputs nativeBuildInputs;
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        bin = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      with pkgs;
      {
        packages = {
          inherit bin;
          default = bin;
        };
        devShells.default = mkShell {
          inputsFrom = [ bin ];
        };
      }
    );
}
