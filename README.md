# Sanchoris

Sanchoris is a pnpm + Vite frontend and Rust backend monorepo.

## Layout

- `apps/frontend`: Vite frontend package managed by pnpm
- `apps/workspace-agent`: the agent that runs inside a Workspace, and the `sanchoris` command (`PROTOCOL.md` describes the Run channel)
- `crates/api`: the `sanchoris` binary (HTTP API and background worker)
- `crates/config`, `crates/engine`, `crates/core`, `crates/worker`: configuration, the skill engine, persistence, and the worker loops
- `scripts/dev.mjs`: local development process supervisor
- `scripts/e2e/fake-agent.ts`: the scripted workspace agent used by the end-to-end test

## Development

Enter the Nix development shell:

```sh
nix develop
```

Install the JavaScript dependencies:

```sh
pnpm install
```

Run the frontend, the api, and the worker on one local development host:

```sh
pnpm dev
```

Dev targets a **neon** Postgres, **Cognito Hosted UI** login, and a GitHub App, so the environment must be set first (see [Environment](#environment)). `pnpm dev` fails fast with the list of missing variables.

The dev command applies `db/schema.sql` at startup, then runs `sanchoris all` (through `cargo watch`) on an internal free port, starts Vite through portless, and proxies `/api`, `/auth/github`, `/webhooks`, `/records`, and `/runs` through the same frontend host. Local workspaces are processes started by the worker under `.sanchoris/dev/workspaces`, and session records are stored under `.sanchoris/dev/records`. By default, local development uses plain HTTP on an unprivileged port so it works without sudo or TLS setup.

A local workspace runs `apps/workspace-agent` with Claude Code, so the dev environment also needs `CLAUDE_CONFIG_DIR` and `SANCHORIS_CLAUDE_PATH` (see `apps/workspace-agent/PROTOCOL.md`) and a `sanchoris` command on `PATH`.

## The `sanchoris` binary

```sh
cargo run -p sanchoris-api --bin sanchoris -- serve   # HTTP API
cargo run -p sanchoris-api --bin sanchoris -- worker  # background loops
cargo run -p sanchoris-api --bin sanchoris -- all     # both in one process
```

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | Process health check |
| `GET /api/me` | The signed-in user (Cognito ID or access token as `Authorization: Bearer`; the user is created on first use) |
| `POST /api/graphql` | The GraphQL API of the web app (`crates/graphql`). Needs the same bearer token; requests without one get `401` |
| `POST /api/tasks/{taskId}/replies` | A member's reply to a task (`{ "text": "..." }`), delivered to the agent and resuming a `NEEDS_YOU` task |
| `POST /api/approvals` | Approve or reject an approval request (`{ "token": "<signed task token>", "approved": true }`) |
| `POST /auth/github` | Starts linking the signed-in user's GitHub account; returns `{ "url": "<GitHub authorize URL>" }` (the `connectGitHub` mutation returns the same URL) |
| `GET /auth/github/setup` | GitHub App Setup URL. Checks the signed `state` from `Tenant.githubAppInstallUrl` and sends the user through the GitHub OAuth authorize page to prove access to the new installation |
| `GET /auth/github/callback` | GitHub App user-to-server OAuth callback. For account linking it stores `github_user_id` and `github_login` and records `integration.github_link`; for an installation it checks the installation is in the user's `GET /user/installations`, stores it in `integrations` with its repositories, and records `integration.github_install`. Redirects to `/settings?github=<outcome>` (`linked`, `installed`, `requested`, `denied`, `in_use`, `installation_in_use`, `not_accessible`, `not_member`, `not_admin`) |
| `POST /webhooks/github` | GitHub App webhook. Verifies `X-Hub-Signature-256`; `installation` and `installation_repositories` events update `integrations`, every other event is recorded once per `X-GitHub-Delivery` |
| `POST /webhooks/stripe` | Stripe webhook. Verifies `Stripe-Signature` (5 minute tolerance) and handles `checkout.session.completed` (activates the tenant on `metadata.plan_id`), `customer.subscription.updated`, `customer.subscription.deleted`, `invoice.paid`, and `invoice.payment_failed` |
| `GET /runs/channel` | Run channel WebSocket for `workspace-agent` (bearer Run token; see `apps/workspace-agent/PROTOCOL.md`) |
| `PUT`/`GET /records/{key}` | Signed session record upload and download when `SANCHORIS_RECORDS_STORE=local` |

## Environment

Every `.env*` file is treated as secret and gitignored; only `*.example` is committed. Copy the template into the gitignored repo-root `.env.local` and fill it in:

```sh
cp .env.example .env.local
```

`.envrc` loads `.env.local` after `use flake`, so its values win over the worktrunk-managed `.env` block and the flake default; `scripts/dev.mjs` reads it too. **Only `.env.example` is committed; never commit real values.** The binary has no defaults: it stops with the list of missing variables.

| Variable | Used by | Value |
| --- | --- | --- |
| `DATABASE_URL` | serve, worker | Postgres, direct connection (not the neon `-pooler` host); neon needs `?sslmode=require`. The role must be a member of `sanchoris_worker` and be allowed to `SET ROLE sanchoris_app` (`pnpm db:apply` grants both) |
| `SANCHORIS_BIND_ADDR` | serve | Listen address, e.g. `127.0.0.1:8080` (set by `pnpm dev`) |
| `SANCHORIS_PUBLIC_URL` | serve | Public origin, e.g. `http://sanchoris.localhost:1355`; OAuth redirects and local record URLs start with it |
| `COGNITO_USER_POOL_ID`, `COGNITO_CLIENT_ID` | serve | aws-infra-k8s exports `dev-k8s-UserPool` and `dev-k8s-UserPoolClient` (the region is the pool-id prefix) |
| `VITE_COGNITO_DOMAIN`, `VITE_COGNITO_CLIENT_ID` | frontend | Hosted UI domain (host only, dev `dev-auth-k8s.sancode.dev`) and the same client id |
| `GITHUB_URL`, `GITHUB_API_URL` | serve, worker | `https://github.com` and `https://api.github.com` |
| `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY` | serve, worker | GitHub App id and private key (PEM) |
| `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET` | serve | GitHub App OAuth client for account linking and installation checks |
| `GITHUB_APP_SLUG` | serve | The App slug in `https://github.com/apps/<slug>`; the install URL is `<GITHUB_URL>/apps/<slug>/installations/new`. Set the App's Setup URL to `<SANCHORIS_PUBLIC_URL>/auth/github/setup` |
| `GITHUB_WEBHOOK_SECRET` | serve | GitHub App webhook secret |
| `STRIPE_API_URL` | serve | `https://api.stripe.com` |
| `STRIPE_SECRET_KEY` | serve | Stripe secret key (Checkout, the customer portal, subscription changes) |
| `STRIPE_WEBHOOK_SECRET` | serve | Stripe webhook signing secret |
| `RUN_TOKEN_SIGNING_KEY` | serve, worker | Signs Run channel tokens (32+ bytes) |
| `TASK_TOKEN_SIGNING_KEY` | serve | Signs approval tokens (32+ bytes) |
| `OAUTH_STATE_SIGNING_KEY` | serve | Signs the GitHub OAuth and installation `state`, bound to the signed-in user and tenant (32+ bytes) |
| `SANCHORIS_CREDENTIAL_KEY` | serve | AES-256-GCM key for stored API keys: 32 bytes in base64 (`openssl rand -base64 32`) |
| `SANCHORIS_RECORDS_STORE` | serve | `s3` or `local` |
| `AWS_REGION`, `S3_RECORDS_BUCKET`, `KMS_KEY_PREFIX` | serve (`s3`) | Records bucket; objects are encrypted with the KMS key `<KMS_KEY_PREFIX><tenant id>` |
| `SANCHORIS_RECORDS_DIR`, `RECORDS_SIGNING_KEY` | serve (`local`) | Record directory (set by `pnpm dev`) and the URL signing key (32+ bytes) |
| `SANCHORIS_WORKSPACE_COMMAND` | serve, worker | Command that starts the workspace agent (set by `pnpm dev`) |
| `SANCHORIS_CONTROL_URL` | serve, worker | Run channel URL given to workspaces, e.g. `ws://127.0.0.1:8080/runs/channel` (set by `pnpm dev`) |
| `SANCHORIS_SOCKET_DIR`, `SANCHORIS_WORKSPACES_DIR` | serve, worker | Directories for the `sanchoris` command sockets and the workspaces (set by `pnpm dev`) |
| `SANCHORIS_GIT_URL` | serve, worker | Base URL the workspaces clone `<owner>/<name>.git` from, e.g. `https://github.com` |

`serve` reads the worker variables too: the GraphQL API runs the trigger dispatcher and the config syncer inline (below).

### GraphQL API

`POST /api/graphql` runs the `crates/graphql` schema behind the Cognito JWT check. Queries and mutations use a second pool on the same `DATABASE_URL` whose connections run `SET ROLE sanchoris_app`, so row-level security limits every statement to the signed-in user's tenants; webhooks, OAuth, and the worker keep the `sanchoris_worker` pool. Because the role is set per connection, `DATABASE_URL` must be a direct connection, and `serve` refuses a neon `-pooler` host.

The schema's ports are wired as follows:

| Port | Implementation |
| --- | --- |
| Checkout | Stripe API. A paid `createTenant` opens a Checkout Session (subscription mode, `client_reference_id` = the new tenant id, `metadata.plan_id`, the plan's `stripe_price_id`) and creates the tenant suspended with `payment_pending`; `checkout.session.completed` activates it. `changePlan` swaps the price of the active subscription, cancels it for the Free plan, or opens Checkout when the tenant has no subscription. `openBillingPortal` opens a customer portal session |
| GitHub | `connectGitHub` returns the OAuth authorize URL; `Tenant.githubAppInstallUrl` returns the App install URL with a signed `state` (see `/auth/github/setup`) |
| Slack | Not available: `connectSlack` fails with `BAD_USER_INPUT` "Slack is not available yet" |
| Config sync | `addConfig` and `syncConfig` run the config syncer for that config inline |
| Config repository | `addConfig` reads the branch head through the GitHub App and validates it; `proposeTriggerFromTemplate` creates a branch, commits the trigger into `sanchoris.yaml` (Contents API), and opens a pull request |
| Chat | `sendChatMessage` runs the trigger dispatcher for the tenant inline, so the new task is in the response |
| Audit export | Writes `t/<tenant>/audit/<file>.jsonl` to the records store (S3 or local) and returns a signed GET URL (15 minutes) |
| Credentials | AES-256-GCM with `SANCHORIS_CREDENTIAL_KEY`, bound to the tenant id |

Read the Cognito ids with:

```sh
aws cloudformation list-exports \
  --query "Exports[?Name=='dev-k8s-UserPool'||Name=='dev-k8s-UserPoolClient'].[Name,Value]" \
  --output text
```

### Login

Login mirrors sanplan: Cognito Hosted UI (OAuth2 authorization-code, no Amplify), stateless JWT verification in the api, and just-in-time user provisioning into `sanchoris.users`. The frontend redirects unauthenticated users to `/login`, which sends them to the Cognito Hosted UI; the callback at `/auth/callback` exchanges the code for tokens and stores them in `localStorage`. The ID token rides on every `/api/*` request as `Authorization: Bearer <id_token>`; the api verifies it (RS256 + issuer + exp + client_id/aud) against the pool JWKS and upserts the user. GitHub accounts are linked separately from Settings → Integrations through `/auth/github`.

> **Prerequisite (separate repo).** The shared Cognito app client
> (`resource-user-pool-client` in aws-infra-k8s `cognito.clj`) must allow the
> sanchoris dev URLs before login can complete: add callback
> `http://sanchoris.localhost:1355/auth/callback` and logout
> `http://sanchoris.localhost:1355/login`, then redeploy the `dev-k8s-cognito`
> stack. Until then the Hosted UI rejects the `redirect_uri`.

### Offline dev with local docker Postgres

neon is the source of truth for dev. To work offline instead, leave `DATABASE_URL` **unset** in `.env.local`; the flake then defaults it to the local docker Postgres, which you start with:

```sh
docker compose up -d postgres
```

The default local `DATABASE_URL` is:

```text
postgres://sanchoris:sanchoris@127.0.0.1:54329/sanchoris
```

`docker-compose.yml` is retained for this offline path. The `devo.yaml` `db` pane skips itself automatically when `DATABASE_URL` is set (neon).

### Applying the schema

`pnpm dev` applies `db/schema.sql` at startup. To apply it manually to whichever `DATABASE_URL` is in effect (neon when set, local docker otherwise):

```sh
pnpm db:apply
```

`db:apply` also grants `sanchoris_worker` to the connecting role, which the api and the worker need to read across tenants under row-level security, and grants `sanchoris_app` with `INHERIT FALSE, SET TRUE` so the GraphQL pool can switch to it without the worker pool picking up its policies.

The schema source of truth is `db/schema.sql`. `pnpm db:schema:check` starts a temporary PostgreSQL server, applies the schema, and verifies the required MVP tables exist.

- Main worktree: `http://sanchoris.localhost:1355/`
- Linked worktree, e.g. branch `fix-health`: `http://fix-health.sanchoris.localhost:1355/`

The frontend GraphQL documents live in `apps/frontend/src/graphql/` (`schema.graphql`, `mvp-shell.graphql`, and the GraphQL Code Generator output in `generated/`).

If you want clean HTTPS URLs without port numbers, start portless with TLS explicitly:

```sh
PORTLESS_HTTPS=1 PORTLESS_PORT=443 pnpm dev
```

## End-to-end test

```sh
nix develop -c pnpm e2e
```

The test (`crates/api/tests/e2e.rs`) starts a temporary Postgres, a fake GitHub (REST, GraphQL, and bare git repositories on disk), and `sanchoris all`. The worker starts the real `apps/workspace-agent` (with the scripted worker in `scripts/e2e/fake-agent.ts`) through `LocalProcessRunner`. It covers:

- A signed `issues.labeled` webhook starts the task; the agent commits and runs `sanchoris skill run open-pr-green-ci`; the skill pushes the branch to the bare repository, opens a pull request, hands the failing check back to the agent, which fixes and continues; the passing check comments on the issue and the task succeeds.
- Checks that keep failing put the task in `NEEDS_YOU` (`Skill.FixesExhausted`); a member's reply resumes it and the second skill run succeeds.

Run `pnpm install` first; the workspace agent runs from source with Node.

`crates/api/tests/graphql.rs` (part of `cargo test --workspace`) drives `POST /api/graphql` over HTTP with a stub token verifier and runs every operation in `apps/frontend/src/graphql/*.graphql` against the real ports: the config syncer and trigger dispatcher inline, the GitHub App client against the fake GitHub, Stripe against a fake Stripe API, and local records. It fails when an operation is added to the frontend without being exercised.

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

Docker Compose reads `COMPOSE_PROJECT_NAME` and `BIND_HOST` from `.env`, so each worktree gets separate containers, networks, volumes, and loopback bind addresses while keeping service ports stable. portless detects linked git worktrees and prefixes the branch name into the hostname, so each worktree can run its own frontend and api without port collisions. Set `SANCHORIS_PUBLIC_URL` to the worktree's origin.

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
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
pnpm e2e
```
