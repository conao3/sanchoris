# Sanchoris

Sanchoris is a pnpm + Vite frontend and Rust backend monorepo.

## Layout

- `apps/frontend`: Vite frontend package managed by pnpm
- `crates/backend`: Rust backend crate
- `scripts/dev.mjs`: local development process supervisor

## Development

Enter the Nix development shell:

```sh
nix develop
```

Install frontend dependencies:

```sh
pnpm install
```

Run the frontend, backend, and local PostgreSQL on one local development host:

```sh
pnpm dev
```

Start the local PostgreSQL container before running dev when you need persistent MVP data storage:

```sh
docker-compose up -d postgres
pnpm dev
```

The dev command applies `db/schema.sql` at startup, then starts the Rust backend on an internal free port, starts Vite through portless, and proxies `/api/*` through the same frontend host. By default, local development uses plain HTTP on an unprivileged port so it works without sudo or TLS setup.

The default local `DATABASE_URL` is:

```text
postgres://sanchoris:sanchoris@127.0.0.1:54329/sanchoris
```

The schema source of truth is `db/schema.sql`. `pnpm db:schema:check` starts a temporary PostgreSQL server, applies the schema, and verifies the required MVP tables exist.

- Main worktree: `http://sanchoris.localhost:1355/admin/health`
- Linked worktree, e.g. branch `fix-health`: `http://fix-health.sanchoris.localhost:1355/admin/health`

The backend keeps `/health` and `/api/v1/health` available for process health checks. Product data is exposed through the single GraphQL endpoint used by the frontend:

- `/api/graphql`: async-graphql endpoint for `viewer`, project profiles, conversations, native tasks, workflow specs, runs, workspace state, verification result, pull request, merge, and gate state.
- `apps/frontend/src/graphql/schema.graphql`: checked-in SDL snapshot generated from the backend.
- `apps/frontend/src/graphql/mvp-shell.graphql`: MVP query and mutation documents.
- `apps/frontend/src/graphql/generated/`: GraphQL Code Generator client preset output used by Apollo Client.

If you want clean HTTPS URLs without port numbers, start portless with TLS explicitly:

```sh
PORTLESS_HTTPS=1 PORTLESS_PORT=443 pnpm dev
```

## Worktrees

Use Worktrunk to manage parallel worktrees:

```sh
wt switch -c fix-health
docker-compose up -d postgres
pnpm dev
```

Worktrunk runs `scripts/worktrunk-env-sync.mjs` for new worktrees. The script updates only the managed block in the gitignored `.env` file:

```dotenv
# BEGIN WORKTRUNK ENV
COMPOSE_PROJECT_NAME=sanchoris_fix_health_abc123
BIND_HOST=127.10.20.30
DATABASE_URL=postgres://sanchoris:sanchoris@127.10.20.30:54329/sanchoris
# END WORKTRUNK ENV
```

Docker Compose reads `COMPOSE_PROJECT_NAME` and `BIND_HOST` from `.env`, so each worktree gets separate containers, networks, volumes, and loopback bind addresses while keeping service ports stable. portless detects linked git worktrees and prefixes the branch name into the hostname, so each worktree can run its own frontend and backend without port collisions.

## Observing Codex runs

Use the local runner when starting a Codex task that should leave inspectable logs:

```sh
node scripts/codex-run.mjs --prompt tasks/example.md --verify "pnpm check" -- \
  codex exec --ask-for-approval never < tasks/example.md
```

Each run creates a timestamped directory under `.sanchoris/codex-runs/` by default. The directory contains `prompt.md`, `stdout.log`, `stderr.log`, `combined.log`, `metadata.json`, `summary.json`, and per-verification logs such as `verify-01-stdout.log`.

Tail a running task:

```sh
tail -f .sanchoris/codex-runs/<run-id>/combined.log
```

Check the machine-readable result:

```sh
cat .sanchoris/codex-runs/<run-id>/summary.json
```

The runner only creates log directories and runs the explicit command after `--`; it does not clean worktrees, reset branches, or remove files.

## Checks

```sh
pnpm check
cargo check --workspace
```
