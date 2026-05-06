{
  description = "Sanchoris monorepo development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            nodejs_22
            pnpm
            rustc
            cargo
            rust-analyzer
            cargo-watch
            clippy
            docker-compose
            openssl
            postgresql_17
            rustfmt
            worktrunk
          ];

          shellHook = ''
            export RUST_BACKTRACE=1
            export DATABASE_URL="''${DATABASE_URL:-postgres://sanchoris:sanchoris@127.0.0.1:54329/sanchoris}"
          '';
        };
      }
    );
}
