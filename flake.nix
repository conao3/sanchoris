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
          version = "0.1.0";
          web = pkgs.stdenv.mkDerivation (finalAttrs: {
            pname = "sanchoris-web";
            inherit version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./package.json
                ./pnpm-lock.yaml
                ./pnpm-workspace.yaml
                ./vite.config.ts
                ./tsconfig.base.json
                ./tsconfig.json
                ./apps
              ];
            };
            pnpmDeps = pkgs.fetchPnpmDeps {
              inherit (finalAttrs) pname version src;
              inherit pnpm;
              fetcherVersion = 5;
              hash = "sha256-xkQAq4RCVU/gBT1KlG6e8zSUogxBAAMhcsTQoEO2eY8=";
            };
            nativeBuildInputs = [
              nodejs
              pnpm
              pkgs.pnpmConfigHook
              pkgs.writableTmpDirAsHomeHook
            ];
            env = {
              VITE_COGNITO_DOMAIN = "dev-auth-k8s.sancode.dev";
              VITE_COGNITO_CLIENT_ID = "1urqpm7f6jdngv0adbcugp5f2l";
            };
            buildPhase = ''
              runHook preBuild
              export PATH="${nodejs}/bin:$PWD/node_modules/.bin:$PATH"
              node node_modules/vite-plus/bin/vp build
              node node_modules/vite-plus/bin/vp pack --filter workspace-agent
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p $out/share/sanchoris
              cp -r apps/frontend/dist $out/share/sanchoris/frontend
              cp -r apps/workspace-agent/dist $out/share/sanchoris/workspace-agent
              runHook postInstall
            '';
          });
          server = rustPlatform.buildRustPackage {
            pname = "sanchoris-server";
            inherit version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
                ./infra/tables.json
                ./infra/plans.json
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "-p"
              "sanchoris-api"
              "--bin"
              "sanchoris"
              "-p"
              "sanchoris-core"
              "--bin"
              "sanchoris-dynamo-create"
            ];
            doCheck = false;
          };
          workspaceCommand = pkgs.writeShellScriptBin "sanchoris" ''
            exec ${nodejs}/bin/node ${web}/share/sanchoris/workspace-agent/sanchoris.mjs "$@"
          '';
          app = pkgs.writeShellApplication {
            name = "sanchoris";
            runtimeInputs = [
              nodejs
              pkgs.dynamodb-local
            ];
            text = ''
              : "''${SANCHORIS_BIND_ADDR:?set SANCHORIS_BIND_ADDR, for example 127.0.0.1:8080}"
              state="''${XDG_STATE_HOME:-$HOME/.local/state}/sanchoris"
              mkdir -p "$state/docs" "$state/records" "$state/sockets" "$state/workspaces"

              dynamo_port=$(node -e "const s = require('net').createServer().listen(0, '127.0.0.1', () => { console.log(s.address().port); s.close(); });")
              dynamodb-local -inMemory -disableTelemetry -port "$dynamo_port" >"$state/dynamodb.log" 2>&1 &
              dynamo_pid=$!
              trap 'kill "$dynamo_pid" 2>/dev/null || true' EXIT
              trap 'exit 143' TERM
              trap 'exit 130' INT
              deadline=$((SECONDS + 60))
              until (exec 3<>"/dev/tcp/127.0.0.1/$dynamo_port") 2>/dev/null; do
                if [ "$SECONDS" -ge "$deadline" ]; then
                  echo "sanchoris: DynamoDB Local did not listen on port $dynamo_port (log: $state/dynamodb.log)" >&2
                  exit 1
                fi
                sleep 0.2
              done

              export AWS_REGION=ap-northeast-1
              export AWS_ACCESS_KEY_ID=local
              export AWS_SECRET_ACCESS_KEY=local
              export AWS_ENDPOINT_URL_DYNAMODB="http://127.0.0.1:$dynamo_port"
              export SANCHORIS_TABLE_MAIN=sanchoris-main
              export SANCHORIS_TABLE_AUDIT=sanchoris-audit
              export SANCHORIS_RECORDS_STORE=local
              export SANCHORIS_RECORDS_DIR="$state/records"
              export SANCHORIS_DOCS_DIR="$state/docs"
              export SANCHORIS_SOCKET_DIR="$state/sockets"
              export SANCHORIS_WORKSPACES_DIR="$state/workspaces"
              export SANCHORIS_FRONTEND_DIR=${web}/share/sanchoris/frontend
              export SANCHORIS_CONTROL_URL="ws://$SANCHORIS_BIND_ADDR/runs/channel"
              export SANCHORIS_WORKSPACE_COMMAND="${nodejs}/bin/node --no-warnings ${web}/share/sanchoris/workspace-agent/workspace-agent.mjs"
              export PATH="${workspaceCommand}/bin:$PATH"

              ${server}/bin/sanchoris-dynamo-create
              ${server}/bin/sanchoris all &
              server_pid=$!
              trap 'kill "$server_pid" "$dynamo_pid" 2>/dev/null || true' EXIT
              wait "$server_pid"
            '';
            meta = {
              description = "Sanchoris API, worker and web app in one local process against an in-memory DynamoDB Local";
              mainProgram = "sanchoris";
            };
          };
        in
        {
          packages = {
            default = app;
            inherit web server;
          };

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
