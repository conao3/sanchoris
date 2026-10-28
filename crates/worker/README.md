# sanchoris-worker

Message handlers of the control plane. Every piece of background work is a `sanchoris_core::jobs::Job` carried in an `Envelope`; `Worker::handle(&envelope)` runs one job, retries it on optimistic-concurrency conflicts, and returns how much it changed. In AWS the handlers run in the worker Lambda behind the SQS FIFO work queue (`Worker::handle_body` takes the message body); locally `driver::LocalDriver` runs them in process against `MemoryQueue` and `MemoryScheduler`. The data model, the queues, and the schedules are described in `docs/architecture.md`.

Jobs are written as message items in the same DynamoDB transaction as the state change that causes them, so a committed change always produces its follow-up work. Handlers are idempotent: they re-read the items they act on and do nothing when the work is already done or a newer message superseded it.

| Job | Emitted by | Does |
|---|---|---|
| `Inbound` | `record_inbound_event` (webhooks, chat, schedules) | Processes the tenant's pending inbound events in order: evaluates enabled triggers, records `trigger_events` with `started`/`skipped`/`discarded`, creates tasks, and dispatches pending tasks. `Check Run Completed` also wakes matching check waits |
| `Dispatch` | `create_task`, a task leaving `RUNNING`, billing changes | Starts pending tasks while the plan's concurrency and task limits allow |
| `RunEvents` | `record_run_event` | Processes the run's pending events in order: `ready`, `heartbeat`, `session_record`, `turn_end`, `rate_limit`, `exec_result`, `human_reply`. Every event from the agent (a `heartbeat` only while `busy`) sends `TaskHeartbeat` to the run's skill runs in `WAITING_AGENT`, so an `agent:handoff` stays alive while the agent works on it |
| `Invoke` | the engine's `InvokeResource` effect | Runs the resource and feeds `ResourceResult` back, or sends an `exec` and waits for `exec_result` |
| `EngineTimer` | the engine's `ScheduleTimer` effect, at the due time | Feeds `TimerDue` to the engine unless the timer was cancelled or re-armed or the task is finished; a timer that arrives before its due time keeps its item and is scheduled again |
| `CheckPoll` | `github:checks`, check-run webhooks, `SweepChecks` | Reads the PR head and its check runs; completes the wait with `TaskSuccess` |
| `TaskResume` | a usage-limit wait, at `ResumeAt` | Sends "Continue where you left off." |
| `Reconcile` | a run being created, finished, or sent a `user_message`/`exec`; a task becoming idle, at the idle deadline | Starts a workspace for a run that needs one, stops a finished or idle one |
| `DeliverOutbox` | `enqueue_outbox` | Calls `OutboxPusher` so the Run channel delivers the run's outbox |
| `ConfigSync` | `SweepConfigs`, the config screen, push webhooks | Reads the head commit through `ConfigSource`, validates it, stores the revision, and on success activates projects, repositories, sessions, skills, and triggers (Project repo keys in `detail.repository` become `owner/name`) and schedules `rate`/`cron` triggers |
| `TriggerSchedule` | trigger activation, the previous firing | Writes a `schedule` inbound event and schedules the next firing |
| `SweepConfigs` | rate schedule | Emits `ConfigSync` for configs not synced within 5 minutes |
| `SweepChecks` | rate schedule | Emits `CheckPoll` for check waits whose poll is due |
| `SweepTaskLimits` | rate schedule | Applies `Start.Limits` to `RUNNING`/`WAITING` tasks; over the limit, `NEEDS_YOU` and an `interrupt` |
| `UsageAlerts` | rate schedule | Writes `usage_alerts` at 80% of the plan |
| `SweepWorkspaces` | rate schedule | Asks `RunnerPort::workspace_exit` about the open workspaces of `RUNNING`/`WAITING` tasks; a workspace that stopped on its own is closed, its run goes `idle` with the reason as `error`, and a `RUNNING` task goes `NEEDS_YOU` (`Workspace.Stopped`, the reason as `cause`). Then `RunnerPort::sweep` (the ECS runner scales the egress relay to 0 when no Workspace runs) |

## Contract with the api

The api writes inbound items and delivers outbox items; it never drives the engine except through the `sanchoris_core` functions listed here. Every write runs in a `TenantTx` and is retried by the caller when `commit` returns `DbError::Conflict`.

### Writes

- GitHub webhooks: `sanchoris_core::channel::ingest_github_delivery(store, delivery_id, event_name, payload)`. Chat messages: `record_inbound_event(tx, "web" | "slack", key, event)` with `detail.thread`, `detail.user`, `detail.text`, and the scope fields the triggers match.
- Run channel messages from the workspace agent: `record_run_event(tx, run_id, Some(seq), type, payload)` with the message minus `type`, `runId`, and `seq`. Types: `ready`, `heartbeat`, `turn_end`, `rate_limit`, `session_record`, `exec_result`.
- A member's reply on the Task screen: `record_run_event(tx, latest_run, None, human_reply, {"member", "text"})`.
- `skill_request`: answer synchronously with `skillrun::accept_skill_request` (`run`), `continue_skill_run` (`continue`), or `skill_run_status` (`status`), and reply with `skill_response`. Errors carry `SkillRunError::code()`.
- Approvals: `skillrun::resolve_approval(tx, signer, signed_token, approved, member, now)`. It refuses a token whose wait is not `waiting` or that the skill run no longer waits for, before it changes the task. Show `TaskTokenSigner::sign(approval_waits.skill_run_id, approval_waits.task_token)` to the user; the raw token never leaves the control plane.

### Deliveries

Outbox items per run, ordered by `seq`, with `delivered_at` set through `mark_outbox_delivered(tx, run_id, seqs, at)` once the Run channel has posted them to the connection (`OutboxPusher`). Send the latest undelivered `start` first. The api adds `runId`; for `user_message` it also adds `messageId` (the item id).

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

`GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY`, `GITHUB_API_URL`, `RUN_TOKEN_SIGNING_KEY`, `SANCHORIS_WORKSPACE_COMMAND`, `SANCHORIS_CONTROL_URL`, `SANCHORIS_SOCKET_DIR`, `SANCHORIS_WORKSPACES_DIR`, `SANCHORIS_GIT_URL`, and the store variables in `docs/architecture.md` (`SANCHORIS_TABLE_MAIN`, `SANCHORIS_TABLE_AUDIT`, `SANCHORIS_DOCS_BUCKET`, and in AWS `SANCHORIS_WORK_QUEUE_URL`, `SANCHORIS_WORK_QUEUE_ARN`, `SANCHORIS_SCHEDULE_GROUP`, `SANCHORIS_SCHEDULER_ROLE_ARN`). The api also needs `TASK_TOKEN_SIGNING_KEY` for approvals. All are required.

`LocalProcessRunner` creates a directory per workspace under `SANCHORIS_WORKSPACES_DIR`, clones each repository from `SANCHORIS_GIT_URL/<owner>/<name>.git` into a directory named after the repository, checks out the task branch (from `origin` when it exists), and starts `SANCHORIS_WORKSPACE_COMMAND` there with `SANCHORIS_RUN_ID`, `SANCHORIS_RUN_TOKEN` (signed over the tenant id and the run id), `SANCHORIS_CONTROL_URL`, `SANCHORIS_SOCKET_PATH`, and `SANCHORIS_WORKSPACE_DIR`. Stopping the workspace kills the process and removes the directory.

## Tests

The acceptance tests run against DynamoDB Local through `sanchoris_core::testing::TestDynamo` (`dynamodb-local` is in the nix devShell) and drive the handlers with `LocalDriver::settle`, which fires due schedules, runs the periodic sweeps, and drains the queue until nothing changes.
