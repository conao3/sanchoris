import { z } from "zod";

export const InputType = z.enum(["string", "number", "boolean", "array", "object"]);
export type InputType = z.infer<typeof InputType>;

export const SkillDefinition = z.strictObject({
  name: z.string().min(1),
  required: z.boolean(),
  inputs: z.record(z.string(), InputType),
});
export type SkillDefinition = z.infer<typeof SkillDefinition>;

export const WorkerAgent = z.enum(["claude", "codex"]);
export type WorkerAgent = z.infer<typeof WorkerAgent>;

const ResumeRecord = z.strictObject({
  sessionId: z.string().min(1),
  url: z.url(),
  relativePath: z.string().min(1),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
  lastTurn: z.number().int().nonnegative(),
});

const JsonObject = z.record(z.string(), z.unknown());

const ErrorBody = z.strictObject({
  code: z.string().min(1),
  message: z.string(),
});
export type ErrorBody = z.infer<typeof ErrorBody>;

const RunId = z.string().min(1);
const RequestId = z.string().min(1);
const SkillRunId = z.number().int().positive();

export const StartMessage = z.strictObject({
  type: z.literal("start"),
  runId: RunId,
  agent: WorkerAgent,
  model: z.string().min(1),
  systemPrompt: z.string().nullable(),
  skills: z.array(SkillDefinition),
  instruction: z.string().min(1).nullable(),
  resume: ResumeRecord.nullable(),
  heartbeatSeconds: z.number().int().positive(),
});
export type StartMessage = z.infer<typeof StartMessage>;

export const UserMessage = z.strictObject({
  type: z.literal("user_message"),
  runId: RunId,
  messageId: z.string().min(1),
  text: z.string().min(1),
});
export type UserMessage = z.infer<typeof UserMessage>;

export const InterruptMessage = z.strictObject({
  type: z.literal("interrupt"),
  runId: RunId,
});

export const ShutdownMessage = z.strictObject({
  type: z.literal("shutdown"),
  runId: RunId,
  reason: z.string(),
});

export const SkillResponseMessage = z.discriminatedUnion("ok", [
  z.strictObject({
    type: z.literal("skill_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(true),
    result: JsonObject,
  }),
  z.strictObject({
    type: z.literal("skill_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(false),
    error: ErrorBody,
  }),
]);
export type SkillResponseMessage = z.infer<typeof SkillResponseMessage>;

export const RecordUploadResponseMessage = z.discriminatedUnion("ok", [
  z.strictObject({
    type: z.literal("record_upload_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(true),
    url: z.url(),
    headers: z.record(z.string(), z.string()),
    key: z.string().min(1),
  }),
  z.strictObject({
    type: z.literal("record_upload_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(false),
    error: ErrorBody,
  }),
]);
export type RecordUploadResponseMessage = z.infer<typeof RecordUploadResponseMessage>;

export const GitTokenResponseMessage = z.discriminatedUnion("ok", [
  z.strictObject({
    type: z.literal("git_token_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(true),
    token: z.string().min(1),
    expiresAt: z.iso.datetime(),
  }),
  z.strictObject({
    type: z.literal("git_token_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(false),
    error: ErrorBody,
  }),
]);
export type GitTokenResponseMessage = z.infer<typeof GitTokenResponseMessage>;

export const WorkerCredentialResponseMessage = z.discriminatedUnion("ok", [
  z.strictObject({
    type: z.literal("worker_credential_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(true),
    env: z.record(z.string().regex(/^[A-Z][A-Z0-9_]*$/), z.string().min(1)),
  }),
  z.strictObject({
    type: z.literal("worker_credential_response"),
    runId: RunId,
    requestId: RequestId,
    ok: z.literal(false),
    error: ErrorBody,
  }),
]);
export type WorkerCredentialResponseMessage = z.infer<typeof WorkerCredentialResponseMessage>;

export const ExecMessage = z.strictObject({
  type: z.literal("exec"),
  runId: RunId,
  requestId: RequestId,
  purpose: z.enum(["git_push", "exec"]),
  commands: z.array(z.string().min(1)).min(1),
});
export type ExecMessage = z.infer<typeof ExecMessage>;

export const AckMessage = z.strictObject({
  type: z.literal("ack"),
  runId: RunId,
  seq: z.number().int().positive(),
});

export const CloseMessage = z.strictObject({
  type: z.literal("close"),
  runId: RunId,
  code: z.number().int().min(4000).max(4999),
  reason: z.string(),
});

export const InboundMessage = z.union([
  StartMessage,
  UserMessage,
  InterruptMessage,
  ShutdownMessage,
  ExecMessage,
  SkillResponseMessage,
  RecordUploadResponseMessage,
  GitTokenResponseMessage,
  WorkerCredentialResponseMessage,
  AckMessage,
  CloseMessage,
]);
export type InboundMessage = z.infer<typeof InboundMessage>;

export const TurnUsage = z.strictObject({
  inputTokens: z.number().int().nonnegative(),
  outputTokens: z.number().int().nonnegative(),
  cacheReadInputTokens: z.number().int().nonnegative(),
  cacheCreationInputTokens: z.number().int().nonnegative(),
});
export type TurnUsage = z.infer<typeof TurnUsage>;

export const TurnError = z.strictObject({
  code: z.enum(["Worker.UsageLimit", "Worker.Error"]),
  message: z.string(),
  resetAt: z.number().int().positive().nullable(),
});
export type TurnError = z.infer<typeof TurnError>;

export const RateLimitWindow = z.strictObject({
  utilization: z.number().nullable(),
  resetsAt: z.number().int().positive().nullable(),
  status: z.string().nullable(),
});
export type RateLimitWindow = z.infer<typeof RateLimitWindow>;

export const EventKind = z.enum(["text", "tool_use", "tool_result", "item", "system", "log"]);
export type EventKind = z.infer<typeof EventKind>;

export const HelloMessage = z.strictObject({
  type: z.literal("hello"),
  runId: RunId,
  protocolVersion: z.literal(1),
  instanceId: z.uuid(),
  started: z.boolean(),
  sessionId: z.string().nullable(),
});

export const HeartbeatMessage = z.strictObject({
  type: z.literal("heartbeat"),
  runId: RunId,
  at: z.iso.datetime(),
  busy: z.boolean(),
});

const sequenced = {
  runId: RunId,
  seq: z.number().int().positive(),
};

export const ReadyMessage = z.strictObject({
  type: z.literal("ready"),
  ...sequenced,
  sessionId: z.string().min(1),
  agent: WorkerAgent,
  model: z.string().min(1),
  resumed: z.boolean(),
});

export const EventMessage = z.strictObject({
  type: z.literal("event"),
  ...sequenced,
  turn: z.number().int().positive(),
  kind: EventKind,
  data: JsonObject,
  at: z.iso.datetime(),
});

export const TurnEndMessage = z.strictObject({
  type: z.literal("turn_end"),
  ...sequenced,
  turn: z.number().int().positive(),
  sessionId: z.string().min(1),
  result: z.enum(["completed", "failed", "interrupted"]),
  summary: z.string(),
  error: TurnError.nullable(),
  usage: TurnUsage,
  totalCostUsd: z.number().nonnegative().nullable(),
  durationMs: z.number().int().nonnegative(),
});

export const RateLimitMessage = z.strictObject({
  type: z.literal("rate_limit"),
  ...sequenced,
  status: z.string(),
  resetsAt: z.number().int().positive().nullable(),
  rateLimitType: z.string().nullable(),
  windows: z.record(z.string(), RateLimitWindow),
});

export const SkillRequestMessage = z.discriminatedUnion("action", [
  z.strictObject({
    type: z.literal("skill_request"),
    ...sequenced,
    requestId: RequestId,
    action: z.literal("run"),
    name: z.string().min(1),
    input: JsonObject,
  }),
  z.strictObject({
    type: z.literal("skill_request"),
    ...sequenced,
    requestId: RequestId,
    action: z.literal("continue"),
    skillRunId: SkillRunId,
    output: z.unknown().nullable(),
  }),
  z.strictObject({
    type: z.literal("skill_request"),
    ...sequenced,
    requestId: RequestId,
    action: z.literal("status"),
    skillRunId: SkillRunId,
  }),
]);

export const RecordUploadRequestMessage = z.strictObject({
  type: z.literal("record_upload_request"),
  ...sequenced,
  requestId: RequestId,
  sessionId: z.string().min(1),
  turn: z.number().int().positive(),
  contentLength: z.number().int().nonnegative(),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
});

export const SessionRecordMessage = z.strictObject({
  type: z.literal("session_record"),
  ...sequenced,
  sessionId: z.string().min(1),
  turn: z.number().int().positive(),
  key: z.string().min(1),
  relativePath: z.string().min(1),
  contentLength: z.number().int().nonnegative(),
  sha256: z.string().regex(/^[0-9a-f]{64}$/),
});

export const GitTokenRequestMessage = z.strictObject({
  type: z.literal("git_token_request"),
  ...sequenced,
  requestId: RequestId,
});

export const WorkerCredentialRequestMessage = z.strictObject({
  type: z.literal("worker_credential_request"),
  ...sequenced,
  requestId: RequestId,
});

export const ExecCommandResult = z.strictObject({
  command: z.string(),
  exitCode: z.number().int(),
  stdout: z.string(),
  stderr: z.string(),
});
export type ExecCommandResult = z.infer<typeof ExecCommandResult>;

export const ExecResultMessage = z.strictObject({
  type: z.literal("exec_result"),
  ...sequenced,
  requestId: RequestId,
  ok: z.boolean(),
  results: z.array(ExecCommandResult),
});

export const OutboundMessage = z.union([
  HelloMessage,
  HeartbeatMessage,
  ReadyMessage,
  EventMessage,
  TurnEndMessage,
  RateLimitMessage,
  SkillRequestMessage,
  RecordUploadRequestMessage,
  SessionRecordMessage,
  ExecResultMessage,
  GitTokenRequestMessage,
  WorkerCredentialRequestMessage,
]);
export type OutboundMessage = z.infer<typeof OutboundMessage>;

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

export type SequencedMessage = Extract<OutboundMessage, { seq: number }>;
export type SequencedPayload = DistributiveOmit<SequencedMessage, "runId" | "seq">;

export const SocketRequest = z.discriminatedUnion("action", [
  z.strictObject({
    action: z.literal("run"),
    name: z.string().min(1),
    input: z.unknown(),
  }),
  z.strictObject({
    action: z.literal("continue"),
    skillRunId: SkillRunId,
    output: z.unknown().nullable(),
  }),
  z.strictObject({
    action: z.literal("status"),
    skillRunId: SkillRunId,
  }),
  z.strictObject({
    action: z.literal("git_credential"),
    host: z.string().min(1),
  }),
]);
export type SocketRequest = z.infer<typeof SocketRequest>;

export const SocketResponse = z.discriminatedUnion("ok", [
  z.strictObject({ ok: z.literal(true), result: JsonObject }),
  z.strictObject({ ok: z.literal(false), error: ErrorBody }),
]);
export type SocketResponse = z.infer<typeof SocketResponse>;
