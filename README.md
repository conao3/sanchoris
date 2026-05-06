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

## Checks

```sh
pnpm check
cargo check --workspace
```
