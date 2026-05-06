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

The dev command starts the Rust backend on an internal free port, starts Vite through portless, and proxies `/api/*` through the same frontend host.

- Main worktree: `https://sanchoris.localhost/admin/health`
- Linked worktree, e.g. branch `fix-health`: `https://fix-health.sanchoris.localhost/admin/health`

On first use, portless may ask to trust its local development CA. If HTTPS trust is not needed, run portless with plain HTTP:

```sh
PORTLESS_HTTPS=0 pnpm dev
```

In non-interactive environments where sudo prompts are unavailable, use an unprivileged port:

```sh
PORTLESS_HTTPS=0 PORTLESS_PORT=1355 pnpm dev
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
