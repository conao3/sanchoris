# Architecture

Sanchoris runs as a serverless control plane on AWS: Lambda functions behind API Gateway and SQS, DynamoDB for state, S3 for large objects, and EventBridge Scheduler for timers. Only Workspaces run on Fargate. This document is the contract between `crates/core`, `crates/worker`, `crates/graphql`, `crates/api`, and the CloudFormation that deploys them. `infra/tables.json` holds the table definitions as CloudFormation properties and `infra/plans.json` the plan catalog; the code reads both, so the templates and the code cannot drift.

## Runtime components

```text
Browser ── HTTPS ──> API Gateway HTTP API ──> Lambda api ───────────────┐
                       /api/* (Cognito JWT); $default: /webhooks/*,      │
                       /auth/github*, /health, the frontend (SPA)        │
workspace-agent ── WSS ─> API Gateway WebSocket API ──> Lambda channel ──┤
                       $connect: Lambda channel-authorizer (Run token)   │
                                                                         ▼
                                       DynamoDB main table (+ stream) and audit table
                                                    │ stream: kind = msg
                                                    ▼
                                             Lambda relay ──> SQS FIFO work queue ──> Lambda worker
                                                    │                 ▲                    │
                                                    └─> EventBridge ──┘ (one-time and      ├─> GitHub API
                                                        Scheduler        rate schedules)   ├─> ECS RunTask (Fargate Workspaces)
                                                                                           └─> API Gateway Management API (outbox push)
```

| Component | Trigger | Code | Does |
|---|---|---|---|
| Lambda `api` | API Gateway HTTP API: `ANY /api/{proxy+}` (`POST /api/graphql`, `GET /api/me`, `POST /api/tasks/{id}/replies`, `POST /api/approvals`; JWT authorizer on the shared Cognito pool) and `$default` (`POST /webhooks/github`, `POST /webhooks/stripe`, `/auth/github*`, `GET /health`, every other `GET` for the frontend) | `crates/api`, `crates/graphql` | GraphQL for the screens, webhook ingest (signature checked in code), OAuth, audit exports, and the built frontend embedded in the binary (SPA fallback to `index.html`), all on one origin. Writes through `TenantTx`. Config sync (`addConfig`, `syncConfig`) and chat dispatch (`sendChatMessage`) run inline through `config_sync::sync_config` and `dispatcher::dispatch_inbound` (the message's own inbound event only) on a `Worker` whose runner and outbox pusher refuse to act: both only read GitHub and write DynamoDB, finish well inside the 29 s integration timeout, and let the mutation return the synced config or the started task. The jobs they emit (`Reconcile`, `DeliverOutbox`, ...) run in the worker function |
| Lambda `channel-authorizer` | REQUEST authorizer of the WebSocket API's `$connect`, identity source `route.request.header.Authorization`, result caching off | `crates/api` (`lambda::authorize`) | Verifies the `Authorization: Bearer <Run token>` header that `workspace-agent` sends (non-browser clients may set headers on the upgrade). An invalid or expired token fails with `Unauthorized` (HTTP 401); a run that is missing or finished gets a `Deny` policy (HTTP 403); otherwise `Allow` with the context `tenantId` and `runId` |
| Lambda `channel` | API Gateway WebSocket API: `$connect`, `$default`, `$disconnect` | `crates/api` (`lambda::websocket`, `channel::Channel`) | Run channel (`apps/workspace-agent/PROTOCOL.md`). `$connect` stores the connection id on the run's `#CHANNEL` item; `$disconnect` clears it when it is still that connection. `$default`: `hello` registers the agent instance (`INST#`) and emits `DeliverOutbox`; sequenced messages are deduplicated by `(instanceId, seq)`, recorded, and acknowledged; `skill_request`, `record_upload_request`, `git_token_request`, and `worker_credential_request` are answered in the same invocation (the credential replies are sent to the connection and never stored). Replies go through the API Gateway Management API. A protocol error sends `close` (code 4000) and deletes the connection |
| Lambda `relay` | DynamoDB Streams on the main table, filter `eventName = INSERT` and `kind = msg` (`infra/tables.json` `streamFilters`) | `sanchoris_core::store::{relayed_from_item, relay}` | Turns committed message items into SQS sends (`deliver_at` absent or past) or EventBridge one-time schedules (`deliver_at` in the future), and deletes cancelled schedules |
| Lambda `worker` | SQS FIFO `sanchoris-work.fifo`, batch size 10, `ReportBatchItemFailures`, `MaximumConcurrency` 5 (the dev account allows 10 concurrent executions in total) | `crates/worker` (`Worker::handle_body`), `crates/api` (`lambda::work`) | Runs one `Job` per message (`crates/worker/README.md`). A failed message is reported with every later message of the batch, so a group keeps its order. `DeliverOutbox` posts to the run's connection (`OutboxPusher`) |
| SQS `sanchoris-work.fifo` + `sanchoris-work-dlq.fifo` | — | — | FIFO with content-based deduplication. `MessageGroupId` is `Job::group()`: `tenant:<id>`, `task:<id>`, `run:<id>`, `config:<id>`, `sweep:<name>`, so the jobs of one task never run concurrently. Visibility timeout 6 × the worker timeout; `maxReceiveCount` 5 to the DLQ |
| EventBridge Scheduler group `sanchoris` | — | `EventBridgeScheduler` | One-time `at()` schedules created by the relay (engine timers, `ResumeAt`, idle deadlines, trigger schedules), `ActionAfterCompletion: DELETE`, target the work queue with the job's group. Four rate schedules created by CloudFormation (below) |
| ECS Fargate | `RunTask` from the worker's `RunnerPort` | — | Workspaces only |
| Egress relay | ECS service in the public subnet, desired count 0 until a Workspace needs it (`ecs::RelayLease`) | `apps/egress-relay` | The Workspaces' `HTTPS_PROXY`: an HTTP `CONNECT` proxy on port 3128 that tunnels to port 443 only, after the operator deny list, then the platform hosts (Anthropic, GitHub, the Run channel host, the records bucket), then the Run's `Network.Allow`; it refuses private, link-local, multicast, reserved, benchmarking, and NAT64 addresses. It keeps at most 256 open tunnels per run (the `run` of the relay token; more get `429`) and 8192 connections in all, closes a client or upstream socket after 10 minutes without traffic, and closes every tunnel on `SIGTERM`. It is the only way out when the Workspaces run in a subnet without an internet route (the strict network); in the public network it is advisory ("Workspaces") |
| DynamoDB | — | `sanchoris_core::store` | `main` (single table) and `audit` (append-only) |
| S3 | — | `BlobStore`, `records::S3RecordStore` | `docs` (attributes over 32 KiB: config revisions, skill bodies, task definitions, large events; content-addressed `docs/<tenant>/<sha256>`) and `records` (session records `t/<tenant>/runs/...` presigned by the channel and the worker, audit exports `t/<tenant>/audit/...` written by the api). Both buckets encrypt objects with SSE-S3 by default; the app sends no encryption headers, and tenant separation is the key prefix that `RecordStore` checks. Run log lines stay in DynamoDB (`LOG#` with TTL) |

### Rate schedules

Each rate schedule sends the same body every time except for the scheduled time, which keeps content-based deduplication from dropping consecutive firings. The worker ignores the extra field.

```json
{"id":"00000000-0000-0000-0000-000000000000","job":{"type":"sweep_checks"},"scheduledTime":"<aws.scheduler.scheduled-time>"}
```

| Job | Rate | Does |
|---|---|---|
| `sweep_configs` | 5 minutes | `ConfigSync` for configs not synced in 5 minutes (config polling) |
| `sweep_checks` | 1 minute | `CheckPoll` for check waits whose `next_poll_at` passed (check polling) |
| `sweep_task_limits` | 1 minute | `Start.Limits` (turns, cost, timeout, heartbeat) on `RUNNING`/`WAITING` tasks |
| `usage_alerts` | 15 minutes | 80% plan usage alerts (usage refresh) |
| `sweep_workspaces` | 1 minute | Records Workspaces that stopped on their own (the ECS task's `stoppedReason`, stop code, and exit code on the run and the task) and scales the egress relay to 0 when no Workspace runs |

### Jobs and messages

A job is emitted inside the `TenantTx` that causes it (`tx.emit(job)`, `tx.emit_at(job, at)`, `tx.cancel_schedule(message_id)`). Commit writes one message item per job (`pk = MSG#<uuid>`, `kind = msg`, `body` = the `Relayed` JSON, TTL 24 h) in the same `TransactWriteItems`, so state and follow-up work commit together. In AWS the stream delivers the message items to the relay (`Relay::Stream`); locally the transaction hands them to the in-memory queue and scheduler right after commit (`Relay::Direct`). Delivery is at least once; the FIFO deduplication id is the envelope id, and every handler re-reads state and ignores messages that a newer one superseded (timers compare `message_id` and are scheduled again at their `due_at` when they arrive before it, trigger schedules compare `schedule_message_id`, resumes compare `resume_at`, check polls compare `next_poll_at`).

The SQS body is the `Envelope` JSON: `{"id": "<uuid>", "job": {"type": "<job>", ...}}`. Job types and their fields are `sanchoris_core::jobs::Job`.

### Local runtime

`sanchoris all` and the tests run every component in one process: the api serves HTTP and the Run channel (`/runs/channel`, the same `channel::Channel` logic as the Lambda routes over an axum WebSocket), `Store` uses `Relay::Direct` over `MemoryQueue` and `MemoryScheduler`, `LocalDriver` handles the queue and fires due schedules on the `Clock`, and DynamoDB Local (`pnpm dynamo:local`) holds the tables. `DirBlobStore` replaces S3, the local records store replaces the records bucket, and the frontend comes from `SANCHORIS_FRONTEND_DIR`. GraphQL's inline config sync and chat dispatch use the same `Worker` as `LocalDriver`.

## Ports

| Port | Production | Local and tests |
|---|---|---|
| `sanchoris_core::ports::Queue` (`send` with group, deduplication id and optional delay; `receive`, `delete`, `release`) | `aws::SqsQueue` | `local::MemoryQueue` (FIFO per group, one message in flight per group, deduplication by id, delays on the `Clock`) |
| `sanchoris_core::ports::Scheduler` (`schedule(name, at, message)`, `cancel(name)`) | `aws::EventBridgeScheduler` (`at()` expression in UTC, rounded up to the next whole second, SQS FIFO target, delete after completion; an existing name counts as scheduled) | `local::MemoryScheduler` (`take_due(now)`) |
| `sanchoris_core::ports::BlobStore` (`put`, `get`, `delete`) | `aws::S3BlobStore` | `local::MemoryBlobStore`, `local::DirBlobStore` |
| `sanchoris_core::clock::Clock` | `SystemClock` | `FakeClock` |
| `sanchoris_worker::ports::OutboxPusher` (`push(tenant, run)`) | `channel::Channel` over `channel::gateway::GatewaySender` (`PostToConnection`, `DeleteConnection`; `GoneException` clears the run's connection) | `Channel` over `channel::local::LocalConnections`; recording fake in the worker tests |
| `RunnerPort` (Workspaces: `start_workspace`, `stop_workspace`, `workspace_exit`, `sweep`) | `ecs::EcsRunner` (`RunTask` on Fargate Spot, then on Fargate when Spot has no capacity, with the Run's environment as container overrides, `StopTask`, `DescribeTasks`, and the egress relay lease) | `LocalProcessRunner` (`try_wait` on the process) |
| `GitHubClient`, `GitHubReader`, `ConfigSource`, `ResourceInvoker` | `GitHubApi`, `GitHubResources` (`crates/worker/src/ports.rs`) | fakes |

## Lambda functions

One crate (`crates/api`) builds every function as its own binary. `pnpm build:lambda` builds the frontend into `apps/frontend/dist` (embedded by the `api` binary), cross-compiles the binaries with `cargo lambda` for `aarch64-unknown-linux-musl` (static, so the glibc of the runtime does not matter), and writes `target/lambda/<function>/bootstrap.zip`. Deploy each zip on the `provided.al2023` runtime with the `arm64` architecture and the handler `bootstrap` (custom runtimes ignore the name; the binary is the handler).

| Function | Handler | Trigger | Timeout, memory | Environment | Secrets (SSM) | IAM actions |
|---|---|---|---|---|---|---|
| `api` | `api`: `lambda_http::run` over `app::router` with the `graphql::gateway_ip` and `graphql::admit_sources` layers | API Gateway HTTP API (payload 2.0, integration timeout 29 s): `ANY /api/{proxy+}` with the Cognito JWT authorizer (issuer = the user pool, audience = the sanchoris app client) and `$default` without an authorizer | 29 s, 512 MB | Store variables; `SANCHORIS_SECRETS_PATH`, `SANCHORIS_ALLOWED_SOURCES`, `SANCHORIS_PUBLIC_URL`, `COGNITO_USER_POOL_ID`, `COGNITO_CLIENT_ID`, `GITHUB_URL`, `GITHUB_API_URL`, `STRIPE_API_URL`, `STRIPE_PORTAL_CONFIGURATION`, `SANCHORIS_RECORDS_STORE=s3`, `SANCHORIS_RECORDS_BUCKET` | `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET`, `GITHUB_APP_SLUG`, `GITHUB_WEBHOOK_SECRET`, `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, `TASK_TOKEN_SIGNING_KEY`, `OAUTH_STATE_SIGNING_KEY`, `SANCHORIS_CREDENTIAL_KEY` | Store actions; records actions; `ssm:GetParameters` on its parameters |
| `channel-authorizer` | `channel-authorizer`: `lambda::authorize` | REQUEST authorizer of the WebSocket API's `$connect` (identity source `route.request.header.Authorization`, no caching) | 10 s, 256 MB | Store variables; `SANCHORIS_SECRETS_PATH`, `SANCHORIS_ALLOWED_SOURCES` | `RUN_TOKEN_SIGNING_KEY` | `dynamodb:GetItem` on `main` and its indexes; `s3:GetObject`, `s3:ListBucket` on `docs`; `ssm:GetParameters` on its parameter |
| `channel` | `channel`: `lambda::websocket` over `channel::Channel` with `GatewaySender` | WebSocket API routes `$connect` (behind `channel-authorizer`), `$default`, `$disconnect` (Lambda proxy integration) | 29 s, 256 MB | Store variables; `SANCHORIS_SECRETS_PATH`, `SANCHORIS_RECORDS_STORE=s3`, `SANCHORIS_RECORDS_BUCKET`, `SANCHORIS_CHANNEL_ENDPOINT` (`https://<api id>.execute-api.<region>.amazonaws.com/<stage>`), `GITHUB_API_URL` | `RUN_TOKEN_SIGNING_KEY`, `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, `SANCHORIS_CREDENTIAL_KEY` | Store actions; records actions; connection actions; `ssm:GetParameters` on its parameters |
| `worker` | `worker`: `lambda::work` over `Worker` with `EcsRunner` and the channel as `OutboxPusher` | SQS FIFO `sanchoris-work.fifo` (batch 10, `ReportBatchItemFailures`, `MaximumConcurrency` from the stack) | 300 s, 512 MB | Store variables; `SANCHORIS_SECRETS_PATH`, `SANCHORIS_RECORDS_STORE=s3`, `SANCHORIS_RECORDS_BUCKET`, `SANCHORIS_CHANNEL_ENDPOINT`, `GITHUB_API_URL`, `SANCHORIS_ECS_CLUSTER`, `SANCHORIS_ECS_TASK_DEFINITION_ARM64` and `SANCHORIS_ECS_TASK_DEFINITION_X86_64` (families), `SANCHORIS_ECS_CONTAINER`, `SANCHORIS_ECS_SUBNETS`, `SANCHORIS_ECS_SECURITY_GROUPS` (comma-separated), `SANCHORIS_ECS_ASSIGN_PUBLIC_IP` (`ENABLED` or `DISABLED`), `SANCHORIS_CONTROL_URL` (the `wss://` URL of the WebSocket API), `SANCHORIS_GIT_URL`, `SANCHORIS_RELAY_SERVICE`, `SANCHORIS_RELAY_PORT` (3128) | `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, `RUN_TOKEN_SIGNING_KEY`, `RELAY_TOKEN_SIGNING_KEY` | Store actions; records actions; connection actions; `sqs:ReceiveMessage`, `sqs:DeleteMessage`, `sqs:GetQueueAttributes`, `sqs:ChangeMessageVisibility` on the work queue; `ecs:RunTask` on the two Workspace task definitions in the cluster, `ecs:StopTask`, `ecs:DescribeTasks`, `ecs:TagResource` on the cluster's tasks, `ecs:ListTasks` in the cluster, `ecs:UpdateService`, `ecs:DescribeServices` on the relay service, `iam:PassRole` on the Workspace execution role; `ssm:GetParameters` on its parameters |
| `relay` | `relay`: `lambda::relay_stream` over `SqsQueue` and `EventBridgeScheduler` | DynamoDB Streams of `main` (`NEW_IMAGE`, filter `eventName = INSERT` and `kind = msg` from `infra/tables.json` `streamFilters`, batch 100, bisect on error, `ReportBatchItemFailures`, failures to a DLQ) | 60 s, 256 MB | `SANCHORIS_WORK_QUEUE_URL`, `SANCHORIS_WORK_QUEUE_ARN`, `SANCHORIS_SCHEDULE_GROUP`, `SANCHORIS_SCHEDULER_ROLE_ARN` | — | `dynamodb:GetRecords`, `dynamodb:GetShardIterator`, `dynamodb:DescribeStream`, `dynamodb:ListStreams` on the stream; `sqs:SendMessage` on the work queue and the stream DLQ; `scheduler:CreateSchedule`, `scheduler:DeleteSchedule`, `scheduler:GetSchedule` in the group; `iam:PassRole` on the scheduler role |

The store variables are `SANCHORIS_TABLE_MAIN`, `SANCHORIS_TABLE_AUDIT`, and `SANCHORIS_DOCS_BUCKET`. The store actions are `dynamodb:GetItem`, `dynamodb:BatchGetItem`, `dynamodb:Query`, `dynamodb:PutItem`, `dynamodb:UpdateItem`, `dynamodb:DeleteItem`, `dynamodb:ConditionCheckItem` (through `TransactWriteItems`) on `main` and its indexes (with a deny of writes to the `PLANS` partition), `dynamodb:PutItem`, `dynamodb:GetItem`, `dynamodb:Query`, `dynamodb:ConditionCheckItem` on `audit` (with the `appendOnlyDeniedActions` deny), and `s3:GetObject`, `s3:PutObject`, `s3:ListBucket` on `docs`. The records actions are `s3:GetObject`, `s3:PutObject`, `s3:ListBucket` on `records` (presigned uploads and downloads are signed with the function's role). The connection actions are `execute-api:ManageConnections` on `arn:aws:execute-api:<region>:<account>:<api id>/<stage>/POST/@connections/*` and `.../DELETE/@connections/*`. Every function also gets `logs:CreateLogStream` and `logs:PutLogEvents` on its log group.

### Client IP

The api function records the client IP of GraphQL requests (the audit events' `source_ip`) from the HTTP API's `requestContext.http.sourceIp`. When `sourceIp` is in Cloudflare's published ranges (`CLOUDFLARE_RANGES` in `crates/api/src/graphql.rs`), it takes `CF-Connecting-IP` instead; from any other address the header is ignored. The list is a copy of https://www.cloudflare.com/ips-v4 and https://www.cloudflare.com/ips-v6 (aws-infra-k8s `resources/cloudflare-ips.edn` holds the same list); when those files change, update both and run `cargo test -p sanchoris-api --lib graphql`. With `SANCHORIS_ALLOWED_SOURCES=cloudflare`, the api function answers 403 to a request whose `sourceIp` is outside these ranges, and the channel authorizer denies a `$connect` whose `requestContext.identity.sourceIp` is outside them, so a request that reaches the regional endpoint of a custom domain without Cloudflare is refused.

### Workspaces

The Fargate task gets `SANCHORIS_RUN_ID`, `SANCHORIS_RUN_TOKEN`, `SANCHORIS_CONTROL_URL`, `SANCHORIS_GIT_URL`, `SANCHORIS_BRANCH`, `SANCHORIS_REPOSITORIES` (comma-separated `owner/name`), `SANCHORIS_CREDENTIAL_SOURCE=channel`, and `SANCHORIS_RELAY_URL` (`http://run:<relay token>@<relay private IP>:<SANCHORIS_RELAY_PORT>`) as container overrides, and `startedBy` = the run id without dashes. The image's entrypoint (`infra/workspace/entrypoint.sh`) removes `SANCHORIS_RUN_TOKEN` and `SANCHORIS_RELAY_URL` from the environment and `exec`s `tini` (which reaps orphaned processes as PID 1) with `workspace-agent` as its child and both secrets on stdin, so no process in the Workspace inherits them and no `/proc/<pid>/environ` holds them. The task definition therefore leaves `initProcessEnabled` off: ECS's init process would keep the original environment as PID 1. The relay URL carries a relay token as its password: hex-encoded JSON `{"run", "exp", "allow"}` (the Session's `Network.Allow`) and its HMAC-SHA256 under `RELAY_TOKEN_SIGNING_KEY`, valid as long as the Run token (12 hours). The relay verifies the token on every `CONNECT`, so the allow list travels with the Run and the relay needs no database. The image (`infra/workspace/Dockerfile`, built for `linux/arm64` and `linux/amd64`) sets `SANCHORIS_SOCKET_PATH`, `SANCHORIS_WORKSPACE_DIR`, `CLAUDE_CONFIG_DIR`, `SANCHORIS_CLAUDE_PATH` (the Claude Code binary bundled with the Agent SDK), `NODE_USE_ENV_PROXY=1`, `HTTPS_PROXY` and `HTTP_PROXY` = `http://127.0.0.1:3129` (the agent's forwarder, which adds the relay token to each `CONNECT` it passes to the relay), `NO_PROXY=127.0.0.1,localhost`, and the git credential helper `sanchoris git-credential`, and runs `workspace-agent` as `node` (uid 1000).

With `SANCHORIS_CREDENTIAL_SOURCE=channel` the agent asks the channel for the worker credential before it starts the session (`worker_credential_request`: the channel decrypts the Session's `Worker.Credential` and returns `ANTHROPIC_API_KEY`, or `CLAUDE_CODE_OAUTH_TOKEN` for a key that starts with `sk-ant-oat`). The agent keeps the credential in memory and runs a credential proxy on `127.0.0.1`: Claude Code gets `ANTHROPIC_BASE_URL` pointing at it and a placeholder in the same variable, and the proxy replaces the placeholder with the credential on each request to `https://api.anthropic.com` (`Authorization: Bearer` with the `oauth-2025-04-20` beta for an OAuth token, `x-api-key` for an API key). Claude Code (2.1.311, bundled with the Agent SDK) sends every model request to `ANTHROPIC_BASE_URL` with either kind of credential and, with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`, contacts no other host. So neither the provider credential, the Run token, nor the relay token is in the environment of Claude Code, its tools, or `exec` commands, and a session that copies its environment or files somewhere it can write (a commit, an allowed host) does not leak them. What stays: any process in the Workspace can send provider requests through the credential proxy for the life of the Workspace (it cannot read the credential), and the agent runs as the same user (`node`) as the session, so a process that reads another process's memory (`ptrace`, `/proc/<pid>/mem`, where the kernel allows it for the same user) can read the credentials from the agent. The agent clones the repositories itself and gets GitHub tokens for `git` through `git_token_request`: a GitHub App installation token narrowed with `repositories` (the names in the Session's `GitHub.Repos`; the whole installation when a pattern is `owner/*`) and `permissions` (`GitHub.Permissions`), valid for one hour. The token therefore sits in the Workspace for the life of a `git` command and in the agent's memory until it expires; the relay does not intercept TLS. Keeping the token outside the Workspace (the relay adding it to GitHub requests) needs TLS interception with a CA that the image trusts and is not built. The relay image (`infra/relay/Dockerfile`) runs `apps/egress-relay` (`SANCHORIS_RELAY_PORT`, `RELAY_TOKEN_SIGNING_KEY`, `SANCHORIS_RELAY_PLATFORM_HOSTS`, `SANCHORIS_RELAY_DENY_HOSTS`) and logs one JSON line per `CONNECT` with the run, the host, and the decision. Both Dockerfiles pin their `node:22` base image by digest; to move to a newer one, read the index digest with `docker buildx imagetools inspect node:22-bookworm` (`node:22-alpine` for the relay) and replace it.

The egress relay runs only while Workspaces need it. Before `RunTask`, `EcsRunner` takes the relay lease (the `RELAY` item: a conditional update sets `lock_owner` and `lock_until` = now + 60 s when no live lock exists), sets the relay service's desired count to 1, adds the run to `holders`, and releases the lock; then it waits up to 240 s for a relay task whose desired and last status are `RUNNING` and reads its private IP from the task's ENI. A failed `RunTask` gives the run back. `stop_workspace` (finished or idle runs) and the `sweep_workspaces` job (while the desired count is above 0) take the lock, drop the stopping run, keep the holders whose Workspace task is still listed (`ListTasks` of the Workspace family, desired status `RUNNING`, matched by `startedBy`) or that asked within the last 10 minutes, and set the desired count to 0 when no holder and no Workspace task remains. Concurrent workers serialize on the lock, so one never scales the relay down while another is starting a Workspace on it. A relay task that ECS replaces gets a new IP; Workspaces that already run keep the old address (their `relayUrl` is fixed at `RunTask`) and lose their way out when the old task stops. The relay service deploys with `MinimumHealthyPercent` 100 and `MaximumPercent` 200, so a deployment starts the new relay task before it stops the old one, which keeps the old address working while the new task starts but not after; deploy a new relay image or task definition when no Workspace runs (`ListTasks` of the Workspace family is empty).

The Session's `Workspace.Architecture` picks the task definition: `arm64` the ARM64 family, `x86_64` the X86_64 family. The tenant's Plan picks the size: `start_workspace` copies the Plan's `workspace_cpu` and `workspace_memory_mib` (`infra/plans.json`: Free and Starter 1024 and 4096, Pro and Max 2048 and 4096) to the Workspace item, and `RunTask` passes them as the task-level `cpu` and `memory` overrides, so one task definition per architecture serves every Plan and a new size is a change to the plan catalog only. Both run the same image, pushed per architecture as `<tag>-arm64` and `<tag>-amd64`: the Dockerfile runs every `RUN` step on the build platform and installs the target's npm packages (pnpm `supportedArchitectures`, so the Agent SDK brings the `linux-arm64` or `linux-x64` Claude Code binary) and `tini` (`apt-get download tini:<arch>`), so a build for the other architecture needs no emulation.

`RunTask` asks for the capacity provider `FARGATE_SPOT` (weight 1, no `launchType`). Fargate does not move a Spot request to On-Demand by itself, so when the response's `failures[].reason` says the capacity is missing (`Capacity is unavailable at this time...` or a `RESOURCE:` reason), the worker logs the reason and calls `RunTask` again with `FARGATE`; other failures and exceptions fail the start. The Workspace's `capacity_provider` records the provider that started it, and the run screen shows it (`Workspace.capacityProvider`).

A Workspace is recorded before it is launched. `start_workspace` commits the `WS#` item without `platform_ref`, the `start` outbox item, and the run in `creating`, then calls `RunTask` and writes the task ARN as `platform_ref` and the capacity provider in a second transaction. Each `RunTask` gets `clientToken` = `spot-` or `ondemand-` and the Workspace id without dashes, so the Spot attempt and the On-Demand attempt are two requests (ECS answers a repeated token with the first response, which for a Spot failure is that failure), the tags `sanchoris:tenant`, `sanchoris:task`, `sanchoris:run`, and `sanchoris:workspace`, and tokens that expire 12 hours after the Workspace's `started_at`, so a retry for the same Workspace (a worker that crashed or timed out after `RunTask`, an optimistic-conflict retry) gets the task the first call started; a retry whose parameters differ (a new relay address) gets `ConflictException`, whose `resourceIds` name that task. `reconcile` launches an open Workspace without `platform_ref` again. `sweep_workspaces` closes a Workspace that still has no `platform_ref` 15 minutes after `started_at` (the run's `error` and the task's `cause` say it did not launch) and stops every running task of the Workspace families whose tags name no open `WS#` item with that ARN, which covers a launch the client token no longer deduplicates (24 hours). The worker Lambda gives each invocation a deadline 30 s before Lambda's, and the relay lock wait (90 s) and the relay start wait (240 s) end at it. A run whose Workspace stopped before its first turn (no `record_s3_key`, `turns` = 0) gets the Task's instruction again in its next `start`.

A Workspace interrupted by the platform is restored, not stopped. ECS sends `SIGTERM` two minutes before a Spot interruption; the agent sends `stopping`, interrupts the running turn, uploads its record, and exits 0 (`apps/workspace-agent/PROTOCOL.md`, "Stopping on SIGTERM"), and the worker writes `signaled_at` on the open Workspace of the run. When `sweep_workspaces` finds the task stopped, it counts the Workspace as interrupted if `signaled_at` is set or the stop code is `SpotInterruption` or `TerminationNotice`: it closes the Workspace at the interruption (the earlier of `signaled_at` and the task's `stoppingAt`), so the Workspace seconds of the Tenant stop there, sets the run to `idle` without an `error`, marks the `exec` items whose invocation still waits and the `user_message` items delivered after the interruption as undelivered, and, for a `RUNNING` task, sends the restore notice as a `user_message`. That message wakes `reconcile`, which starts a new Workspace with `resume` from the run's latest record on the Task branch, like the restore after an idle stop; for a waiting task the next message or a redelivered `exec` starts it. An interrupted turn's `turn_end` (`result: "interrupted"`) records usage and does not decide the task.

Workspace containers have no log driver. `workspace-agent` sends its log lines over the Run channel (`log` events become `LOG#` items). A Workspace that stops before the channel connects (image pull failure, a crash at start) is found by `sweep_workspaces`: `DescribeTasks` shows the task `STOPPED`, and the worker closes the Workspace, sets the run to `idle` with `error` = the `stoppedReason`, stop code, and the container's exit code or reason, and moves a `RUNNING` task to `NEEDS_YOU` with `Workspace.Stopped` and the same text as `cause`.

The network of Workspaces is the stack's choice (aws-infra-k8s `WorkspaceNetwork`). In the strict network they run in a subnet without an internet route, pull images through ECR interface endpoints and the S3 gateway endpoint, and the relay is their only way out. In the public network they run in the public subnet with a public IP, pull from ECR over the internet, and their security group allows port 443 to the internet besides port 3128 to the relay: `HTTPS_PROXY` makes Claude Code, git, and `NODE_USE_ENV_PROXY` clients use the relay, but a process that ignores the proxy reaches any host on port 443, so the relay's lists are advisory for a misbehaving agent.

Tenant isolation is enforced in the application: every function uses one role, and `TenantTx` rejects keys outside its tenant. No request assumes a tenant-scoped role.

### Secrets

Secrets never sit in a function's environment. Each function reads the parameters of its "Secrets" column once at cold start with `ssm:GetParameters` (with decryption, so String and SecureString both work) from `<SANCHORIS_SECRETS_PATH><parameter>` and stops with the names of missing parameters. This keeps the `api` function (which needs the GitHub App private key and ten more secrets) well under Lambda's 4 KB limit on environment variables. `sanchoris all` reads the same variables from its environment. The same path holds four non-secret String parameters: `stripe-portal-config`, which the stack resolves into the `api` function's `STRIPE_PORTAL_CONFIGURATION` (the Stripe account is shared with other products, so every customer portal session names this configuration), and `stripe-price-starter`, `stripe-price-pro`, `stripe-price-max`, which seed `PLANS` (see the partition table).

| Parameter | Variable |
|---|---|
| `github-app-id` | `GITHUB_APP_ID` |
| `github-app-private-key` | `GITHUB_APP_PRIVATE_KEY` (PEM) |
| `github-app-client-id` | `GITHUB_APP_CLIENT_ID` |
| `github-app-client-secret` | `GITHUB_APP_CLIENT_SECRET` |
| `github-app-slug` | `GITHUB_APP_SLUG` |
| `github-webhook-secret` | `GITHUB_WEBHOOK_SECRET` |
| `stripe-secret-key` | `STRIPE_SECRET_KEY` |
| `stripe-webhook-secret` | `STRIPE_WEBHOOK_SECRET` |
| `run-token-signing-key` | `RUN_TOKEN_SIGNING_KEY` (32+ bytes) |
| `task-token-signing-key` | `TASK_TOKEN_SIGNING_KEY` (32+ bytes) |
| `oauth-state-signing-key` | `OAUTH_STATE_SIGNING_KEY` (32+ bytes) |
| `credential-key` | `SANCHORIS_CREDENTIAL_KEY` (32 bytes in base64) |
| `relay-token-signing-key` | `RELAY_TOKEN_SIGNING_KEY` (32+ bytes; the worker signs relay tokens, the relay task reads it through its task definition) |

## Environment

All variables are required where they are used; nothing has a default.

| Variable | Used by | Value |
|---|---|---|
| `SANCHORIS_TABLE_MAIN`, `SANCHORIS_TABLE_AUDIT` | api, channel-authorizer, channel, worker, `sanchoris all` | Table names |
| `SANCHORIS_DOCS_BUCKET` | api, channel-authorizer, channel, worker | Bucket of offloaded attributes |
| `SANCHORIS_DOCS_DIR` | `sanchoris all` | Directory of offloaded attributes (`DirBlobStore`) |
| `SANCHORIS_SECRETS_PATH` | api, channel-authorizer, channel, worker | SSM path of the secrets, `/<prefix>/sanchoris/` |
| `SANCHORIS_ALLOWED_SOURCES` | api, channel-authorizer | `cloudflare` when the APIs sit behind Cloudflare (custom domains): requests from other source addresses get 403 from the api function and a deny from the channel authorizer; `any` otherwise |
| `SANCHORIS_RECORDS_STORE`, `SANCHORIS_RECORDS_BUCKET` | api, channel, worker | `s3` and the records bucket |
| `SANCHORIS_FRONTEND_DIR` | `sanchoris all` | Built frontend; the `api` function embeds it |
| `SANCHORIS_WORK_QUEUE_URL` | relay | Work queue URL |
| `SANCHORIS_WORK_QUEUE_ARN`, `SANCHORIS_SCHEDULE_GROUP`, `SANCHORIS_SCHEDULER_ROLE_ARN` | relay | Target, group, and execution role of one-time schedules |
| `SANCHORIS_CHANNEL_ENDPOINT` | channel, worker | API Gateway Management API endpoint of the WebSocket stage: `https://<channel custom domain>` when the stack has custom domains (the `execute-api` endpoint is disabled then), otherwise `https://<api id>.execute-api.<region>.amazonaws.com/prod` |
| `SANCHORIS_RELAY_SERVICE`, `SANCHORIS_RELAY_PORT` | worker | ECS service and port of the egress relay that Workspaces use as `HTTPS_PROXY` |
| `SANCHORIS_ECS_ASSIGN_PUBLIC_IP` | worker | `ENABLED` for Workspaces in the public subnet, `DISABLED` in the strict subnet |
| `AWS_REGION` | all | Region |
| `AWS_ENDPOINT_URL_DYNAMODB` | local only | DynamoDB Local |

The GitHub, Stripe, Cognito, signing-key, and local records variables are listed in `README.md`.

## Data model

### Tables

| Table | Keys | Indexes | Other settings |
|---|---|---|---|
| `main` | `pk` (S), `sk` (S) | `gsi1` (`gsi1pk`, `gsi1sk`, ALL), `gsi2` (`gsi2pk`, `gsi2sk`, ALL) | TTL `expires_at`, stream `NEW_IMAGE`, PITR, deletion protection, on-demand |
| `audit` | `pk` = `T#<tenant>`, `sk` = `<at>#<id>` | — | TTL `expires_at` (the plan's `audit_retention_days`), PITR, deletion protection; IAM denies `UpdateItem`, `DeleteItem`, `BatchWriteItem`, PartiQL updates and deletes, `DeleteTable`, and `UpdateTimeToLive` (`appendOnlyDeniedActions`) |

Every item in `main` has `kind` (the entity), `v` (a version incremented by every write), and its own attributes. Times are strings `YYYY-MM-DDTHH:MM:SS.ffffffZ` (UTC, microseconds), which sort by time; numbers in keys are zero-padded to 12 digits. JSON values are stored as JSON strings. A string attribute longer than 32 KiB is stored in the `docs` bucket at `docs/<tenant>/<sha256>` and replaced by `{"$blob": "<key>"}`; reads restore it. Message items (`MSG#`) are written inline: a `Relayed` body carries ids, times, and schedule names, well under 1 KiB, so the relay reads it from the stream image and needs no access to the `docs` bucket. No other attribute may make an item exceed 400 KB.

### Partitions

`<t>` is the tenant id. Every tenant-owned partition key starts with `T#<t>`.

| Partition (`pk`) | `sk` | Entity (`kind`) | Notes |
|---|---|---|---|
| `PLANS` | `<plan id>` | `plan` | Seeded from `infra/plans.json` by `sanchoris-dynamo-create`, which sets each paid plan's `stripe_price_id` from `SANCHORIS_STRIPE_PRICE_<PLAN ID>` (for example `SANCHORIS_STRIPE_PRICE_STARTER`) and stops when one is missing; in AWS an administrator runs it after each deploy with the prices from the SSM String parameters `<SANCHORIS_SECRETS_PATH>stripe-price-<plan id>`. Read-only for the app (`readOnlyLeadingKeys`) |
| `USER#<user>` | `#` | `user` | `cognito_subject`, `email`, `display_name`, `github_user_id`, `github_login`, `is_operator` |
| `USER#<user>` | `MEMBER#<t>` | `membership` | `member_id`; the user's tenants. Unique per (user, tenant) |
| `UNIQ#<kind>#<value>` | `#` | `guard` | Global uniqueness and lookup: `cognito`, `email` (lowercase), `ghuser`, `ghlogin` (lowercase) → user; `slug`, `stripe` → tenant; `free_tenant#<user>` → the one tenant on the Free plan that the user created (taken by `createTenant` and `changePlan` to Free, released when the tenant leaves Free); `installation` → integration (+ `tenant_id`); `slack` → integration. Slugs skip the reserved route names in `tenants::RESERVED_SLUGS` |
| `UNIQ#pending_checkout#<user>` | `#` | `pending_checkout` | `tenant_id`, `session_id`, `expires_at`: the one paid tenant of the user that waits for its Checkout Session (`PENDING_CHECKOUT_MINUTES`, 60). A new paid `createTenant` expires the open session at Stripe and discards that tenant; activation and a change to Free release the item |
| `UNIQ#stripe_event#<event id>` | `#` | `stripe_event` | `event_type`, `tenant_id`, `outcome` (`processed` or `ignored`), `reason`, `received_at`; TTL 30 days. Written in the transaction that applies the event, so a redelivered event changes nothing |
| `INVITEMAIL#<lowercase email>` | `INVITE#<t>#<invitation>` | `invitation_email` | `tenant_id`, `invitation_id`; invitations addressed to an email, for the invitee before they are a member |
| `MSG#<uuid>` | `#` | `msg` | Relayed jobs (TTL 24 h) |
| `RELAY` | `#` | `relay_lease` | The egress relay lease of the ECS runner: `holders` (map of run id to the time it asked for the relay), `lock_owner`, `lock_until`. Written with the DynamoDB client directly, outside `TenantTx` |
| `T#<t>` | `#TENANT` | `tenant` | `slug`, `name`, `plan_id`, `period_start`, `status`, `suspended_reason`, `stripe_customer_id`, counters `tasks_this_period`, `workspace_seconds_this_period`, `running_tasks`; `gsi2` = (`TENANTS`, `<t>`). Changed only with update expressions |
| `T#<t>` | `MEMBER#<member>` | `member` | `user_id`, `role`, `joined_at`, `last_active_at` |
| `T#<t>` | `INVITE#<invitation>` / `INVITEPENDING#<email>` | `invitation` / `guard` | `email`, `role`, `status`, `invited_by_member_id`, `expires_at`, `accepted_at`; the guard points at the pending invitation of an email and keeps one per email |
| `T#<t>` | `CRED#<credential>` / `CREDNAME#<name>` | `credential` / `guard` | Encrypted value (base64), KMS key id, status; the guard keeps names unique and finds a credential by name |
| `T#<t>` | `USAGESNAP#<credential>#<captured_at>` | `usage_snapshot` | TTL 30 days |
| `T#<t>` | `INTEG#<integration>` | `integration` | GitHub installation or Slack team; `details` JSON |
| `T#<t>` | `CONFIG#<config>` / `CONFIGREPO#<repository>#<root>` | `config` / `guard` | `active_revision_id`, `revisions` (counter), `synced_at`; `gsi2` = (`SWEEP#CONFIG`, `<synced_at or 0>#<t>#<config>`) |
| `T#<t>` | `ALERT#<period_start>#<metric>#<threshold>` | `usage_alert` | Unique by key |
| `T#<t>` | `PENDINGTASK#<created_at>#<task>` | `pending_task` | Dispatch queue of `PENDING` tasks |
| `T#<t>` | `CHECKWAIT#<wait>` | `check_wait_marker` | Waiting check waits with their repos, for check-run webhooks |
| `T#<t>#REF#<id>` | `#` | `ref` | Immutable pointer from an id to its item (`ref_kind`, `target_pk`, `target_sk`, `parent_id`) for trigger events, triggers, config revisions, runs, skill runs, resource invocations, and check waits |
| `T#<t>#CONFIG#<config>` | `REV#<number>` / `REVSHA#<sha>` | `config_revision` / `guard` | `parsed` (offloaded when large), validation result. Unique per (config, sha) |
| `T#<t>#CONFIG#<config>` | `PROJECT#<name>`, `PROJECT#<name>#REPO#<repo>`, `SESSION#<name>`, `SKILL#<name>`, `TRIGGER#<name>` | `project`, `repository`, `session`, `skill`, `trigger` | The active revision's definitions. Ids are `uuid v5(config, "<kind>:<name>")`, so a trigger keeps its id across revisions. Triggers carry `removed_at`, `next_run_at`, `schedule_message_id` |
| `T#<t>#CONFIG#<config>` | `PROJECT#<name>#REPO#<repo>` | `repository` | `prebuild_status`, `prebuild_at` of the warm build |
| `T#<t>#CONFIG#<config>` | `CR#<pr number>` | `change_request` | `title`, `state`, `changed_files`, `changed_definitions`, validation, requesting member or task. Unique per (config, PR) |
| `T#<t>#TRIGGER#<trigger>` | `EVT#<key>` | `trigger_event` | Unique per (trigger, idempotency key); `result`, `reason`, `task_id`; `gsi1` = (`T#<t>#TRIGGEREVENTS`, `<received_at>#<id>`) |
| `T#<t>#TRIGGER#<trigger>` | `TASK#<name>` | `task_name` | Unique per (trigger, task name); `task_id`, `created_at`; `gsi1` = (`T#<t>#TASKNAME#<name>`, `<created_at>#<task>`) |
| `T#<t>#TRIGGER#<trigger>` | `RATE#<hour>` | `trigger_rate` | Receive limit of the trigger for one clock hour (UTC): `starts` (capped at `Settings::trigger_starts_per_hour`, 60) and `discarded`; TTL 1 day |
| `T#<t>#TASK#<task>` | `#TASK` | `task` | Every `tasks` column, `definition` (offloaded when large), counters `runs`, `skill_runs`; `gsi1` = (`T#<t>#TASKS`, `<created_at>#<task>`); `gsi2` = (`SWEEP#ACTIVETASK`, `<t>#<task>`) while `RUNNING` or `WAITING` |
| `T#<t>#TASK#<task>` | `DEP#<blocker>`, `CHILD#<created_at>#<child>` | `task_dependency`, `task_child` | `blocked_by` and children |
| `T#<t>#TASK#<task>` | `RUN#<run>` | `run` | `number`, status, turns, `cost_usd`, `instance_costs` (the last cumulative `turn_end.totalCostUsd` of each agent instance, keyed by the instance index `seq / SEQ_STRIDE`; a `turn_end` adds the increase to `cost_usd` and to its `usage` item), record key; `gsi1` = (`T#<t>#RUNS`, `<started_at>#<run>`); `gsi2` = (`T#<t>#OPENRUNS`, `<started_at>#<run>`) until the status is `completed` or `failed` |
| `T#<t>#TASK#<task>` | `WS#<workspace>` | `workspace` | `run_id`, `compute`, `architecture` (`arm64` or `x86_64`), `cpu` and `memory_mib` (the Plan's size when the Workspace started), platform, `platform_ref`, `capacity_provider` (`FARGATE_SPOT` or `FARGATE`, absent before the launch and for local Workspaces), `started_at`, `signaled_at` (the agent reported `SIGTERM`; absent otherwise), `stopped_at`; `gsi1` = (`T#<t>#WORKSPACES`, `<started_at>#<id>`) |
| `T#<t>#TASK#<task>` | `SR#<number>` | `skill_run` | `state` (the engine's `SkillRun`), status, output. A task that succeeds or is stopped aborts its active skill runs in the same transaction (`FAILED` with `Skill.Aborted`, timers and waits cancelled, no notification), and timers and invocations of a finished task do nothing; `gsi1` = (`T#<t>#SKILLRUNS#<skill>`, `<started_at>#<id>`) |
| `T#<t>#TASK#<task>` | `SR#<number>#E#<sequence>` | `state_entry` | Unique per (skill run, sequence) |
| `T#<t>#TASK#<task>` | `SR#<number>#OUT#<kind>#<repo>#<number>` | `output` | Unique by key; `gsi1` = (`T#<t>#OUTPUTS`, `<created_at>#<id>`) |
| `T#<t>#TASK#<task>` | `SR#<number>#INV#<idempotency key>` | `resource_invocation` | `pending` → `awaiting` → `done` |
| `T#<t>#TASK#<task>` | `SR#<number>#CHECK#<token>`, `SR#<number>#APPROVAL#<token>` | `check_wait`, `approval_wait` | Check waits carry `gsi2` = (`SWEEP#CHECKWAIT`, `<next_poll_at>#<t>#<wait>`) while waiting. When the state of a wait closes without it (an error, a retry, a timeout, the skill run finishing or being aborted), the step sets the wait `cancelled` and deletes the check wait's `CHECKWAIT#` marker, so polling stops and its approval link is refused |
| `T#<t>#TASK#<task>` | `SR#<number>#TIMER#<timer id>` | `engine_timer` | `timer_id = uuid v5(skill run, timer)`, `due_at`, `message_id` of the scheduled job |
| `T#<t>#TASK#<task>` | `USAGE#<recorded_at>#<id>` | `usage` | Tokens and the cost added by each turn; `gsi1` = (`T#<t>#USAGE`, `<recorded_at>#<id>`) |
| `T#<t>#TASK#<task>` | `HELD#<created_at>#<dedupe key>` | `held_message` | A skill notification (`text`, `dedupe_key`) that arrived while the task was `NEEDS_YOU` for a reason other than `Approval.Requested`. It neither reaches the outbox nor wakes the Workspace; the human reply that resumes the task enqueues the held messages in order before the reply and deletes them |
| `T#<t>#TASK#<task>` | `CHAT#<created_at>#<id>` | `chat_message` | The agent's posts to the task's chat thread: `run_id`, `outbox_id`, `channel`, `author_kind`, `text`; written when the run's `chat_post` outbox items are delivered. Members' messages are the thread's inbound events |
| `T#<t>#RUN#<run>` | `#CHANNEL` | `run_channel` | `task_id`, `next_outbox_seq` (outbox order), `instances` (agent instances), the WebSocket connection id |
| `T#<t>#RUN#<run>` | `EVT#<seq, 20 digits>` or `EVT#~<received_at>#<id>` | `run_event` | Unique per (run, seq); events without a seq (`human_reply`) get a time key |
| `T#<t>#RUN#<run>` | `PEND#<received_at>#<event>` | `pending` | Unprocessed run events, in arrival order |
| `T#<t>#RUN#<run>` | `OUT#<seq>` / `OUTKEY#<dedupe key>` | `outbox` / `guard` | Outbox in `seq` order; the guard makes `(run, dedupe_key)` unique |
| `T#<t>#RUN#<run>` | `INST#<instance>` | `channel_instance` | `seq_offset` = index × `SEQ_STRIDE`, `processed_seq` |
| `T#<t>#RUN#<run>` | `LOG#<seq, 20 digits>` | `run_log_entry` | TTL = the plan's `retention_days` |
| `T#<t>#INBOUND` | `EVT#<source>#<key>` | `inbound_event` | Unique per (tenant, source, key); `processed_at`, and `error` when the event could not be evaluated at all; `gsi1` = (`T#<t>#THREAD#<source>#<thread>`, `<received_at>#<id>`) when the event has `detail.thread` |
| `T#<t>#INBOUND` | `PEND#<received_at>#<event>` | `pending` | Unprocessed inbound events, in arrival order |

The Postgres tables map as follows: `plans` → `PLANS`; `users` → `USER#`; `tenants` → `#TENANT`; `members` → `MEMBER#` + `membership`; `invitations` → `INVITE#` + `INVITEPENDING#` + `INVITEMAIL#`; `audit_events` → the audit table; `credentials`, `usage_snapshots`, `integrations`, `configs`, `usage_alerts` → tenant partition; `config_revisions`, `projects`, `repositories`, `sessions`, `skills`, `triggers`, `change_requests` → config partition; `trigger_events` → trigger partition; `tasks`, `task_dependencies`, `runs`, `workspaces`, `skill_runs`, `state_entries`, `outputs`, `usage`, `resource_invocations`, `check_waits`, `approval_waits`, `chat_messages` → task partition; `run_events`, `run_outbox`, `run_channel_instances`, `run_log_entries` → run partition; `inbound_events` → inbound partition; `due_jobs` → EventBridge Scheduler plus `engine_timer` items and `next_run_at`.

### Transactions

`TenantTx` is the unit of work. `get` and `query` read with strong consistency, remember each item's version, and return the transaction's own staged writes; `insert` writes with `attribute_not_exists(pk)`, `put` replaces with `v = <read version>`, `delete` checks the read version (an item deleted without a read must exist: `attribute_exists(pk)`), and `update` applies an update expression (counters with `ADD`, conditions with `cap_after` and `require`). An `update` of an item staged for `insert` or `put` folds into the staged item and rejects `cap_after` and `require` with `DbError::ConditionOnStagedPut`. `commit` sends everything, including message items and audit inserts, in one `TransactWriteItems` (at most 100 items, otherwise `DbError::TooManyWrites`). A failed condition or a concurrent transaction returns `DbError::Conflict`; callers re-run the whole operation (`store::retry`, `Worker::handle`, the GraphQL mutations). `list` reads a `Listing` (a partition or index key, a sort key range such as a prefix or a lower bound, ascending or descending, an optional limit) and keeps the items of one entity that pass a filter in code, paging until the limit is reached; it does not track versions. `TenantTx::list` rejects partitions outside the tenant like `get`. Indexes (`gsi1`, `gsi2`) are eventually consistent and are only used for listings and sweeps, never for decisions inside a transaction.

### How the Postgres guarantees carry over

| Guarantee | Postgres | DynamoDB |
|---|---|---|
| Tenant isolation | Row-level security on `tenant_id` | Every tenant key starts with `T#<t>`. `TenantTx` rejects any key outside its tenant (`DbError::CrossTenant`) except the global prefixes `UNIQ#`, `USER#`, `INVITEMAIL#`, `MSG#`, and rejects writes to `PLANS`. Every function runs under one role; isolation is the application check |
| Atomic engine steps | One SQL transaction per step | `persist_step` stages the skill run, entries, invocations, timers, notifications, and task decisions in one `TenantTx`; the step commits with one `TransactWriteItems`. A step makes at most `MAX_TRANSITIONS_PER_STEP` (70) state transitions, one `state_entry` write each, which leaves 30 items for the rest of the step; the next transition fails the skill run with `States.Runtime`. Config validation bounds every time value of a skill (`TimeoutSeconds`, `Seconds`, `IntervalSeconds`, `MaxDelaySeconds`) by the plan's `maxTaskTimeoutSeconds` and `BackoffRate` to 1–10, and the engine computes times with checked arithmetic, failing the skill run with `States.Runtime` when one overflows |
| Uniqueness and idempotency | `UNIQUE` constraints, `ON CONFLICT DO NOTHING` | Natural keys (`EVT#<key>`, `TASK#<name>`, `SR#<n>#E#<seq>`, `OUT#…`, `ALERT#…`) or guard items, written with `attribute_not_exists`; a losing writer gets `Conflict`, retries, and reads the winner (`Recorded::Duplicate`) |
| Concurrency cap | `FOR NO KEY UPDATE` of the tenant, `count(*)` of `RUNNING` | `running_tasks` counter on the tenant item, adjusted by every status change into or out of `RUNNING` (`tasks::store_task`); dispatch adds with the conditions `running_tasks <= max_concurrency − delta`, `tasks_this_period <= task_limit − delta`, and `status = active` |
| Queue claiming | `FOR UPDATE SKIP LOCKED` | SQS FIFO hands each message group to one consumer at a time; handlers re-read state, act only on `pending`/`waiting` items, and commit with version conditions, so a redelivered or concurrent message changes nothing twice |
| Outbox ordering | `GENERATED ALWAYS AS IDENTITY` | `next_outbox_seq` on the run's `#CHANNEL` item, read and incremented in the enqueuing transaction |
| Append-only audit | `REVOKE UPDATE, DELETE`, trigger | Separate audit table with the IAM deny above; the app only inserts with `attribute_not_exists` and `TenantTx` refuses `put`, `update`, and `delete` on it (`DbError::AppendOnly`) |
| Large values | `jsonb`, `text` | Offloaded to the `docs` bucket over 32 KiB, key in the item |
| Retention | Periodic deletes | TTL on audit events, usage snapshots, run log entries, and message items |
| Timers | `due_jobs` polled | EventBridge one-time schedules carrying the job; `engine_timer` items and `next_run_at` make cancellation and re-arming checkable |

### Access patterns

Reads marked "consistent" use the base table; "index" uses `gsi1`/`gsi2`; "app" means filtering or sorting in code over the returned items.

#### Worker

| Step | Keys |
|---|---|
| Process inbound events | Query `T#<t>#INBOUND` `PEND#` (consistent) → Get `EVT#<source>#<key>` |
| Event triggers of a tenant | Query `T#<t>` `CONFIG#` → per config Query `TRIGGER#` |
| Schedule trigger by id | Get `T#<t>#REF#<trigger>` → Get the trigger |
| Record trigger event | Get/Insert `T#<t>#TRIGGER#<tr>` `EVT#<key>` + ref |
| Requester by GitHub login | Get `UNIQ#ghlogin#<login>` → Get `USER#<u>` `MEMBER#<t>` |
| Parent and blockers by issue | Query index `gsi1` `T#<t>#TASKNAME#<repo>#<n>` (latest) |
| Depth and children count | Get parent tasks up the chain; Query parent `CHILD#`; Put the parent task with its read version, so the children of one parent are counted one at a time |
| Bot login of the App | `GitHubReader::app_login` (`GET /app`, once per process) |
| Receive limit | Get `T#<t>#TRIGGER#<tr>` `RATE#<hour>`; Insert it, or Update `starts` with `cap_after`, or Update `discarded` and audit `trigger.receive_limit` on the first discard of the hour |
| Chat thread tasks | Query `T#<t>#TRIGGER#<tr>` `TASK#<thread>/` |
| Session, project, skill of a config | Get `SESSION#`, `PROJECT#`, `SKILL#` in the config partition |
| Create task | Insert task, `TASK#<name>` guard, `PENDINGTASK#`, `DEP#`, `CHILD#`; update the trigger event's `task_id` |
| Dispatch | Get tenant, plan; Query `T#<t>` `PENDINGTASK#`; Get each task and its blockers; update tenant counters with caps |
| Run events | Query `T#<t>#RUN#<r>` `PEND#` → Get the event; Get `T#<t>#REF#<r>` → run |
| Latest run, runs of a task | Query `T#<t>#TASK#<task>` `RUN#` (app sort by `started_at`) |
| Latest skill status | Query `T#<t>#TASK#<task>` `SR#` (app) |
| Invocation by id (`exec_result` request id) | Get ref → Get `SR#<n>#INV#<key>` |
| Check waits for a check-run webhook | Query `T#<t>` `CHECKWAIT#` (app filter repo) → ref → wait |
| Due check waits, active tasks, stale configs, tenants | Query index `gsi2` `SWEEP#CHECKWAIT` (`≤ now`), `SWEEP#ACTIVETASK`, `SWEEP#CONFIG` (`≤ now − 5 min`), `TENANTS` |
| Engine timer | Get ref (skill run) → Get `SR#<n>#TIMER#<id>`, compare `message_id` |
| Workspace reconcile | Get task, run; Query `WS#` (app filter run, open); Query run `OUT#` (undelivered `user_message`/`exec`) |
| Project repositories for checkout | Get ref (trigger) → config → Query `PROJECT#<p>#REPO#` |
| Credential for a rate limit | Query `T#<t>` `CRED#` (app), Query `USAGESNAP#<cred>#` |
| Config sync | Get config, `REVSHA#<sha>` (→ ref → revision); Query `T#<t>` `CRED#`; insert the revision with the config's `synced_at` in one transaction. A valid revision that is not active is then activated: upserts of `PROJECT#`, `SESSION#`, `SKILL#`, `TRIGGER#`, then trigger removals and deletes of the definitions the revision drops (from Query `PROJECT#`, `SESSION#`, `SKILL#`, `TRIGGER#`), committed in batches of at most 90 writes; finally Put the config with `active_revision_id` and audit `config.activate` (revision number, commit sha, and the commit's author from `ConfigSource::commit_author`). An activation that stops between batches resumes on the next sync of the same head |

#### Screens and api

| Screen or endpoint | Keys |
|---|---|
| Sign-in (`provision`) | Get `UNIQ#cognito#<sub>` → Get `USER#<u>`; create with Insert `USER#`, `UNIQ#cognito`, `UNIQ#email` |
| Memberships, tenant switcher | Query `USER#<u>` `MEMBER#` → Get each `T#<t>` `MEMBER#<member>` (role) and `#TENANT` (app sort by name) |
| Membership in one tenant (every tenant-scoped field and mutation) | Get `USER#<u>` `MEMBER#<t>` → Get `T#<t>` `MEMBER#<member>`; no membership is `NOT_FOUND` |
| `tenant(slug)` | Get `UNIQ#slug#<slug>` → Get `USER#<u>` `MEMBER#<t>`; null for non-members, operators included |
| Viewer invitations | Query `INVITEMAIL#<email>` → Get invitations (app filter pending, unexpired, not a member) |
| Accept invitation | Query `INVITEMAIL#<email>` for the invitation's tenant; Put invitation (version), delete `INVITEPENDING#`, insert member + membership, audit |
| Plans, billing | Query `PLANS` |
| Home setup state | Get `USER#<u>`; Query `T#<t>` `INTEG#`, `CONFIG#`, `CRED#`; per config Query `TRIGGER#`, `PROJECT#`; Query index `gsi1` `T#<t>#TASKS` limit 1 |
| Home usage and activity | Tenant counters (`running_tasks`, `tasks_this_period`); seats by Query `T#<t>` `MEMBER#`, `INVITE#` (app filter pending, unexpired); running runs by Query index `gsi2` `T#<t>#OPENRUNS`; Query index `gsi1` `T#<t>#RUNS`, `T#<t>#USAGE`, `T#<t>#OUTPUTS` from the earlier of midnight and 24 hours ago (app sums and buckets) |
| Task list, chats, counts per trigger or member | Query index `gsi1` `T#<t>#TASKS` descending, stopping at 200 matches (`TASK_LISTING_LIMIT`; app filter by status, trigger, skill, channel source, requester; app sort `NEEDS_YOU` first, then `updated_at`); `tasksLast7d` starts the query at seven days ago. The lists and counts cover the latest 200 matching tasks |
| Task row and detail | Get `T#<t>#TASK#<task>` `#TASK`; Query the task partition with prefixes `RUN#`, `DEP#`, `CHILD#`, `SR#` (app); trigger event, config revision, and the trigger's config (`parent_id`) by ref; parent, children, and blockers by Get |
| Task by name | Query index `gsi1` `T#<t>#TASKNAME#<name>` (latest) |
| Task by trigger event | The trigger event's `task_id` |
| State entries, last N | Query `SR#<n>#E#` descending with a limit; count by Query `SR#<n>#E#` |
| Run log | Query run partition `EVT#`, `OUT#`, `LOG#` (from `after`); Query task `WS#` |
| Chat messages of a task | Get `T#<t>#INBOUND` `EVT#<source>#<trigger event key>` for the start; Query index `gsi1` `T#<t>#THREAD#<source>#<thread>` from the start, before the next thread task (Query trigger `TASK#<thread>/`); the agent's posts by Query `T#<t>#TASK#<task>` `CHAT#` |
| Send chat message | `record_inbound_event`, then the `ChatDispatcher` port (`dispatcher::dispatch_inbound` with the message's source and key); result by Get `T#<t>#TRIGGER#<tr>` `EVT#<key>` per trigger |
| Resume, snooze, stop a task; stop a run | Get the task; `record_run_event` (human reply), `tasks::abort_task`, or `tasks::stop_run`; Put the task; audit |
| Triggers | Per config Query `TRIGGER#` (app filter `removed_at`); counts by Query index `gsi1` `T#<t>#TASKS` (app); `next_run_at` on the trigger |
| Trigger events | Query index `gsi1` `T#<t>#TRIGGEREVENTS` descending, limit 20 (app filter evaluated, trigger) |
| Enable or disable a trigger | Get ref → Put the trigger after `dispatcher::schedule_trigger` (cancels the previous schedule, arms the next one while enabled); audit |
| Propose a trigger | Insert `T#<t>#CONFIG#<config>` `CR#<number>`; audit |
| Skills and skill stats | Per config Query `SKILL#`; Query index `gsi1` `T#<t>#SKILLRUNS#<skill>` descending (since, limit); entries per skill run |
| State activity of a skill | Query index `gsi1` `T#<t>#SKILLRUNS#<skill>` (app filter active or finished today); Get the task of each active run; Query `SR#<n>#E#` per run (app count succeeded today) |
| Config, revisions | Query `T#<t>` `CONFIG#`; Query config `REV#` descending, limit 10; active revision by ref |
| Projects, repositories, sessions, change requests | Query config `PROJECT#`, `PROJECT#<p>#REPO#`, `SESSION#`, `CR#`; by name Get `PROJECT#<name>`, `SESSION#<name>`, `SKILL#<name>` |
| Add config | Get `CONFIGREPO#` guard; Insert config + guard; audit; then the `ConfigSyncTrigger` port (`ConfigSync` or `config_sync::sync_config` inline) |
| Credentials | Query `T#<t>` `CRED#`; by name Get `CREDNAME#<name>`; latest usage Query `USAGESNAP#<cred>#` descending limit 1; add with Insert `CRED#` + `CREDNAME#` after checking the key prefix of the provider (`sk-ant-api` or `sk-ant-oat` for Anthropic, `sk-` but not `sk-ant-` for OpenAI) and storing `account` as the first and last four characters; remove deletes both. `Member.credentials` lists another member's personal credentials only to admins and owners |
| Members, invitations | Query `T#<t>` `MEMBER#` → Get users; Query `INVITE#`; invite with `INVITEPENDING#` guard, `INVITEMAIL#` item, and an update of the tenant item conditioned on its read version; role changes and removals take the same tenant condition, so seat and owner counts are checked one change at a time |
| Audit log, export | Query audit `T#<t>` descending from `since`, filter `actor_kind`, `actor_ref`, `action`, limit 100; actor names by Get `MEMBER#` + `USER#`, `USER#`, or ref (trigger, run → task); export writes all pages as JSON lines to `t/<t>/audit/<file>` in the records bucket and returns a presigned GET URL |
| Create tenant | `tenants::create_tenant`: Insert `#TENANT`, `UNIQ#slug`, member, membership; Free takes `UNIQ#free_tenant#<user>`, paid inserts `UNIQ#pending_checkout#<user>` after the Checkout Session exists; audit |
| Plan change, suspension | `billing::stage_plan_change`: Update `#TENANT` (`plan_id` with conditions on the current plan and status); a change to Free takes the free-tenant guard, starts a new period with zero counters, activates a tenant suspended for `payment_pending` or `payment_failed`, and drops its pending checkout; audit. `changePlan` to a paid plan while the subscription is `past_due`, `unpaid`, or `incomplete` returns the customer portal URL in `checkoutUrl` and keeps the plan |
| Operator | Query index `gsi2` `TENANTS`; per tenant Get `#TENANT`, Query index `gsi2` `T#<t>#OPENRUNS`, Query index `gsi1` `T#<t>#WORKSPACES` for run hours |
| Integrations, OAuth | Query `T#<t>` `INTEG#`; link GitHub with Insert `UNIQ#ghuser`, `UNIQ#ghlogin`; unlink with Put `USER#<u>` and Delete both guards, then an audit item per tenant of the user; installation with `UNIQ#installation#<id>` (+ tenant check) |
| GitHub webhook | `channel::ingest_github_delivery`: Get `UNIQ#installation#<id>` → integration → `record_inbound_event` |
| Stripe webhook | Get `UNIQ#stripe_event#<id>` (a recorded event answers `duplicate`); `billing::*`: Get `UNIQ#stripe#<customer>` → tenant, or the subscription's `metadata.tenant_id` (`parent.subscription_details.metadata.tenant_id` on invoices) for a customer that no checkout has linked yet; plans by Query `PLANS`; the change and the `stripe_event` item commit together. `checkout.session.completed` and `checkout.session.async_payment_succeeded` activate the tenant only with `payment_status = paid` (a new period with zero counters; an unpaid session links the customer and keeps `payment_pending`); `checkout.session.async_payment_failed` and `checkout.session.expired` discard a tenant still waiting for its checkout; `customer.subscription.updated` applies only `active`, `trialing`, and `past_due`; `invoice.paid` resumes a tenant suspended for `payment_failed` and resets the counters only for `billing_reason` `subscription_create` or `subscription_cycle` with a line period that starts after the stored `period_start`. Events for a tenant that is gone or not payable are recorded as `ignored` with the reason and answered 200; an unknown customer without a tenant hint answers 409 so Stripe retries |
| Requests by bare id (reply, approval, GraphQL ids) | For each of the user's tenants, Get `T#<t>#REF#<id>` (runs, triggers, trigger events, config revisions) or the item keyed by the id (`T#<t>#TASK#<id>` `#TASK`, `T#<t>` `MEMBER#<id>`, `INVITE#<id>`, `CRED#<id>`, `CONFIG#<id>`); an id of another tenant is never found |
| Run channel connect | Authorizer: Run token → `(tenant, run)`; Get ref → run status. `$connect`: Put `#CHANNEL` with the connection id; `$disconnect`: clear it when it is still that connection |
| Agent instance (`hello`) | Get or insert `INST#<instance>` with `seq_offset` from `#CHANNEL.instances`; Put `#CHANNEL` with the instance and `started`; emit `DeliverOutbox` |
| Sequenced message | Get `#CHANNEL` (current connection) and `INST#<instance>`; Put `INST#` with `processed_seq = seq` (version) and `record_run_event` with `(run, offset + seq)` in one transaction |
| Log line | Insert `LOG#<offset + seq>` with TTL from the plan's `retention_days` |
| Deliver outbox | `DeliverOutbox` job → `OutboxPusher`: `chat_post` items become `CHAT#` items; `undelivered_outbox` then `PostToConnection` to `#CHANNEL.connection_id`; after the posts succeed, `mark_outbox_delivered(run, seqs)` and `INST#.start_outbox_seq`; `GoneException` clears the connection and leaves the items undelivered for the next `hello` |
| Session record for restore | The run's `record_s3_key` |

## Open decisions

- Retention of task data: TTL covers audit, snapshots, logs, and messages. Deleting a finished task's partitions after the plan's `retention_days` needs a scheduled `purge` job; the offloaded `docs/` objects then need a lifecycle rule or reference tracking.
- Config activation writes the definitions in batches, so while it runs the config partition holds definitions of both revisions. Per-revision partitions behind the `active_revision_id` flip would make the switch atomic for readers.
- Audit tamper resistance stops at IAM: an app role with `PutItem` could overwrite an existing event by key. S3 Object Lock on the `t/<t>/audit/` objects of the records bucket would make exported history immutable.
- Hot partitions: `T#<t>` holds the tenant counters that every status change touches; at high volume the counter moves to its own sharded items. The sweep partitions in `gsi2` are single keys and may need shards.
