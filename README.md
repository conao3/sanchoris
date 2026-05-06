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

Run the frontend and backend on one local development host:

```sh
pnpm dev
```

The dev command starts the Rust backend on an internal free port, starts Vite through portless, and proxies `/api/*` through the same frontend host. By default, local development uses plain HTTP on an unprivileged port so it works without sudo or TLS setup.

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
pnpm install
pnpm dev
```

portless detects linked git worktrees and prefixes the branch name into the hostname, so each worktree can run its own frontend and backend without port collisions.

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
