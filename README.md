# Sanchoris

Sanchoris is a pnpm + Vite frontend and Rust backend monorepo.

## Layout

- `apps/frontend`: Vite frontend package managed by pnpm
- `apps/workspace-agent`: the agent that runs inside a Workspace, and the `sanchoris` command (`PROTOCOL.md` describes the Run channel)
- `apps/egress-relay`: the Workspaces' egress proxy (`CONNECT` to port 443 after the operator deny list, the platform hosts, and the Run's `Network.Allow`)
- `infra/workspace/Dockerfile`, `infra/relay/Dockerfile`: the Workspace image (`linux/arm64` and `linux/amd64`, built without emulation: every `RUN` step runs on the build platform and installs the target's packages) and the relay image (`linux/arm64`; it only copies files)
- `crates/api`: the HTTP API, the Run channel, and their Lambda functions (`api`, `channel`, `channel-authorizer`, `worker`, `relay`), and the `sanchoris` binary that runs everything in one process for local development
- `crates/config`, `crates/engine`, `crates/core`, `crates/worker`: configuration, the skill engine, persistence (DynamoDB, S3, and the queue and scheduler ports), and the worker's message handlers
- `crates/graphql`: the GraphQL schema of the web app (`apps/frontend/src/graphql/schema.graphql`) over the DynamoDB store
- `docs/architecture.md`: the serverless AWS architecture and the DynamoDB data model
- `infra/tables.json`, `infra/plans.json`: the DynamoDB table definitions (CloudFormation properties) and the plan catalog, read by `sanchoris-dynamo-create` and the tests
- `scripts/dev.mjs`: local development process supervisor
- `scripts/build-lambda.mjs`: builds the frontend and the Lambda zips (`pnpm build:lambda`)
- `scripts/verify.sh`: the checks (`quick` for the changed crates and apps, `full` for everything)
- `scripts/deploy.mjs`, `scripts/e2e-aws.mjs`: deploy to an AWS environment and run an issue through it end to end (`AGENTS.md`)
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

Dev targets DynamoDB Local, **Cognito Hosted UI** login, and a GitHub App, so the environment must be set first (see [Environment](#environment)). `pnpm dev` fails fast with the list of missing variables.

The dev command creates the DynamoDB tables at startup (`pnpm dynamo:create`), builds the frontend for the api's fallback (`apps/frontend/dist`), then runs `sanchoris all` (through `cargo watch`) on an internal free port, starts Vite through portless, and proxies `/api`, `/auth/github`, `/webhooks`, `/records`, and `/runs` through the same frontend host. Local workspaces are processes started by the worker under `.sanchoris/dev/workspaces`, and session records are stored under `.sanchoris/dev/records`. Vite registers its route with the host's portless proxy, which serves HTTPS on port 443 with a locally trusted CA and must already run as a system service. `portless run` (0.15) probes a proxy over plain HTTP unless `PORTLESS_STATE_DIR` names its state directory, so set `PORTLESS_STATE_DIR` to the directory the system proxy uses (for example `~/.portless`); otherwise it stops with "Proxy is not running":

- Main worktree: `https://sanchoris.localhost/`
- Linked worktree, e.g. branch `fix-health`: `https://fix-health.sanchoris.localhost/`

A local workspace runs `apps/workspace-agent` with Claude Code, so the dev environment also needs `CLAUDE_CONFIG_DIR` and `SANCHORIS_CLAUDE_PATH` (see `apps/workspace-agent/PROTOCOL.md`) and a `sanchoris` command on `PATH`.

## The `sanchoris` binary

```sh
cargo run -p sanchoris-api --bin sanchoris -- all
```

`sanchoris all` runs the HTTP API, the Run channel WebSocket, the worker (`LocalDriver` over the in-memory queue and scheduler), and the built frontend from `SANCHORIS_FRONTEND_DIR` in one process against DynamoDB Local, with offloaded attributes under `SANCHORIS_DOCS_DIR`. In AWS the same code runs as Lambda functions:

```sh
VITE_COGNITO_DOMAIN=dev-auth-k8s.sancode.dev VITE_COGNITO_CLIENT_ID=<client id> \
  nix develop -c pnpm build:lambda
```

builds the frontend into `apps/frontend/dist`, then `target/lambda/{api,channel,channel-authorizer,worker,relay}/bootstrap.zip` for `provided.al2023` on `arm64` (static `aarch64-unknown-linux-musl` binaries cross-compiled by `cargo lambda` with the devShell's musl cross linker). The `api` binary embeds `apps/frontend/dist` and serves it on the same origin as the API, so the deployment needs no separate static hosting; it refuses to start when the build embedded no `index.html`. `docs/architecture.md` ("Lambda functions") lists each function's trigger, handler, variables, and IAM actions.

| Endpoint | Purpose |
| --- | --- |
| `GET /health` | Process health check |
| `GET /api/me` | The signed-in user (Cognito ID or access token as `Authorization: Bearer`; the user is created on first use) |
| `POST /api/graphql` | The GraphQL API of the web app (`crates/graphql`). Needs the same bearer token; requests without one get `401` |
| `POST /api/tasks/{taskId}/replies` | A member's reply to a task (`{ "text": "..." }`), delivered to the agent and resuming a `NEEDS_YOU` task |
| `POST /api/approvals` | Approve or reject an approval request (`{ "token": "<signed task token>", "approved": true }`). Owners, admins, and the member who requested the task may resolve it (others get 403); each decision writes an `approval.resolve` audit event |
| `POST /auth/github` | Starts linking the signed-in user's GitHub account; returns `{ "url": "<GitHub authorize URL>" }` (the `connectGitHub` mutation returns the same URL) |
| `GET /auth/github/setup` | GitHub App Setup URL. Checks the signed `state` from `Tenant.githubAppInstallUrl` and sends the user through the GitHub OAuth authorize page to prove access to the new installation |
| `GET /auth/github/callback` | GitHub App user-to-server OAuth callback. For account linking it stores `github_user_id` and `github_login` and records `integration.github_link`; for an installation it checks the installation is in the user's `GET /user/installations`, stores it in `integrations` with its repositories, and records `integration.github_install`. Redirects to `/settings?github=<outcome>` (`linked`, `installed`, `requested`, `denied`, `in_use`, `installation_in_use`, `not_accessible`, `not_member`, `not_admin`) |
| `POST /webhooks/github` | GitHub App webhook. Verifies `X-Hub-Signature-256`; `installation`, `installation_repositories`, and `repository` events update `integrations` and follow the configs by repository id (a rename updates the cached names; a removal from the installation or a deleted repository disables the config, `docs/architecture.md` "GitHub repositories"), a `push` to the branch of an active config (matched by `repository.id`) queues its `ConfigSync` at once, and every other event is recorded once per `X-GitHub-Delivery` |
| `POST /webhooks/stripe` | Stripe webhook. Verifies `Stripe-Signature` (5 minute tolerance), applies each event id once, and handles `checkout.session.completed` and `checkout.session.async_payment_succeeded` (activate the tenant on `metadata.plan_id` when `payment_status` is `paid`), `checkout.session.async_payment_failed` and `checkout.session.expired` (discard the tenant that waits for the checkout), `customer.subscription.updated` (`active`, `trialing`, `past_due` only), `customer.subscription.deleted`, `invoice.paid` (resets the period for `subscription_create` and `subscription_cycle`), and `invoice.payment_failed`. The Stripe endpoint must subscribe to all nine events. Events for a tenant that is gone or not payable are answered 200 with `{"status": "ignored", "reason": ...}` |
| `GET /runs/channel` | Run channel WebSocket for `workspace-agent` in `sanchoris all` (bearer Run token; see `apps/workspace-agent/PROTOCOL.md`). In AWS the API Gateway WebSocket API serves the same protocol |
| `PUT`/`GET /records/{key}` | Signed session record upload and download when `SANCHORIS_RECORDS_STORE=local` |
| `GET` any other path | The built frontend: a file of `apps/frontend/dist` (`assets/` with an immutable cache), `index.html` for client routes without a file extension, `404` for missing files and for unknown paths under `/api`, `/webhooks`, `/auth/github`, `/records`, and `/runs` |

## Environment

Every `.env*` file is treated as secret and gitignored; only `*.example` is committed. Copy the template into the gitignored repo-root `.env.local` and fill it in:

```sh
cp .env.example .env.local
```

`.envrc` loads `.env.local` after `use flake`, so its values win over the worktrunk-managed `.env` block; `scripts/dev.mjs` reads it too. **Only `.env.example` is committed; never commit real values.** The binary has no defaults: it stops with the list of missing variables.

Secrets (the rows marked *secret*) come from the environment in `sanchoris all` and from SSM Parameter Store in the Lambda functions: each function reads the parameters it needs once at cold start from `<SANCHORIS_SECRETS_PATH><name>` (String or SecureString) and stops with the list of missing parameters. The parameter names are in `docs/architecture.md` ("Secrets").

| Variable | Used by | Value |
| --- | --- | --- |
| `SANCHORIS_TABLE_MAIN`, `SANCHORIS_TABLE_AUDIT` | api, channel-authorizer, channel, worker, `sanchoris all` | DynamoDB table names (`docs/architecture.md`) |
| `AWS_REGION`, `AWS_ENDPOINT_URL_DYNAMODB` | all | Region of the tables; the endpoint points at DynamoDB Local in development and is unset in AWS |
| `SANCHORIS_DOCS_BUCKET` | api, channel-authorizer, channel, worker (Lambda) | Bucket for attributes larger than 32 KiB (config revisions, skill bodies, large events) |
| `SANCHORIS_SECRETS_PATH` | api, channel-authorizer, channel, worker (Lambda) | SSM path of the secrets, e.g. `/dev-k8s/sanchoris/` |
| `SANCHORIS_WORK_QUEUE_URL`, `SANCHORIS_WORK_QUEUE_ARN`, `SANCHORIS_SCHEDULE_GROUP`, `SANCHORIS_SCHEDULER_ROLE_ARN` | relay | The SQS FIFO work queue and the EventBridge Scheduler group and role that deliver delayed jobs to it |
| `SANCHORIS_BIND_ADDR` | `sanchoris all` | Listen address, e.g. `127.0.0.1:8080` (set by `pnpm dev`) |
| `SANCHORIS_FRONTEND_DIR` | `sanchoris all` | Built frontend (`apps/frontend/dist`, built and set by `pnpm dev`); the api Lambda embeds it instead |
| `SANCHORIS_PUBLIC_URL` | api | Public origin, e.g. `https://sanchoris.localhost`; OAuth redirects, Stripe return URLs, and local record URLs start with it |
| `COGNITO_USER_POOL_ID`, `COGNITO_CLIENT_ID` | api | aws-infra-k8s exports `dev-k8s-UserPool` and `dev-k8s-SanchorisUserPoolClient` (the region is the pool-id prefix) |
| `VITE_COGNITO_DOMAIN`, `VITE_COGNITO_CLIENT_ID` | frontend (`pnpm dev`, `pnpm build:lambda`) | Hosted UI domain (host only, dev `dev-auth-k8s.sancode.dev`) and the same client id |
| `GITHUB_URL` | api | `https://github.com` |
| `GITHUB_API_URL` | api, channel, worker | `https://api.github.com` |
| `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY` | api, channel, worker (*secret*) | GitHub App id and private key (PEM) |
| `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET` | api (*secret*) | GitHub App OAuth client for account linking and installation checks |
| `GITHUB_APP_SLUG` | api (*secret*) | The App slug in `https://github.com/apps/<slug>`; the install URL is `<GITHUB_URL>/apps/<slug>/installations/new`. Set the App's Setup URL to `<SANCHORIS_PUBLIC_URL>/auth/github/setup` |
| `GITHUB_WEBHOOK_SECRET` | api (*secret*) | GitHub App webhook secret |
| `STRIPE_API_URL` | api | `https://api.stripe.com` |
| `STRIPE_PORTAL_CONFIGURATION` | api | Id of the Stripe customer portal configuration (`bpc_...`) that every portal session names; the Stripe account is shared with other products |
| `SANCHORIS_STRIPE_PRICE_STARTER`, `SANCHORIS_STRIPE_PRICE_PRO`, `SANCHORIS_STRIPE_PRICE_MAX` | `sanchoris-dynamo-create` (`pnpm dynamo:create`, `pnpm dev`) | Stripe price ids (`price_...`) written into the paid plans of `PLANS`; seeding stops when one is missing |
| `STRIPE_SECRET_KEY` | api (*secret*) | Stripe secret key (Checkout, the customer portal, subscription changes) |
| `STRIPE_WEBHOOK_SECRET` | api (*secret*) | Stripe webhook signing secret |
| `RUN_TOKEN_SIGNING_KEY` | channel, channel-authorizer, worker (*secret*) | Signs Run channel tokens (32+ bytes) |
| `TASK_TOKEN_SIGNING_KEY` | api (*secret*) | Signs approval tokens (32+ bytes) |
| `OAUTH_STATE_SIGNING_KEY` | api (*secret*) | Signs the GitHub OAuth and installation `state`, bound to the signed-in user and tenant (32+ bytes) |
| `SANCHORIS_CREDENTIAL_KEY` | api, channel (*secret*) | AES-256-GCM key for stored API keys: 32 bytes in base64 (`openssl rand -base64 32`) |
| `SANCHORIS_RECORDS_STORE` | api, channel, worker | `s3` or `local` (Lambda functions take `s3`) |
| `SANCHORIS_RECORDS_BUCKET` | api, channel, worker (`s3`) | Bucket of session records and audit exports (SSE-S3 by default) |
| `SANCHORIS_RECORDS_DIR`, `RECORDS_SIGNING_KEY` | `sanchoris all` (`local`) | Record directory (set by `pnpm dev`) and the URL signing key (32+ bytes) |
| `SANCHORIS_WORKSPACE_COMMAND` | `sanchoris all` | Command that starts the workspace agent (set by `pnpm dev`) |
| `SANCHORIS_CONTROL_URL` | `sanchoris all`, worker | Run channel URL given to workspaces: locally `ws://127.0.0.1:8080/runs/channel` (set by `pnpm dev`), in AWS the `wss://` URL of the WebSocket API |
| `SANCHORIS_SOCKET_DIR`, `SANCHORIS_WORKSPACES_DIR` | `sanchoris all` | Directories for the `sanchoris` command sockets and the workspaces (set by `pnpm dev`) |
| `SANCHORIS_GIT_URL` | worker | Base URL the workspaces clone `<owner>/<name>.git` from, e.g. `https://github.com` |
| `SANCHORIS_DOCS_DIR` | `sanchoris all` | Directory for attributes larger than 32 KiB (set by `pnpm dev`) |
| `SANCHORIS_CHANNEL_ENDPOINT` | channel, worker | API Gateway Management API endpoint of the WebSocket stage: `https://<channel custom domain>` when the stack has custom domains (the `execute-api` endpoint is disabled then), otherwise `https://<api id>.execute-api.<region>.amazonaws.com/prod` |
| `SANCHORIS_ECS_CLUSTER`, `SANCHORIS_ECS_TASK_DEFINITION_ARM64`, `SANCHORIS_ECS_TASK_DEFINITION_X86_64`, `SANCHORIS_ECS_CONTAINER`, `SANCHORIS_ECS_SUBNETS`, `SANCHORIS_ECS_SECURITY_GROUPS`, `SANCHORIS_ECS_ASSIGN_PUBLIC_IP` | worker | Fargate cluster, the task definition families for `arm64` and `x86_64` Sessions, container name, the comma-separated subnets and security groups of Workspaces, and `ENABLED` or `DISABLED` for their public IP |
| `SANCHORIS_RELAY_SERVICE`, `SANCHORIS_RELAY_PORT` | worker | ECS service of the egress relay (`apps/egress-relay`) in the same cluster and its port; the worker scales the service between 0 and 1 and puts the relay task's private IP into the Workspaces' `HTTPS_PROXY` |
| `RELAY_TOKEN_SIGNING_KEY` | worker (*secret*) | Signs the relay tokens that carry each Run's `Network.Allow` (32+ bytes) |

`sanchoris all` reads every variable of the api and the worker except the AWS-only ones. The functions and their variables and IAM actions are listed in `docs/architecture.md` ("Lambda functions").

### GraphQL API

`POST /api/graphql` runs the `crates/graphql` schema behind the Cognito JWT check. Queries and mutations run in a `TenantTx` of a tenant the signed-in user is a member of, which rejects every key outside that tenant's partitions (`docs/architecture.md`, "Tenant isolation").

The schema's ports are wired as follows:

| Port | Implementation |
| --- | --- |
| Checkout | Stripe API. A paid `createTenant` opens a Checkout Session (subscription mode, `client_reference_id` = the new tenant id, `metadata.plan_id`, `subscription_data.metadata.tenant_id`, the plan's `stripe_price_id`, `expires_at` one hour ahead) and creates the tenant suspended with `payment_pending`; a paid `checkout.session.completed` activates it. A user has one such tenant at a time: the next paid `createTenant` expires the open session and discards the waiting tenant. `changePlan` swaps the price of an `active` or `trialing` subscription, returns the customer portal URL when the subscription is `past_due`, `unpaid`, or `incomplete`, cancels it for the Free plan, or opens Checkout when the tenant has no subscription. `openBillingPortal` opens a customer portal session. A tenant on the Admin plan (created by a verified `@sancode.dev` email; `docs/architecture.md` "Plans") has no Checkout, no subscription, no plan change, and no portal |
| GitHub | `connectGitHub` returns the OAuth authorize URL; `Tenant.githubAppInstallUrl` returns the App install URL with a signed `state` (see `/auth/github/setup`) |
| Slack | Not available: `connectSlack` fails with `BAD_USER_INPUT` "Slack is not available yet" |
| Config sync | `addConfig` and `syncConfig` run the config syncer for that config inline, in the api process or the api Lambda (GitHub reads and DynamoDB writes, within the API Gateway 29 s limit) |
| Config repository | `addConfig` reads the branch head through the GitHub App and validates it; `proposeTriggerFromTemplate` creates a branch, commits the trigger into `sanchoris.yaml` (Contents API), and opens a pull request |
| Chat | `sendChatMessage` runs the trigger dispatcher for the tenant inline, so the new task is in the response; starting the Workspace (`Reconcile`) stays with the worker |
| Audit export | Writes `t/<tenant>/audit/<file>.jsonl` to the records store (the records bucket or the local directory) and returns a signed GET URL (15 minutes) |
| Credentials | AES-256-GCM with `SANCHORIS_CREDENTIAL_KEY`, bound to the tenant id |

Read the Cognito ids with:

```sh
aws cloudformation list-exports \
  --query "Exports[?Name=='dev-k8s-UserPool'||Name=='dev-k8s-SanchorisUserPoolClient'].[Name,Value]" \
  --output text
```

### Login

Login mirrors sanplan: Cognito Hosted UI (OAuth2 authorization-code, no Amplify), stateless JWT verification in the api, and just-in-time user provisioning (the `USER#` items in `docs/architecture.md`). The frontend redirects unauthenticated users to `/login`, which sends them to the Cognito Hosted UI; the callback at `/auth/callback` exchanges the code for tokens and stores them in `localStorage`. The ID token rides on every `/api/*` request as `Authorization: Bearer <id_token>`; the api verifies it (RS256 + issuer + exp + client_id/aud) against the pool JWKS and upserts the user. GitHub accounts are linked separately from Settings → Integrations through `/auth/github`.

> **Prerequisite (separate repo).** The sanchoris app client of the shared
> user pool (aws-infra-k8s `cognito.clj`, export
> `dev-k8s-SanchorisUserPoolClient`) allows callback
> `<origin>/auth/callback` and logout `<origin>/login` for
> `https://sanchoris.localhost` and `https://dev-sanchoris.sancode.dev`. The
> Hosted UI rejects every other `redirect_uri`, so sign in on the main
> worktree's origin.

### DynamoDB Local

Development runs against DynamoDB Local from the nix devShell (`dynamodb-local`, a Java jar with a bundled JRE). Start it on the worktree's port and create the tables:

```sh
pnpm dynamo:local
pnpm dynamo:create
```

`dynamo:local` keeps its data under `.sanchoris/dynamodb`. `dynamo:create` (`sanchoris-dynamo-create`) creates the tables declared in `infra/tables.json` when they are missing and writes the plans from `infra/plans.json` with the Stripe prices from `SANCHORIS_STRIPE_PRICE_STARTER`, `SANCHORIS_STRIPE_PRICE_PRO`, and `SANCHORIS_STRIPE_PRICE_MAX`. Point the SDK at DynamoDB Local with `AWS_ENDPOINT_URL_DYNAMODB` (the worktrunk block sets it); DynamoDB Local accepts any credentials, so set `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` to any value when no AWS profile is configured. The `devo.yaml` `db` pane runs `dynamo:local`.

In AWS the stack creates the tables, and `scripts/deploy.mjs` seeds `PLANS` on each deploy before it changes the functions, so a plan attribute that new code reads is in place when the code starts (the code before ignores attributes it does not read). By hand, with the deployed table names and the prices from SSM:

```sh
price() { aws ssm get-parameter --name "/dev-k8s/sanchoris/stripe-price-$1" --query Parameter.Value --output text; }
SANCHORIS_TABLE_MAIN=<main table> SANCHORIS_TABLE_AUDIT=<audit table> \
  SANCHORIS_STRIPE_PRICE_STARTER=$(price starter) \
  SANCHORIS_STRIPE_PRICE_PRO=$(price pro) \
  SANCHORIS_STRIPE_PRICE_MAX=$(price max) \
  cargo run -q -p sanchoris-core --bin sanchoris-dynamo-create
```

The Rust tests (`sanchoris_core::testing::TestDynamo`) connect to one of the DynamoDB Local instances whose ports `scripts/verify.sh` exports in `SANCHORIS_TEST_DYNAMO_PORTS` (the one of the test's nextest slot), or start their own in-memory instance on a free port when the variable is unset, and create uniquely named tables per test.

The frontend GraphQL documents live in `apps/frontend/src/graphql/` (`schema.graphql`, `mvp-shell.graphql`, and the GraphQL Code Generator output in `generated/`).

## End-to-end test

```sh
nix develop -c pnpm e2e
```

The test (`crates/api/tests/e2e.rs`) starts DynamoDB Local, a fake GitHub (REST, GraphQL, and bare git repositories on disk), and the same assembly as `sanchoris all` in process (`server::LocalServer` with a stub token verifier, so the member reply goes through `POST /api/tasks/{taskId}/replies`). The worker starts the real `apps/workspace-agent` (with the scripted worker in `scripts/e2e/fake-agent.ts`) through `LocalProcessRunner`. It covers:

- A signed `issues.labeled` webhook starts the task; the agent commits and runs `sanchoris skill run open-pr-green-ci`; the skill pushes the branch to the bare repository, opens a pull request, hands the failing check back to the agent, which fixes and continues; the passing check comments on the issue and the task succeeds.
- Checks that keep failing put the task in `NEEDS_YOU` (`Skill.FixesExhausted`); a member's reply resumes it and the second skill run succeeds.

Run `pnpm install` first; the workspace agent runs from source with Node.

`crates/api/tests/http.rs` covers the HTTP endpoints and the local Run channel (webhook signatures, duplicate deliveries, installation events, Stripe, replies and approvals, records, outbox delivery, sequence deduplication, and the close on an out-of-order message). `crates/api/tests/lambda.rs` drives the Lambda handlers with recorded API Gateway WebSocket and authorizer events, SQS batches, and DynamoDB Streams records (`crates/api/tests/fixtures/`), with a recording connection sender in place of the Management API.

`crates/api/tests/graphql.rs` (part of `cargo test --workspace`) drives `POST /api/graphql` over HTTP with a stub token verifier and runs every operation in `apps/frontend/src/graphql/*.graphql` against the real ports: the config syncer and trigger dispatcher inline, the GitHub App client against the fake GitHub, Stripe against a fake Stripe API, and local records. It fails when an operation is added to the frontend without being exercised.

## Worktrees

Use Worktrunk to manage parallel worktrees:

```sh
wt switch -c fix-health
pnpm dynamo:local &
pnpm dev
```

Worktrunk runs `scripts/worktrunk-env-sync.mjs` for new worktrees. The script updates only the managed block in the gitignored `.env` file:

```dotenv
# BEGIN WORKTRUNK ENV
SANCHORIS_DYNAMO_PORT=8473
AWS_ENDPOINT_URL_DYNAMODB=http://127.0.0.1:8473
SANCHORIS_TABLE_MAIN=sanchoris-fix_health_abc123-main
SANCHORIS_TABLE_AUDIT=sanchoris-fix_health_abc123-audit
# END WORKTRUNK ENV
```

Each worktree gets its own DynamoDB Local port and table names. portless detects linked git worktrees and prefixes the branch name into the hostname, so each worktree can run its own frontend and api without port collisions. Set `SANCHORIS_PUBLIC_URL` to the worktree's origin.

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
scripts/verify.sh full
```

runs `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo nextest run --workspace` (with the end-to-end test), and `pnpm check`. `scripts/verify.sh quick` runs them for the crates and apps changed since the merge base with the local `master`. `scripts/verify.sh test <arguments>` runs `cargo nextest run <arguments>` against the DynamoDB Local instances it starts.
