# sanchoris-worker

Background loops of the control plane. Each loop claims rows with `FOR UPDATE SKIP LOCKED` and processes them idempotently, so any number of worker processes can run against the same database. `Worker::tick(Loop)` runs one loop once; `Worker::run` runs all loops until shutdown; `run_from_env` is the entry point for the `api` binary's worker subcommand.

| Loop | Claims | Does |
|---|---|---|
| `ConfigSyncer` | `configs` with `synced_at` older than 5 minutes or `NULL` | Reads the head commit through `ConfigSource`, validates with the plan limits and credential names, stores `config_revisions`, and on success upserts `projects`, `repositories`, `sessions`, `skills`, and `triggers` (Project repo keys in `detail.repository` become `owner/name`) |
| `Scheduler` | `due_jobs` of kind `trigger_schedule` | Writes a `schedule` inbound event per due `rate`/`cron` trigger and schedules the next one |
| `TriggerDispatcher` | `inbound_events` | Evaluates enabled triggers, records `trigger_events` with `started`/`skipped`/`discarded`, creates tasks, and dispatches pending tasks. `Check Run Completed` also wakes matching `check_waits` |
| `EngineTimers` | `due_jobs` of kind `engine_timer` | Feeds `TimerDue` to the engine |
| `ResourceInvocations` | `resource_invocations` in `pending` | Runs the resource and feeds `ResourceResult` back in the same transaction |
| `CheckPoller` | `check_waits` due for a poll | Reads the PR head and its check runs; completes the wait with `TaskSuccess` |
| `RunEvents` | `run_events` | `ready`, `heartbeat`, `session_record`, `turn_end`, `rate_limit`, `exec_result`, `human_reply` |
| `TaskResumes` | `due_jobs` of kind `task_resume` | Sends "Continue where you left off." after `ResumeAt` |
| `TaskLimits` | `RUNNING`/`WAITING` tasks | Applies `Start.Limits`; over the limit, `NEEDS_YOU` and an `interrupt` |
| `Workspaces` | runs without an open workspace, idle workspaces | Starts workspaces through `RunnerPort` with a `start` message; stops idle ones after `Workspace.IdleSeconds` |
| `PendingDispatch` | tenants with `PENDING` tasks | Starts tasks when slots free up |
| `LimitsEnforcer` | active tenants | Writes `usage_alerts` at 80% of the plan |

## Contract with the api

The api writes inbound rows and delivers outbox rows; it never drives the engine except through the `sanchoris_core` functions listed here.

### Writes

- GitHub webhooks: `sanchoris_core::channel::ingest_github_delivery(pool, delivery_id, event_name, payload)` with a pool whose role can read `integrations` across tenants. Chat messages: `record_inbound_event(tx, "web" | "slack", key, event)` with `detail.thread`, `detail.user`, `detail.text`, and the scope fields the triggers match.
- Run channel messages from the workspace agent: `record_run_event(tx, run_id, Some(seq), type, payload)` with the message minus `type`, `runId`, and `seq`. Types: `ready`, `heartbeat`, `turn_end`, `rate_limit`, `session_record`, `exec_result`.
- A member's reply on the Task screen: `record_run_event(tx, latest_run, None, human_reply, {"member", "text"})`.
- `skill_request`: answer synchronously with `skillrun::accept_skill_request` (`run`), `continue_skill_run` (`continue`), or `skill_run_status` (`status`), and reply with `skill_response`. Errors carry `SkillRunError::code()`.
- Approvals: `skillrun::resolve_approval(tx, signer, signed_token, approved, member, now)`. Show `TaskTokenSigner::sign(approval_waits.skill_run_id, approval_waits.task_token)` to the user; the raw token never leaves the control plane.

### Deliveries

`run_outbox` rows per run, ordered by `seq`, with `delivered_at` set after the agent acknowledges. Send the latest undelivered `start` first. The api adds `runId`; for `user_message` it also adds `messageId` (the row id).

| `type` | `payload` |
|---|---|
| `start` | `agent`, `model`, `systemPrompt`, `skills` (`name`, `required`, `inputs`), `instruction` (the start message, `null` on restore), `resume` (`null` or `sessionId`, `recordKey`, `lastTurn`; presign `recordKey` into the agent's `resume.url`), `heartbeatSeconds`, `branch`, `repositories` |
| `user_message` | `text` |
| `exec` | `requestId`, `purpose` (`git_push` or `exec`), `commands` |
| `interrupt` | `{}` |
| `shutdown` | `reason` |
| `chat_post` | `text`, `channel` (the task's channel). Post to the chat thread; do not send to the agent |

### `exec` and `exec_result`

`git:push` and `workspace:exec.sync` run in the workspace. The agent runs `commands` in order in the workspace root (each repository is cloned into a directory named after the repository), stops at the first failure, and answers with `exec_result`:

```json
{ "type": "exec_result", "runId": "...", "seq": 12, "requestId": "<from exec>", "ok": true,
  "results": [{ "command": "git -C 'shop-web' push origin 'HEAD:refs/heads/sanchoris/<task id>'", "exitCode": 0, "stdout": "", "stderr": "" }] }
```

The task branch is `sanchoris/<task id>`.

## Environment

`DATABASE_URL`, `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, `RUN_TOKEN_SIGNING_KEY`, `SANCHORIS_WORKSPACE_COMMAND`, `SANCHORIS_CONTROL_URL`, `SANCHORIS_SOCKET_DIR`. The api also needs `TASK_TOKEN_SIGNING_KEY` for approvals. All are required.
