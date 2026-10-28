{
  description = "Sanchoris monorepo development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
          config.allowUnfreePredicate = pkg: (nixpkgs.lib.getName pkg) == "dynamodb-local";
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
          targets = [ "aarch64-unknown-linux-musl" ];
        };
        lambdaCc = pkgs.pkgsCross.aarch64-multiplatform-musl.stdenv.cc;
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            nodejs_latest
            pnpm
            rust
            rust-analyzer
            cargo-watch
            cargo-lambda
            dynamodb-local
            jre_headless
            gh
            jq
            openssl
            worktrunk
          ];

          CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER = "${lambdaCc}/bin/${lambdaCc.targetPrefix}cc";
          CC_aarch64_unknown_linux_musl = "${lambdaCc}/bin/${lambdaCc.targetPrefix}cc";
          AR_aarch64_unknown_linux_musl = "${lambdaCc}/bin/${lambdaCc.targetPrefix}ar";

          shellHook = ''
            export RUST_BACKTRACE=1
          '';
        };
      }
    );
}
