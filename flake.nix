{
  description = "eithne — UEFI kernel";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        ovmf = pkgs.OVMF.fd;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.qemu
            pkgs.gnumake
          ];

          OVMF_X64 = "${ovmf}/FV/OVMF.fd";

          shellHook = ''
            echo "eithne dev shell"
            echo "  rust: $(rustc --version)"
            echo "  OVMF: $OVMF_X64"
          '';
        };
      });
}
