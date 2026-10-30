{
  description = "Sanchoris: hosted agent sessions that finish only when their required skills succeed";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];

      imports = [ inputs.treefmt-nix.flakeModule ];

      perSystem =
        { system, lib, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
            config.allowUnfreePredicate = pkg: lib.getName pkg == "dynamodb-local";
          };
          rust = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" ];
            targets = [ "aarch64-unknown-linux-musl" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
          nodejs = pkgs.nodejs_26;
          pnpm = pkgs.pnpm.override { nodejs-slim = pkgs.nodejs-slim_26; };
          vp = pkgs.writeShellApplication {
            name = "vp";
            runtimeInputs = [
              nodejs
              pnpm
            ];
            text = ''
              workspace_root="$PWD"
              while [[ "$workspace_root" != / && ! -f "$workspace_root/pnpm-workspace.yaml" ]]; do
                workspace_root="$(dirname "$workspace_root")"
              done

              if [[ ! -f "$workspace_root/pnpm-workspace.yaml" ]]; then
                printf '%s\n' "vp must run inside the sanchoris workspace" >&2
                exit 1
              fi

              if [[ "''${1:-}" == install ]]; then
                shift
                exec pnpm install "$@"
              fi

              vp_cli="$workspace_root/node_modules/vite-plus/bin/vp"
              if [[ ! -f "$vp_cli" ]]; then
                printf '%s\n' "The local Vite+ CLI is missing; run 'vp install' first" >&2
                exit 1
              fi

              export PATH="${nodejs}/bin:$workspace_root/node_modules/.bin:$PATH"
              exec ${nodejs}/bin/node "$vp_cli" "$@"
            '';
          };
        in
        {
          devShells.default = pkgs.mkShell {
            packages = [
              nodejs
              pnpm
              vp
              rust
              pkgs.rust-analyzer
              pkgs.cargo-nextest
              pkgs.cargo-watch
              pkgs.cargo-lambda
              pkgs.zig
              pkgs.sccache
              pkgs.dynamodb-local
              pkgs.jre_headless
              pkgs.awscli2
              pkgs.gh
              pkgs.jq
              pkgs.openssl
              pkgs.gnumake
              pkgs.worktrunk
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];

            RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";
            CARGO_BUILD_JOBS = "6";
            CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS = "-C link-arg=-fuse-ld=mold";

            shellHook = ''
              export RUST_BACKTRACE=1
              export SCCACHE_DIR="$HOME/.cache/sccache"
              mkdir -p "$SCCACHE_DIR"
              export SCCACHE_SERVER_UDS="$SCCACHE_DIR/server.sock"
              export SCCACHE_CACHE_SIZE="''${SCCACHE_CACHE_SIZE:-30G}"
            '';
          };

          treefmt.programs = {
            nixfmt.enable = true;
            rustfmt = {
              enable = true;
              package = rust;
            };
          };
        };
    };
}
