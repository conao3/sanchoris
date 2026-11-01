# Code adapted from T3 Code

The worker adapters are adapted from [T3 Code](https://github.com/pingdotgg/t3code) at commit `044f07257c9684ae4543bd1e1ebf9c394e9e3830`, under the MIT License below. T3 Code builds its adapters on Effect; the parts used here are rewritten as plain async TypeScript with no Effect dependency. No file is copied verbatim.

| File here | Adapted from | What was taken |
|---|---|---|
| `src/workers/claude.ts` | `apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts` | Running one Claude Agent SDK `query()` per session in streaming-input mode fed by a prompt queue; the shape of `SDKUserMessage` for a text turn (`makeClaudeUserMessage`); `resume` versus `sessionId` thread identity; reading `rate_limit_event` (`resetsAt` in Unix seconds, `status: "rejected"` as the usage-limit signal) and the `result` message as the end of a turn; `interrupt()` on the query; `thinking: { type: "adaptive", display: "summarized" }` with `settings.showThinkingSummaries` and the `thinking` blocks of `assistant` messages; a `canUseTool` callback next to `bypassPermissions` that answers `AskUserQuestion` with `updatedInput.answers` keyed by the question text, multiple labels joined with ", " (`claudeUserInputQuestions`, `claudeSdkUserInputAnswers`); the runtime and interaction modes mapped to `permissionMode` (`plan`, `default`, `acceptEdits`, `bypassPermissions`) and `setPermissionMode` when they change; approvals in `canUseTool` answered with `allow`, `allow` plus the suggested permission updates for the session, or `deny` (with `interrupt` for cancel), and plan-mode tools denied under full access; `ExitPlanMode` captured as the proposed plan and denied with T3 Code's message; `TodoWrite` read as the task list (`claudeTodoSteps`); a steer pushed as a user message with `priority: "now"`; `includePartialMessages` with the `text_delta` and `thinking_delta` of `stream_event` for the text being written; attached images as base64 `image` blocks before the text with a line per image naming where it is saved (`attachmentPrompt.ts`) |
| `src/checkpoint.ts` | `apps/server/src/vcs/GitVcsDriver.ts`, `apps/server/src/checkpointing/CheckpointCaptureService.ts` | Checkpoints of the workspace at turn boundaries: the working tree written with a temporary `GIT_INDEX_FILE` so the person's index and `HEAD` stay untouched, kept under a private ref namespace, and the per-turn file list from `git diff --numstat -z <from> <to>` with the patch from `git diff` |
| `src/workers/codex-rpc.ts` | `packages/effect-codex-app-server/src/protocol.ts`, `src/_internal/shared.ts` | The Codex app-server wire format: newline-delimited JSON without a `jsonrpc` field, requests `{ id, method, params }`, notifications `{ method, params }`, responses `{ id, result | error }`, numeric request ids, and server-initiated requests answered by id |
| `src/workers/codex.ts` | `apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts`, `apps/server/src/provider/CodexProvider.ts`, `apps/server/src/provider/codexUsageLimits.ts`, `packages/effect-codex-app-server/src/_generated/schema.gen.ts` | The session lifecycle `initialize` (with `capabilities.experimentalApi`) → `initialized` → `thread/start` or `thread/resume` → `turn/start` → `turn/completed`, `turn/interrupt`; the `full-access` runtime mode (`approvalPolicy: "never"`, `sandboxPolicy: { type: "dangerFullAccess" }`); `thread.path` as the rollout file; `thread/tokenUsage/updated` for per-turn usage; `account/rateLimits/updated` handling (`mergeCodexRateLimits`, primary and secondary windows by `windowDurationMins`, main allowance only); `codexErrorInfo` values for usage limits; the `item/tool/requestUserInput` server request and its response `{ answers: { <id>: { answers } } }` (`toCodexUserInputAnswers`); the runtime modes as `approvalPolicy` with `sandboxPolicy` per turn and the `item/commandExecution/requestApproval` and `item/fileChange/requestApproval` server requests answered with `{ decision }`; `collaborationMode` for plan mode, the thread config `tools.update_plan.enabled`, `turn/plan/updated` as the task list (`codexPlanStepStatus`), and `plan` items as the proposed plan; `turn/steer` with `expectedTurnId` for a steer; `item/agentMessage/delta` and `item/reasoning/summaryTextDelta` for the text being written; attached images as `image` input items with a data URL after the text item; the `summary` and `content` of `reasoning` items, and the fields of `fileChange`, `mcpToolCall` and `webSearch` items |

## License

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
