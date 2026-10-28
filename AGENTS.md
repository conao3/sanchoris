# AGENTS.md

Read this file instead of reading `README.md` and `docs/architecture.md` in full. It maps the code, names the checks for each area, and points to the one section of a design document that answers a question.

## Rules

- Write everything in the repository in English: documents, code comments, test names, commit messages.
- User-visible text in the web app comes from `apps/frontend/src/i18n/en.json` (the source) and `ja.json`; never hardcode it in components.
- Commit messages are one English sentence or paragraph without a subject prefix, like `git log --oneline` shows.
- Fail fast: required environment variables have no defaults, and missing ones stop the process with their names.
- Run commands in the Nix devShell (`nix develop -c <command>`, or direnv). It wraps `rustc` with `sccache` (shared cache `~/.cache/sccache`) and sets `CARGO_BUILD_JOBS=6`; keep both, and do not set `CARGO_TARGET_DIR` or run `cargo clean`.

## Verify before committing

| Command | Runs | When |
|---|---|---|
| `scripts/verify.sh quick` | Against `origin/master`: `cargo fmt --check` when Rust changed; clippy `-D warnings` on the changed crates and their dependents; `cargo test` of the changed crates; `pnpm e2e` when only the workspace agent or `scripts/e2e` changed; `pnpm --filter <package> check` for each changed TS app | While working |
| `scripts/verify.sh full` | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --no-fail-fast` (includes the e2e test that `pnpm e2e` runs), `pnpm check` | Before each commit and merge |

Both re-enter the devShell when started outside it, run `pnpm install` when `node_modules` is missing, and print each step's time.

## Map

| Path | What it holds |
|---|---|
| `crates/config` | `sanchoris.yaml` and `skill.yaml` parsing and validation (`config.rs`, `skill.rs`, `validate.rs`), built-in Skills (`builtin.rs`) |
| `crates/engine` | The pure skill state machine (`machine.rs`), states and paths (`model.rs`, `path.rs`, `intrinsic.rs`) |
| `crates/core` | DynamoDB store and `TenantTx` (`db.rs`, `store.rs`), item types (`items.rs`, `keys.rs`), tasks and skill runs (`tasks.rs`, `skillrun.rs`), EventPattern and InputTransformer (`pattern.rs`, `transform.rs`), table schema from `infra/tables.json` (`schema.rs`), `TestDynamo` (`testing.rs`), `sanchoris-dynamo-create` (`bin/dynamo_create.rs`) |
| `crates/worker` | Job handlers: config sync (`config_sync.rs`), trigger dispatch (`dispatcher.rs`), engine worker (`engine_worker.rs`), task supervisor (`supervisor.rs`), Resources (`resources.rs`), local runner (`runner.rs`); api contract in `README.md` |
| `crates/graphql` | Resolvers for `apps/frontend/src/graphql/schema.graphql` (`query.rs`, `mutation.rs`, `task.rs`, `config.rs`, `tenant.rs`, `skill.rs`) |
| `crates/api` | HTTP API and webhooks (`app.rs`, `webhooks.rs`, `stripe.rs`, `github_oauth.rs`), Run channel (`channel/`), Lambda handlers (`lambda.rs`, `bin/{api,channel,channel_authorizer,worker,relay}.rs`), ECS Workspaces and relay lease (`ecs.rs`), `sanchoris all` (`bin/sanchoris.rs`, `server.rs`), embedded frontend (`frontend.rs`) |
| `apps/frontend` | React + Apollo + TanStack Router; screens in `src/pages`, GraphQL documents and codegen output in `src/graphql`, strings in `src/i18n` |
| `apps/workspace-agent` | Agent inside a Workspace (`src/agent.ts`, `src/channel.ts`, workers in `src/workers`) and the `sanchoris` command (`src/cli.ts`); protocol in `PROTOCOL.md` |
| `apps/egress-relay` | Workspace egress `CONNECT` proxy (`src/relay.ts`) |
| `infra/` | `tables.json`, `plans.json`, the Workspace and relay Dockerfiles |
| `scripts/` | `dev.mjs` (`pnpm dev`), `build-lambda.mjs` (`pnpm build:lambda`), `verify.sh`, `deploy.mjs`, `e2e-aws.mjs`, `e2e/fake-agent.ts` |

## Checks per area

`scripts/verify.sh quick` picks these from the changed files; run one directly when iterating on a single test.

| Change | Check |
|---|---|
| A crate | `cargo test -p <crate>`; one file: `cargo test -p sanchoris-api --test http` (`http`, `lambda`, `graphql`, `e2e`) |
| `apps/frontend/src/graphql/*.graphql` | `cargo test -p sanchoris-api --test graphql` (runs every operation) and `cargo test -p sanchoris-graphql --test schema_contract` |
| `apps/frontend` | `pnpm --filter @sanchoris/frontend check`; UI changes also in the browser through `pnpm dev` (README "Development") |
| `apps/workspace-agent`, `scripts/e2e` | `pnpm --filter @sanchoris/workspace-agent check` and `pnpm e2e` |
| `apps/egress-relay` | `pnpm --filter @sanchoris/egress-relay check` |
| `infra/tables.json`, `infra/plans.json` | `cargo test -p sanchoris-core`, then the aws-infra-k8s template check (`docs/014-sanchoris.md` "Deploy") |
| Lambda functions, their variables or IAM | Keep `docs/architecture.md` "Lambda functions" and aws-infra-k8s `docs/014-sanchoris.md` "Lambda functions" in step |
| Behavior in AWS (Workspaces, relay, webhooks) | `scripts/deploy.mjs dev`, then `scripts/e2e-aws.mjs dev --arch arm64` and `--arch x86_64` |

## Deploy and e2e on AWS

Available after phase B of the agent tooling work; until then follow aws-infra-k8s `docs/014-sanchoris.md`. Deploy only with the operator's approval.

- `node scripts/deploy.mjs <dev|prd> [--plan]`: needs `AWS_INFRA_K8S_DIR` (the aws-infra-k8s checkout). In parallel it builds the Lambda zips (restored from `target/lambda-cache/<env>/<key>` when the sources and the Cognito values from the CloudFormation exports are unchanged), builds and pushes the Workspace and relay images unless ECR already has their `src-<hash>` tag, and renders the stack template with aws-infra-k8s (an uberjar cached in `~/.cache/sanchoris-deploy` by its sources). When the template hash equals the stack output `SanchorisTemplateHash`, dev updates the five functions with `aws lambda update-function-code`; otherwise, and always on prd, it runs `deploy sanchoris`, waits for the stack, and then updates any function whose code still differs from the local zip. It seeds `PLANS` and prints one JSON summary with the time of each step. `--plan` builds nothing and changes nothing in AWS. Logs go to `.sanchoris/deploy/<env>/`.
- `node scripts/e2e-aws.mjs <dev|prd> --arch arm64|x86_64 [--title <text>] [--timeout-minutes <n>]`: opens an issue in the environment's test repository (dev `sancode-org/sanchoris-e2e-dev`, prd `sancode-org/sanchoris-e2e`), adds the label of the architecture (`sanchoris` or `sanchoris-x86`), waits for the Task with backoff, and prints JSON: the Task, its runs with `LOG#` entries and relay connections, Workspaces, ECS stop reasons, CPU, architecture and capacity provider, and the pull request with its checks. The full report goes to `.sanchoris/e2e-aws/`. Exit 0 only when the Task succeeded on the requested architecture with green checks.

## Bugs that show up only in AWS

Turn each one into a test before fixing it.

1. Capture the event that the function received, and redact secrets, tokens, signatures, account ids and personal data:
   - SQS work jobs: the job body is the `MSG#<uuid>` item in `main` (TTL 24 h; `aws dynamodb get-item`), and failed jobs stay in `<prefix>-sanchoris-work-dlq.fifo` (`aws sqs receive-message --attribute-names All`, which hides the message again for the visibility timeout).
   - DynamoDB Streams: the `NewImage` is the item as stored; read it with `aws dynamodb get-item`.
   - Run channel (WebSocket): the agent's messages are the run's `EVT#` and `PEND#` items, and its log lines the `LOG#` items.
   - HTTP: GitHub webhooks from the App's recent deliveries (`gh api /app/hook/deliveries` with the App JWT), Stripe events with `GET /v1/events/<id>`.
   - The functions log JSON (`LogFormat: JSON`) to `/aws/lambda/<prefix>-sanchoris-<function>`; `aws logs filter-log-events --filter-pattern '{ $.level = "ERROR" }'` finds the failing invocation and its request id.
2. Save it under `crates/api/tests/fixtures/`, replacing ids that the test creates with `{{name}}` placeholders, and load it with `fixture(name, vars)` in `crates/api/tests/lambda.rs` (`sqs_event`, `stream_event`, and `websocket_event` wrap the recorded `sqs.json`, `stream.json`, and `websocket-*.json`; `http.json` and `authorizer.json` are the HTTP API and authorizer events).
3. Write the failing test in `lambda.rs` against `TestDynamo`, fix the bug, and run `cargo test -p sanchoris-api --test lambda`.

## Design documents

Read only the section you need; this prints one heading of any level with its subsections:

```sh
awk -v h='Partitions' '!p && $0 ~ "^#+ " h {p=1; l=index($0, " "); print; next} p && /^#+ / && index($0, " ") <= l {exit} p' docs/architecture.md
```

| Topic | Document and section |
|---|---|
| Components, jobs, schedules, local runtime | `docs/architecture.md` "Runtime components", "Rate schedules", "Jobs and messages", "Local runtime" |
| Ports between crates | `docs/architecture.md` "Ports" |
| Lambda functions, client IP, Workspaces, secrets, variables | `docs/architecture.md` "Lambda functions", "Client IP", "Workspaces", "Secrets", "Environment"; README "Environment" |
| DynamoDB keys, items, transactions, queries | `docs/architecture.md` "Tables", "Partitions", "Transactions", "Access patterns" |
| Run channel messages | `apps/workspace-agent/PROTOCOL.md` |
| Worker inbox and outbox | `crates/worker/README.md` |
| Local e2e test | README "End-to-end test" |

The product design lives in the conao3/idea repository under `projects/sanchoris/` (Japanese). `README.md` and `skill.md` there decide design questions.

| Topic | Document and section |
|---|---|
| Terms, data structures, how a Task flows | `README.md` "用語", "データ構造", "動き方" |
| Config, tenants and members, sign-in and billing | `README.md` "Config", "Tenant とメンバー", "認証と課金" |
| GitHub, Run channel, `sanchoris` command, relay, Stripe, sign-in | `implementation.md` "4.1 GitHub" to "4.6 サインインと GitHub の連携" |
| Config validation, tests, environment and secrets | `implementation.md` "5. 設定の検証", "7. テスト", "9. 環境と秘密" |
| `skill.yaml`, Resources, `open-pr-green-ci` | `skill.md` "skill.yaml", "Resource", "例: open-pr-green-ci" |
| Trigger, Session, `sanchoris.yaml`, Task and Run, engine, limits | `skill.md` "Trigger", "Session", "Config の YAML", "Task、Run、SkillRun、StateEntry", "実行エンジン", "利用枠の上限" |
| Threats, abuse, Workspace network, config trust, audit, App permissions | `security.md` "脅威と対策", "悪用の防止", "Workspace のネットワーク", "設定リポジトリの信頼境界", "監査", "GitHub App の権限" |
| Results of the AWS runs (dev, Spot and arm64, prd, renames) | `aws-verification.md` "T8", "T9", "T10", "T11", "未検証の点" |
| Fargate prices, limits, startup, Spot | `aws-compute.md` "2. 料金", "4. クォータ・リージョン", "10. Fargate の運用上の論点" |
