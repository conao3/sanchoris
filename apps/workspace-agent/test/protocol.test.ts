import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";
import type { ZodType } from "zod";

import {
  AckMessage,
  EventMessage,
  HeartbeatMessage,
  HelloMessage,
  InboundMessage,
  InterruptMessage,
  OutboundMessage,
  RateLimitMessage,
  ReadyMessage,
  RecordUploadRequestMessage,
  RecordUploadResponseMessage,
  SessionRecordMessage,
  ShutdownMessage,
  SkillRequestMessage,
  SkillResponseMessage,
  SocketRequest,
  SocketResponse,
  StartMessage,
  TurnEndMessage,
  UserMessage,
} from "../src/protocol.ts";
import { claudeRateLimit } from "../src/workers/claude.ts";
import { codexRateLimit, mergeCodexRateLimits } from "../src/workers/codex.ts";

const inbound: Record<string, ZodType> = {
  start: StartMessage,
  user_message: UserMessage,
  interrupt: InterruptMessage,
  shutdown: ShutdownMessage,
  skill_response: SkillResponseMessage,
  record_upload_response: RecordUploadResponseMessage,
  ack: AckMessage,
};

const outbound: Record<string, ZodType> = {
  hello: HelloMessage,
  heartbeat: HeartbeatMessage,
  ready: ReadyMessage,
  event: EventMessage,
  turn_end: TurnEndMessage,
  rate_limit: RateLimitMessage,
  skill_request: SkillRequestMessage,
  record_upload_request: RecordUploadRequestMessage,
  session_record: SessionRecordMessage,
};

const examples = [...readFileSync(resolve(import.meta.dirname, "../PROTOCOL.md"), "utf8").matchAll(/```json\n([\s\S]*?)```/g)].map(
  (match) => JSON.parse(match[1] as string) as Record<string, unknown>,
);

describe("PROTOCOL.md examples", () => {
  it("covers every message type", () => {
    const types = new Set(examples.map((example) => example.type).filter((type) => type !== undefined));
    expect([...types].sort()).toEqual([...Object.keys(inbound), ...Object.keys(outbound)].sort());
  });

  it.each(examples.map((example, index) => [index, example] as const))("example %i matches the schema", (_index, example) => {
    if (typeof example.type !== "string") {
      const schema = "action" in example ? SocketRequest : SocketResponse;
      expect(() => schema.parse(example)).not.toThrow();
      return;
    }
    const specific = inbound[example.type] ?? outbound[example.type];
    expect(specific).toBeDefined();
    expect(() => specific?.parse(example)).not.toThrow();
    const union = example.type in inbound ? InboundMessage : OutboundMessage;
    expect(() => union.parse(example)).not.toThrow();
  });
});

describe("rate limit mapping", () => {
  it("maps Claude unified windows", () => {
    expect(
      claudeRateLimit({
        status: "allowed",
        resetsAt: 1793061000,
        rateLimitType: "five_hour",
        unifiedWindows: {
          five_hour: { utilization: 0.04, resetsAt: 1793061000, status: "allowed" },
          seven_day: { utilization: 0.15, resetsAt: 1793476800, status: "allowed" },
        },
      }),
    ).toEqual({
      status: "allowed",
      resetsAt: 1793061000,
      rateLimitType: "five_hour",
      windows: {
        five_hour: { utilization: 0.04, resetsAt: 1793061000, status: "allowed" },
        seven_day: { utilization: 0.15, resetsAt: 1793476800, status: "allowed" },
      },
    });
  });

  it("maps a Claude event without unified windows to its single window", () => {
    expect(claudeRateLimit({ status: "rejected", resetsAt: 1793061000, rateLimitType: "seven_day", utilization: 1 })).toEqual({
      status: "rejected",
      resetsAt: 1793061000,
      rateLimitType: "seven_day",
      windows: { seven_day: { utilization: 1, resetsAt: 1793061000, status: "rejected" } },
    });
  });

  it("merges and maps Codex snapshots", () => {
    const first = mergeCodexRateLimits(null, {
      limitId: "codex",
      primary: { usedPercent: 40, resetsAt: 1793061000, windowDurationMins: 300 },
      secondary: { usedPercent: 10, resetsAt: 1793476800, windowDurationMins: 10080 },
    });
    const merged = mergeCodexRateLimits(first, {
      primary: { usedPercent: 100, resetsAt: 1793061000, windowDurationMins: 300 },
      rateLimitReachedType: "rate_limit_reached",
    });
    expect(mergeCodexRateLimits(merged, { limitId: "spark", primary: { usedPercent: 1 } })).toBe(merged);
    expect(codexRateLimit(merged ?? {})).toEqual({
      status: "rejected",
      resetsAt: 1793061000,
      rateLimitType: "rate_limit_reached",
      windows: {
        five_hour: { utilization: 1, resetsAt: 1793061000, status: "rejected" },
        seven_day: { utilization: 0.1, resetsAt: 1793476800, status: "allowed" },
      },
    });
  });
});
