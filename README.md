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

Dev targets a **neon** Postgres and **Cognito Hosted UI** login, so a few env vars
must be set first (see [Login and environment](#login-and-environment)). `pnpm dev`
fails fast with a clear message if any required var is missing.

The dev command applies `db/schema.sql` at startup, then starts the Rust backend on an internal free port, starts Vite through portless, and proxies `/api/*` through the same frontend host. By default, local development uses plain HTTP on an unprivileged port so it works without sudo or TLS setup.

## Login and environment

Login mirrors sanplan: Cognito Hosted UI (OAuth2 authorization-code, no Amplify),
stateless JWT verification in the backend, and just-in-time user provisioning into
`sanchoris.users`. The frontend redirects unauthenticated users to `/login`, which
sends them to the Cognito Hosted UI; the callback at `/auth/callback` exchanges the
code for tokens and stores them in `localStorage`. The ID token rides on every
`/api/graphql` request as `Authorization: Bearer <id_token>`; the backend verifies
it (RS256 + issuer + exp + client_id/aud) against the pool JWKS and upserts the user.

### Required environment

Copy the placeholders from [`.env.example`](.env.example) into the gitignored
repo-root `.envrc.local` (sourced by `.envrc` after `.env`, so it wins over the
worktrunk-managed `.env` block and the flake default). **Never commit real values.**

| Variable | Consumer | Source |
| --- | --- | --- |
| `DATABASE_URL` | backend | neon dashboard; include `?sslmode=require` |
| `COGNITO_USER_POOL_ID` | backend | aws-infra-k8s export `dev-k8s-UserPool` (region is derived from the pool-id prefix) |
| `COGNITO_CLIENT_ID` | backend | aws-infra-k8s export `dev-k8s-UserPoolClient` |
| `VITE_COGNITO_DOMAIN` | frontend | custom Hosted UI domain, host only — dev `dev-auth-k8s.sancode.dev` |
| `VITE_COGNITO_CLIENT_ID` | frontend | same value as `COGNITO_CLIENT_ID` |

Read the Cognito ids with:

```sh
aws cloudformation list-exports \
  --query "Exports[?Name=='dev-k8s-UserPool'||Name=='dev-k8s-UserPoolClient'].[Name,Value]" \
  --output text
```

Example `.envrc.local` additions (placeholders — fill from the sources above):

```sh
export DATABASE_URL="postgres://USER:PASS@HOST/DB?sslmode=require"   # neon
export COGNITO_USER_POOL_ID="ap-northeast-1_xxxxxxxxx"
export COGNITO_CLIENT_ID="xxxxxxxxxxxxxxxxxxxxxxxxxx"
export VITE_COGNITO_DOMAIN="dev-auth-k8s.sancode.dev"
export VITE_COGNITO_CLIENT_ID="$COGNITO_CLIENT_ID"
```

> **Prerequisite (separate repo).** The shared Cognito app client
> (`resource-user-pool-client` in aws-infra-k8s `cognito.clj`) must allow the
> sanchoris dev URLs before login can complete: add callback
> `http://sanchoris.localhost:1355/auth/callback` and logout
> `http://sanchoris.localhost:1355/login`, then redeploy the `dev-k8s-cognito`
> stack. Until then the Hosted UI rejects the `redirect_uri`.

### Offline dev with local docker Postgres

neon is the source of truth for dev. To work offline instead, leave `DATABASE_URL`
**unset** in `.envrc.local`; the flake then defaults it to the local docker Postgres,
which you start with:

```sh
docker compose up -d postgres
```

The default local `DATABASE_URL` is:

```text
postgres://sanchoris:sanchoris@127.0.0.1:54329/sanchoris
```

`docker-compose.yml` is retained for this offline path. The `devo.yaml` `db` pane
skips itself automatically when `DATABASE_URL` is set (neon).

### Applying the schema

`pnpm dev` applies `db/schema.sql` at startup. To apply it manually to whichever
`DATABASE_URL` is in effect (neon when set, local docker otherwise):

```sh
pnpm db:apply
```

`db/schema.sql` uses no extensions or `gen_random_uuid()` (the `users.id` UUID is
generated in the backend), so it applies cleanly to neon over TLS.

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
