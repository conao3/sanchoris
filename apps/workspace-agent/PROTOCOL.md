# Run channel protocol (version 1)

The Run channel connects one `workspace-agent` process (inside a Workspace) to the control plane (`api`). The schema in `src/protocol.ts` is the source of truth; every JSON example in this file is parsed against it by `test/protocol.test.ts`.

## Environment

The `runner` starts `workspace-agent` with one JSON document on stdin and these variables. A missing or empty one stops the agent with an error.

The JSON document carries the secrets, so that they never sit in an environment that the worker or `exec` commands inherit: `runToken` (the Run's short-lived bearer token, always) and `relayUrl` (Fargate workspaces: the egress relay with the Run's relay token as the password, `http://run:<token>@<relay>:3128`). The agent reads stdin to its end before it starts. The Fargate image's entrypoint (`infra/workspace/entrypoint.sh`) receives `SANCHORIS_RUN_TOKEN` and `SANCHORIS_RELAY_URL` from the task, removes them from the environment, and `exec`s the agent with the document on stdin, so `/proc/1/environ` of the agent holds neither.

| Variable | When | Value |
|---|---|---|
| `SANCHORIS_RUN_ID` | always | The Run id (`runId` in every message) |
| `SANCHORIS_CONTROL_URL` | always | WebSocket URL of the Run channel (`ws://` or `wss://`) |
| `SANCHORIS_SOCKET_PATH` | always | Unix socket path for the `sanchoris` command (also set in the worker's environment) |
| `SANCHORIS_WORKSPACE_DIR` | always | Working directory of the agent session |
| `SANCHORIS_CREDENTIAL_SOURCE` | always | `environment`: the worker's credentials are already in its environment and the runner checked out the repositories (local workspaces). `channel`: the agent asks the control plane for the worker credential and GitHub tokens over the Run channel and checks out the repositories itself (Fargate workspaces) |
| `SANCHORIS_GIT_URL`, `SANCHORIS_BRANCH`, `SANCHORIS_REPOSITORIES` | `channel` | Git origin (`https://github.com`), the task branch, and the comma-separated `owner/name` repositories to check out |
| `HTTPS_PROXY`, `HTTP_PROXY`, `NO_PROXY` | with `relayUrl` | `http://127.0.0.1:<port>` and `127.0.0.1,localhost` (the image sets port 3129). The agent listens on that port and forwards each `CONNECT` to `relayUrl` with the relay token as `Proxy-Authorization`, so the Run channel, record uploads (with `NODE_USE_ENV_PROXY=1`), `git`, and the worker reach the relay without holding its token |
| `CLAUDE_CONFIG_DIR`, `SANCHORIS_CLAUDE_PATH` | `agent: "claude"` | Claude config directory (credentials, session records) and the `claude` executable |
| `CODEX_HOME`, `SANCHORIS_CODEX_PATH` | `agent: "codex"` | Codex home (credentials, session records) and the `codex` executable |

The worker and `exec` commands run with the agent's environment, which holds neither the Run token nor the relay token (with the `environment` source it holds the worker's own credentials), plus, for the worker with the `channel` source, the credential proxy's variables. `sanchoris` must be on `PATH`.

## Transport

- The agent opens a WebSocket to `SANCHORIS_CONTROL_URL` (`ws://` or `wss://`) with the header `Authorization: Bearer <SANCHORIS_RUN_TOKEN>`. It sends a WebSocket ping every 30 s, so proxies that close idle connections (Cloudflare closes a WebSocket after 100 s without frames) keep the connection open.
- The control plane rejects an invalid or expired token at the upgrade with HTTP `401` (or `403`). The agent treats that as fatal and exits with code 1.
- Each WebSocket text frame is one JSON object. Every object has `type` and `runId`. `runId` is an opaque string equal to `SANCHORIS_RUN_ID`. Unknown fields are rejected on both sides.
- An inbound message that fails validation, or carries another `runId`, is a protocol error: the agent exits with code 1.
- In AWS the control plane is an API Gateway WebSocket API: a Lambda authorizer checks the `Authorization` header on `$connect`, and messages are answered from Lambda through the API Gateway Management API. Locally `sanchoris all` serves the same protocol at `/runs/channel`.
- The control plane ends a Run by sending `shutdown`, or rejects the agent by sending `close` (a code in `4000`–`4999` and a reason) and closing the socket; the agent exits with code 1 without reconnecting. The agent treats a WebSocket close code in `4000`–`4999` the same way. Any other close or network error makes the agent reconnect with exponential backoff (0.5 s doubling up to 30 s, reset after a successful upgrade).

## Delivery

- Each agent process has an `instanceId` (UUID) sent in `hello`. A new Workspace, or a restarted agent, has a new `instanceId`.
- Outbound messages other than `hello` and `heartbeat` carry `seq`, a positive integer that increases by 1 per message within one agent process, starting at 1. Sequence numbers are scoped to the `instanceId` of the connection's `hello`.
- The control plane answers `hello` with `ack` (`seq` = the highest sequence number of that `instanceId` it has persisted, `0` for a new instance) and acknowledges each sequenced message with `ack` (`seq` = the highest sequence number of the current `instanceId` it has persisted; acks are cumulative). The agent sends sequenced messages only after the `ack` of its `hello` and keeps one sequenced message in flight: the API Gateway WebSocket API invokes Lambda once per frame and concurrently, so two frames sent back to back may be processed out of order. Unacknowledged messages are sent again in order after every reconnect. The control plane must ignore a message whose `(instanceId, seq)` it has already processed and should re-ack it.
- Inbound messages are not acknowledged. `user_message` carries `messageId`; the agent drops a `messageId` it has already seen in this process, so the control plane may resend after a reconnect.
- `hello` is sent first on every connection. `started: false` means the agent has not received `start` in this process; the control plane then sends `start`. `started: true` means it must not send `start` again (a second `start` is a protocol error).

## Messages: control plane to agent

| `type` | Fields | Meaning |
|---|---|---|
| `start` | `agent`, `model`, `systemPrompt`, `skills`, `instruction`, `resume`, `heartbeatSeconds` | Start (or restore) the agent session. Sent once per agent process, after `hello` with `started: false` |
| `user_message` | `messageId`, `text` | A user turn: `[sanchoris]` notifications, a person's reply, "Continue where you left off.", chat messages |
| `interrupt` | | Interrupt the current turn. The turn ends with `turn_end` `result: "interrupted"` |
| `shutdown` | `reason` | Finish: the agent waits for the in-flight record upload and commands, stops the worker, waits up to 10 s for all `ack`s, closes with code 1000, and exits 0 |
| `exec` | `requestId`, `purpose`, `commands` | Run shell commands in the workspace for a skill (`purpose`: `git_push` for `git:push`, `exec` for `workspace:exec`). The agent answers with `exec_result` |
| `skill_response` | `requestId`, `ok`, `result` or `error` | Reply to `skill_request` |
| `record_upload_response` | `requestId`, `ok`, `url`, `headers`, `key` or `error` | Reply to `record_upload_request` |
| `git_token_response` | `requestId`, `ok`, `token` and `expiresAt` or `error` | Reply to `git_token_request` |
| `worker_credential_response` | `requestId`, `ok`, `env` or `error` | Reply to `worker_credential_request` |
| `ack` | `seq` | Cumulative acknowledgement of outbound messages, and the answer to `hello` |
| `close` | `code`, `reason` | The control plane rejects this agent (`4000`: protocol error). The agent exits with code 1 without reconnecting |

`start` fields:

- `agent`: `"claude"` or `"codex"`.
- `model`: the Worker model, fixed for the session.
- `systemPrompt`: string appended to the worker's system prompt (Project `SystemPrompt`), or `null`.
- `skills`: the Trigger's `Start.Skills` that have `skill.yaml`. Each has `name`, `required` (in `Start.RequiredSkills`), and `inputs` (the skill's `Inputs`: field name to `"string" | "number" | "boolean" | "array" | "object"`).
- `instruction`: the first user turn, or `null` to wait for `user_message`.
- `resume`: `null` for a new session, or the last saved record: `sessionId`, `url` (presigned GET), `relativePath` and `sha256` (from the `session_record` being restored), `lastTurn` (the `turn` of that record; new turns continue from `lastTurn + 1`).
- `heartbeatSeconds`: interval of `heartbeat`.

```json
{
  "type": "start",
  "runId": "42",
  "agent": "claude",
  "model": "claude-sonnet-5-5",
  "systemPrompt": "You are working on the Acme shop. Always write tests for new endpoints.",
  "skills": [{ "name": "open-pr-green-ci", "required": true, "inputs": { "Repos": "array", "Issue": "number", "MaxFixes": "number" } }],
  "instruction": "[sanchoris] Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\nInstruction: Resolve issue 35 in acme/shop-web.\nSkills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`.",
  "resume": null,
  "heartbeatSeconds": 60
}
```

```json
{
  "type": "start",
  "runId": "42",
  "agent": "claude",
  "model": "claude-sonnet-5-5",
  "systemPrompt": null,
  "skills": [{ "name": "open-pr-green-ci", "required": true, "inputs": { "Repos": "array", "Issue": "number", "MaxFixes": "number" } }],
  "instruction": "[sanchoris] Skill open-pr-green-ci (run 1) needs you: CI failed on acme/shop-web#52 (checkout_spec::flaky).\nFix it, commit, and run `sanchoris skill continue 1`.",
  "resume": {
    "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10",
    "url": "https://records.s3.amazonaws.com/t/7/runs/42/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10/turn-3.jsonl?X-Amz-Signature=abc",
    "relativePath": "projects/-workspace/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10.jsonl",
    "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    "lastTurn": 3
  },
  "heartbeatSeconds": 60
}
```

```json
{ "type": "user_message", "runId": "42", "messageId": "skill-1-succeeded", "text": "[sanchoris] Skill open-pr-green-ci (run 1) SUCCEEDED: {\"PullRequests\":[\"acme/shop-web#52\"]}" }
```

```json
{ "type": "interrupt", "runId": "42" }
```

```json
{ "type": "exec", "runId": "42", "requestId": "3d0f8f6e-6b8e-4d7a-9b1c-2a4e5f6a7b80", "purpose": "git_push", "commands": ["git -C 'shop-web' push origin 'HEAD:refs/heads/sanchoris/7c1e2d3f-4a5b-4c6d-8e9f-0a1b2c3d4e5f'"] }
```

```json
{ "type": "shutdown", "runId": "42", "reason": "idle for 900 seconds" }
```

```json
{ "type": "skill_response", "runId": "42", "requestId": "b0b5e0a4-5d1e-4a8e-9a43-1f5d1c1b2a01", "ok": true, "result": { "RunId": 1, "Status": "RUNNING" } }
```

```json
{ "type": "skill_response", "runId": "42", "requestId": "b0b5e0a4-5d1e-4a8e-9a43-1f5d1c1b2a02", "ok": false, "error": { "code": "skill_not_allowed", "message": "Skill deploy is not in Start.Skills" } }
```

```json
{
  "type": "record_upload_response",
  "runId": "42",
  "requestId": "c7d2f9b0-6a3e-4f1b-8c55-2e9d0b7a4c11",
  "ok": true,
  "url": "https://records.s3.amazonaws.com/t/7/runs/42/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10/turn-4.jsonl?X-Amz-Signature=def",
  "headers": { "content-type": "application/octet-stream", "x-amz-checksum-sha256": "q1MKE+RZFJgrefm34/uplM/R8/si9xzqGvvwK0YMbR0=" },
  "key": "t/7/runs/42/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10/turn-4.jsonl"
}
```

```json
{ "type": "record_upload_response", "runId": "42", "requestId": "c7d2f9b0-6a3e-4f1b-8c55-2e9d0b7a4c12", "ok": false, "error": { "code": "run_finished", "message": "Run 42 is no longer running" } }
```

```json
{ "type": "ack", "runId": "42", "seq": 17 }
```

```json
{ "type": "close", "runId": "42", "code": 4000, "reason": "expected seq 4, received 9" }
```

## Messages: agent to control plane

| `type` | Sequenced | Fields | Meaning |
|---|---|---|---|
| `hello` | no | `protocolVersion` (`1`), `instanceId`, `started`, `sessionId` (string or `null`) | First message on every connection |
| `heartbeat` | no | `at` (ISO 8601), `busy` (a turn is running) | Every `heartbeatSeconds` after `start`, and once immediately |
| `ready` | yes | `sessionId`, `agent`, `model`, `resumed` | The worker session is ready. `sessionId` is the Claude session ID or the Codex thread ID; on resume it equals `resume.sessionId` |
| `event` | yes | `turn`, `kind`, `data`, `at` | One line of agent output (S3 log and screen summary) |
| `rate_limit` | yes | `status`, `resetsAt`, `rateLimitType`, `windows` | A usage-limit snapshot from the worker |
| `skill_request` | yes | `requestId`, `action`, and `name` + `input` (`run`), `skillRunId` + `output` (`continue`), `skillRunId` (`status`) | A `sanchoris skill` command; the control plane answers with `skill_response` |
| `record_upload_request` | yes | `requestId`, `sessionId`, `turn`, `contentLength`, `sha256` | Asks for a presigned PUT URL for the session record after a turn |
| `session_record` | yes | `sessionId`, `turn`, `key`, `relativePath`, `contentLength`, `sha256` | The record of `turn` has been uploaded to `key` |
| `turn_end` | yes | `turn`, `sessionId`, `result`, `summary`, `error`, `usage`, `totalCostUsd`, `durationMs` | A turn ended |
| `exec_result` | yes | `requestId`, `ok`, `results` | The outcome of `exec` |
| `git_token_request` | yes | `requestId` | Asks for a GitHub token for `git` (`channel` source only) |
| `worker_credential_request` | yes | `requestId` | Asks for the worker credential before the session starts (`channel` source only) |

Field details:

- `turn`: 1-based number of the turn within the Run (continues from `resume.lastTurn`). `event.turn` is the turn in progress.
- `event.kind` and `data`:
  - `text`: `{ "text": string }` assistant text.
  - `tool_use`: `{ "id": string | null, "name": string, "input": object }`.
  - `tool_result`: `{ "toolUseId": string | null, "isError": boolean, "content": string }` (content truncated to 4000 characters).
  - `item`: `{ "itemType": string, "id": string | null }` another Codex item.
  - `system`: `{ "subtype": string }` worker system message.
  - `log`: `{ "line": string }` one stderr line of the worker process.
- `rate_limit`: `status` is `"allowed"`, `"allowed_warning"` or `"rejected"`. `resetsAt` is Unix seconds or `null`. `windows` maps a window name (`five_hour`, `seven_day`, `seven_day_opus`, ...) to `{ utilization (0–1 or null), resetsAt (Unix seconds or null), status (string or null) }`. Claude values come from `rate_limit_event`; Codex values from `account/rateLimits/updated` (`usedPercent / 100`).
- `turn_end.result`: `completed`, `failed`, or `interrupted`. `summary` is the final assistant text (or the error text). `error` is `null` unless `failed`: `{ code: "Worker.UsageLimit" | "Worker.Error", message, resetAt }`; `resetAt` (Unix seconds) is set for `Worker.UsageLimit` when the provider reported it.
- `turn_end.usage`: tokens of this turn's main loop: `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens`. `totalCostUsd` is Claude's cumulative estimated cost reported by the SDK since this agent process opened the session; `null` for Codex. A restored session starts again from 0 (observed with Claude Code 2.1.310), so the Run's cost is the sum over `instanceId`s of the last `totalCostUsd` of each.
- `skill_request.output` is the parsed `--output` JSON, or `null` when the option was omitted.

```json
{ "type": "hello", "runId": "42", "protocolVersion": 1, "instanceId": "0d6f1c3a-8b2e-4f57-a1c9-5e7b2d4f6a80", "started": false, "sessionId": null }
```

```json
{ "type": "heartbeat", "runId": "42", "at": "2026-10-27T03:15:00.000Z", "busy": true }
```

```json
{ "type": "ready", "runId": "42", "seq": 1, "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10", "agent": "claude", "model": "claude-sonnet-5-5", "resumed": false }
```

```json
{ "type": "event", "runId": "42", "seq": 2, "turn": 1, "kind": "tool_use", "data": { "id": "toolu_01", "name": "Bash", "input": { "command": "sanchoris skill run open-pr-green-ci --input '{\"Repos\":[\"acme/shop-web\"],\"Issue\":35,\"MaxFixes\":3}'" } }, "at": "2026-10-27T03:15:02.000Z" }
```

```json
{
  "type": "rate_limit",
  "runId": "42",
  "seq": 3,
  "status": "allowed",
  "resetsAt": 1793061000,
  "rateLimitType": "five_hour",
  "windows": {
    "five_hour": { "utilization": 0.04, "resetsAt": 1793061000, "status": "allowed" },
    "seven_day": { "utilization": 0.15, "resetsAt": 1793476800, "status": "allowed" }
  }
}
```

```json
{ "type": "skill_request", "runId": "42", "seq": 4, "requestId": "b0b5e0a4-5d1e-4a8e-9a43-1f5d1c1b2a01", "action": "run", "name": "open-pr-green-ci", "input": { "Repos": ["acme/shop-web"], "Issue": 35, "MaxFixes": 3 } }
```

```json
{ "type": "skill_request", "runId": "42", "seq": 5, "requestId": "b0b5e0a4-5d1e-4a8e-9a43-1f5d1c1b2a02", "action": "continue", "skillRunId": 1, "output": { "Fixed": true } }
```

```json
{ "type": "skill_request", "runId": "42", "seq": 6, "requestId": "b0b5e0a4-5d1e-4a8e-9a43-1f5d1c1b2a03", "action": "status", "skillRunId": 1 }
```

```json
{ "type": "record_upload_request", "runId": "42", "seq": 7, "requestId": "c7d2f9b0-6a3e-4f1b-8c55-2e9d0b7a4c11", "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10", "turn": 1, "contentLength": 184320, "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" }
```

```json
{ "type": "session_record", "runId": "42", "seq": 8, "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10", "turn": 1, "key": "t/7/runs/42/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10/turn-1.jsonl", "relativePath": "projects/-workspace/5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10.jsonl", "contentLength": 184320, "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08" }
```

```json
{
  "type": "turn_end",
  "runId": "42",
  "seq": 9,
  "turn": 1,
  "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10",
  "result": "completed",
  "summary": "Started open-pr-green-ci (run 1). Waiting for the result.",
  "error": null,
  "usage": { "inputTokens": 1200, "outputTokens": 340, "cacheReadInputTokens": 21110, "cacheCreationInputTokens": 751 },
  "totalCostUsd": 0.0123,
  "durationMs": 18234
}
```

```json
{
  "type": "turn_end",
  "runId": "42",
  "seq": 30,
  "turn": 5,
  "sessionId": "5f0c6a8e-2d55-4c43-9a51-0a7b8f3d2e10",
  "result": "failed",
  "summary": "Claude usage limit reached.",
  "error": { "code": "Worker.UsageLimit", "message": "Claude usage limit reached.", "resetAt": 1793061000 },
  "usage": { "inputTokens": 0, "outputTokens": 0, "cacheReadInputTokens": 0, "cacheCreationInputTokens": 0 },
  "totalCostUsd": 0.4561,
  "durationMs": 912
}
```

```json
{
  "type": "exec_result",
  "runId": "42",
  "seq": 31,
  "requestId": "3d0f8f6e-6b8e-4d7a-9b1c-2a4e5f6a7b80",
  "ok": true,
  "results": [{ "command": "git -C 'shop-web' push origin 'HEAD:refs/heads/sanchoris/7c1e2d3f-4a5b-4c6d-8e9f-0a1b2c3d4e5f'", "exitCode": 0, "stdout": "", "stderr": "" }]
}
```

## Commands (`exec`)

The control plane sends `exec` when a skill state runs commands in the workspace (`git:push`, `workspace:exec`). The agent runs `commands` one at a time with `/bin/sh -c`, in `SANCHORIS_WORKSPACE_DIR` (each repository is cloned into a directory named after the repository), with the agent's environment. It stops at the first command that exits non-zero and sends `exec_result`: `ok` is `true` when every command exited 0, and `results` has one entry per command that ran (`exitCode`, and the last 16000 characters of `stdout` and `stderr`). `exec` requests run in the order received, independently of the worker's turns. The agent drops an `exec` whose `requestId` it has already seen in this process.

## Credentials (`channel` source)

A Fargate workspace holds no provider credential when it starts. After `start` and before it starts the worker, the agent sends `worker_credential_request`; the control plane decrypts the Session's `Worker.Credential` and answers with one variable (an Anthropic API key as `ANTHROPIC_API_KEY`, a Claude Code OAuth token from `claude setup-token` as `CLAUDE_CODE_OAUTH_TOKEN`). The agent keeps the value in memory and starts a credential proxy on `127.0.0.1` with an ephemeral port: the worker gets `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>` and the same variable with a placeholder value, and the proxy forwards each request to `https://api.anthropic.com` (through the relay) after it replaces the client's `Authorization` or `x-api-key` with the real credential (`Authorization: Bearer <token>` and the `oauth-2025-04-20` beta for an OAuth token, `x-api-key` for an API key). Claude Code sends every model request to `ANTHROPIC_BASE_URL` with an OAuth token as well as with an API key. Any other variable, or more than one, stops the agent. A refusal stops the agent with exit code 1. The agent then clones `SANCHORIS_REPOSITORIES` from `SANCHORIS_GIT_URL` into `SANCHORIS_WORKSPACE_DIR` and checks out `SANCHORIS_BRANCH` (from `origin` when it exists, otherwise a new branch).

Git asks for credentials through the helper `sanchoris git-credential` (the workspace image configures it). The helper sends `git_credential` with the host over the command socket; the agent answers only for the host of `SANCHORIS_GIT_URL`, with `x-access-token` and a token from `git_token_request`, which it reuses until five minutes before `expiresAt`. The control plane issues a GitHub App installation token narrowed to the Session's `GitHub.Repos` (all repositories of the installation when a pattern is `owner/*`) and `GitHub.Permissions`; it expires after one hour.

```json
{ "type": "worker_credential_request", "runId": "42", "seq": 1, "requestId": "6a1f0c2e-1d4b-4e8a-9c3f-7b2d5e8a1c90" }
```

```json
{ "type": "worker_credential_response", "runId": "42", "requestId": "6a1f0c2e-1d4b-4e8a-9c3f-7b2d5e8a1c90", "ok": true, "env": { "CLAUDE_CODE_OAUTH_TOKEN": "sk-ant-oat01-example" } }
```

```json
{ "type": "git_token_request", "runId": "42", "seq": 2, "requestId": "9e4d2b7a-3c1f-4a6e-8b5d-0f2a7c9e1b34" }
```

```json
{ "type": "git_token_response", "runId": "42", "requestId": "9e4d2b7a-3c1f-4a6e-8b5d-0f2a7c9e1b34", "ok": true, "token": "ghs_example", "expiresAt": "2026-10-27T04:15:00Z" }
```

```json
{ "type": "git_token_response", "runId": "42", "requestId": "9e4d2b7a-3c1f-4a6e-8b5d-0f2a7c9e1b35", "ok": false, "error": { "code": "credential_unavailable", "message": "the session's GitHub.Repos spans more than one GitHub installation" } }
```

```json
{ "action": "git_credential", "host": "github.com" }
```

## Request and response correlation

`skill_request` and `record_upload_request` carry a `requestId` (UUID) chosen by the agent. The control plane answers each exactly once with the matching `skill_response` or `record_upload_response`, echoing `requestId`. A resent request (same `seq` after a reconnect) must not be answered twice. The agent waits 30 s for a reply; a reply that arrives after that is ignored, and a reply for an unknown `requestId` is a protocol error.

`skill_response`:

- `ok: true`: `result` is a JSON object printed verbatim by the `sanchoris` command. For `run` and `continue` it is `{ "RunId": <SkillRun id>, "Status": "RUNNING" }` (or the SkillRun's current status). For `status` it is the SkillRun state, for example `{ "RunId": 1, "Skill": "open-pr-green-ci", "Status": "WAITING_AGENT", "Output": null, "Error": null, "Cause": null }`.
- `ok: false`: `error` is `{ code, message }`. The command prints `sanchoris: <code>: <message>` and exits 1. The control plane rejects a `run` whose skill is not in `Start.Skills` or whose input does not match `Inputs`, and a `continue` whose SkillRun is not `WAITING_AGENT` or belongs to another Task.

## Session records

After every turn, before `turn_end`:

1. The agent reads the worker's session record (Claude: `$CLAUDE_CONFIG_DIR/projects/<dir>/<sessionId>.jsonl`; Codex: the rollout file under `$CODEX_HOME/sessions/`) and sends `record_upload_request` with its byte length and SHA-256.
2. The control plane answers with a presigned PUT `url`, the `headers` the agent must send with it (for example the SHA-256 checksum), and the object `key`.
3. The agent uploads the whole file with `PUT` and sends `session_record`. `relativePath` is the path of the record relative to the worker home (`CLAUDE_CONFIG_DIR` or `CODEX_HOME`).
4. The agent sends `turn_end`.

If the upload fails or the control plane refuses it, the agent exits with code 1.

To restore, the control plane sends `start` with `resume` built from the latest `session_record` and a presigned GET `url` for its `key`. The agent downloads the file, verifies `sha256` (mismatch is fatal), places it (Claude: `projects/<the new workspace dir, non-alphanumerics replaced by ->/<sessionId>.jsonl`; Codex: `relativePath` under `CODEX_HOME`), and resumes by session ID (`resume` in the Claude Agent SDK, `thread/resume` in the Codex app-server).

## `sanchoris` command socket

The `sanchoris` command talks to the agent over the Unix socket `SANCHORIS_SOCKET_PATH` (mode 0600). One request per connection: the command writes one JSON line, the agent writes one JSON line and closes.

```json
{ "action": "run", "name": "open-pr-green-ci", "input": { "Repos": ["acme/shop-web"], "Issue": 35, "MaxFixes": 3 } }
```

```json
{ "action": "continue", "skillRunId": 1, "output": null }
```

```json
{ "action": "status", "skillRunId": 1 }
```

```json
{ "ok": true, "result": { "RunId": 1, "Status": "RUNNING" } }
```

```json
{ "ok": false, "error": { "code": "unknown_skill", "message": "Skill deploy is not available in this task. Available skills: open-pr-green-ci" } }
```

Agent-side error codes: `not_started` (no `start` yet), `unknown_skill`, `invalid_input` (the input is not an object, misses a field of `Inputs`, has a field of the wrong type, or has a field not in `Inputs`), `invalid_request`, `agent_error` (for example, no reply from the control plane). Other codes come from the control plane's `skill_response`.
