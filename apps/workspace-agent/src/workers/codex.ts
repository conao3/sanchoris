import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative } from "node:path";

import type { RateLimitWindow } from "../protocol.ts";
import {
  emptyUsage,
  requireEnv,
  type RestoredRecord,
  type Worker,
  type WorkerContext,
  type WorkerRateLimit,
  type WorkerRecord,
  type WorkerTurnResult,
} from "../worker.ts";
import { CodexAppServerClient } from "./codex-rpc.ts";

const SESSION_MINUTES = 5 * 60;
const WEEK_MINUTES = 7 * 24 * 60;

interface CodexRateLimitWindow {
  usedPercent: number;
  resetsAt?: number | null;
  windowDurationMins?: number | null;
}

export interface CodexRateLimitSnapshot {
  limitId?: string | null;
  planType?: string | null;
  rateLimitReachedType?: string | null;
  primary?: CodexRateLimitWindow | null;
  secondary?: CodexRateLimitWindow | null;
}

interface CodexThread {
  id: string;
  path?: string | null;
}

interface CodexTokenUsage {
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  cacheWriteInputTokens?: number;
}

interface CodexTurn {
  id: string;
  status: "completed" | "interrupted" | "failed" | "inProgress";
  error?: { message: string; codexErrorInfo?: unknown } | null;
  startedAt?: number | null;
  completedAt?: number | null;
  durationMs?: number | null;
}

interface CodexItem {
  type: string;
  id?: string;
  text?: string;
  command?: string;
  exitCode?: number | null;
  aggregatedOutput?: string | null;
}

function windowName(window: CodexRateLimitWindow, fallbackMinutes: number): string {
  const minutes = typeof window.windowDurationMins === "number" ? window.windowDurationMins : fallbackMinutes;
  if (minutes === SESSION_MINUTES) return "five_hour";
  if (minutes === WEEK_MINUTES) return "seven_day";
  return `${minutes}_minutes`;
}

export function mergeCodexRateLimits(
  previous: CodexRateLimitSnapshot | null,
  update: CodexRateLimitSnapshot,
): CodexRateLimitSnapshot | null {
  if (update.limitId && update.limitId !== "codex") return previous;
  if (!previous) return update;
  return {
    ...previous,
    ...(update.limitId !== undefined ? { limitId: update.limitId } : {}),
    ...(update.planType !== undefined ? { planType: update.planType } : {}),
    ...(update.rateLimitReachedType !== undefined ? { rateLimitReachedType: update.rateLimitReachedType } : {}),
    ...(update.primary !== undefined ? { primary: update.primary } : {}),
    ...(update.secondary !== undefined ? { secondary: update.secondary } : {}),
  };
}

export function codexRateLimit(snapshot: CodexRateLimitSnapshot): WorkerRateLimit {
  const windows: Record<string, RateLimitWindow> = {};
  const positions = [
    [snapshot.primary, SESSION_MINUTES],
    [snapshot.secondary, WEEK_MINUTES],
  ] as const;
  let latestReset: number | null = null;
  for (const [window, fallback] of positions) {
    if (!window || !Number.isFinite(window.usedPercent)) continue;
    const resetsAt = typeof window.resetsAt === "number" && window.resetsAt > 0 ? window.resetsAt : null;
    windows[windowName(window, fallback)] = {
      utilization: window.usedPercent / 100,
      resetsAt,
      status: window.usedPercent >= 100 ? "rejected" : "allowed",
    };
    if (window.usedPercent >= 100 && resetsAt !== null) latestReset = Math.max(latestReset ?? 0, resetsAt);
  }
  return {
    status: snapshot.rateLimitReachedType ? "rejected" : "allowed",
    resetsAt: latestReset,
    rateLimitType: snapshot.rateLimitReachedType ?? null,
    windows,
  };
}

function isUsageLimit(info: unknown): boolean {
  return info === "usageLimitExceeded" || info === "rateLimitExceeded";
}

export class CodexWorker implements Worker {
  private readonly context: WorkerContext;
  private readonly codexHome: string;
  private readonly executable: string;
  private child: ChildProcessWithoutNullStreams | null = null;
  private client: CodexAppServerClient | null = null;
  private threadId: string | null = null;
  private recordPath: string | null = null;
  private activeTurnId: string | null = null;
  private readonly completedTurnIds = new Set<string>();
  private turnStarting = false;
  private readonly queued: string[] = [];
  private closing = false;
  private lastText = "";
  private lastUsage: CodexTokenUsage | null = null;
  private rateLimits: CodexRateLimitSnapshot | null = null;

  constructor(context: WorkerContext) {
    this.context = context;
    this.codexHome = requireEnv(context.env, "CODEX_HOME");
    this.executable = requireEnv(context.env, "SANCHORIS_CODEX_PATH");
  }

  async start(restore: RestoredRecord | null): Promise<{ sessionId: string }> {
    if (restore) {
      if (isAbsolute(restore.relativePath) || restore.relativePath.split("/").includes("..")) {
        throw new Error(`Invalid Codex record path ${restore.relativePath}`);
      }
      const path = join(this.codexHome, restore.relativePath);
      await mkdir(dirname(path), { recursive: true });
      await writeFile(path, restore.bytes);
      this.recordPath = path;
    }
    const child = spawn(this.executable, ["app-server"], {
      cwd: this.context.workspaceDir,
      env: this.context.env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.child = child;
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (data: string) => {
      for (const line of data.split("\n")) if (line !== "") this.context.handlers.onEvent("log", { line });
    });
    child.on("exit", (code, signal) => {
      const error = new Error(`Codex app-server exited (code ${String(code)}, signal ${String(signal)})`);
      this.client?.rejectAll(error);
      if (!this.closing) this.context.handlers.onFatal(error);
    });
    const client = new CodexAppServerClient(child, {
      onNotification: (method, params) => this.handleNotification(method, params),
      onRequest: async (method) => {
        this.context.handlers.onEvent("log", { line: `Codex request ${method} was declined` });
        throw new Error(`sanchoris does not handle ${method}`);
      },
      onProtocolError: (error) => this.context.handlers.onFatal(error),
    });
    this.client = client;
    await client.request("initialize", {
      clientInfo: { name: "sanchoris", title: "sanchoris", version: "0.1.0" },
      capabilities: { experimentalApi: true },
    });
    client.notify("initialized");
    const threadParams = {
      cwd: this.context.workspaceDir,
      model: this.context.model,
      approvalPolicy: "never",
      sandbox: "danger-full-access",
      ...(this.context.systemPrompt === null ? {} : { developerInstructions: this.context.systemPrompt }),
    };
    const response = (
      restore
        ? await client.request("thread/resume", { threadId: restore.sessionId, ...threadParams })
        : await client.request("thread/start", threadParams)
    ) as { thread: CodexThread };
    this.threadId = response.thread.id;
    if (!restore) {
      if (!response.thread.path) throw new Error("Codex did not report a session record path");
      this.recordPath = response.thread.path;
    }
    return { sessionId: response.thread.id };
  }

  send(text: string): void {
    this.queued.push(text);
    this.startNextTurn();
  }

  async interrupt(): Promise<void> {
    if (!this.client || !this.threadId || !this.activeTurnId) return;
    await this.client.request("turn/interrupt", { threadId: this.threadId, turnId: this.activeTurnId });
  }

  async readRecord(): Promise<WorkerRecord> {
    if (!this.recordPath) throw new Error("Codex session is not running");
    return { relativePath: relative(this.codexHome, this.recordPath), bytes: await readFile(this.recordPath) };
  }

  async close(): Promise<void> {
    this.closing = true;
    this.child?.kill("SIGTERM");
  }

  private startNextTurn(): void {
    if (this.activeTurnId || this.turnStarting || !this.client || !this.threadId) return;
    const text = this.queued.shift();
    if (text === undefined) return;
    this.turnStarting = true;
    this.lastText = "";
    this.lastUsage = null;
    this.client
      .request("turn/start", {
        threadId: this.threadId,
        input: [{ type: "text", text, text_elements: [] }],
        approvalPolicy: "never",
        sandboxPolicy: { type: "dangerFullAccess" },
        model: this.context.model,
      })
      .then(
        (response) => {
          this.turnStarting = false;
          const turnId = (response as { turn: { id: string } }).turn.id;
          if (this.completedTurnIds.has(turnId)) {
            this.startNextTurn();
            return;
          }
          this.activeTurnId = turnId;
        },
        (error: Error) => this.context.handlers.onFatal(error),
      );
  }

  private handleNotification(method: string, params: unknown): void {
    const { onEvent, onRateLimit } = this.context.handlers;
    switch (method) {
      case "item/completed": {
        const item = (params as { item: CodexItem }).item;
        if (item.type === "agentMessage") {
          this.lastText = item.text ?? "";
          onEvent("text", { text: this.lastText });
        } else if (item.type === "commandExecution") {
          onEvent("tool_use", { id: item.id ?? null, name: "commandExecution", input: { command: item.command ?? null } });
          onEvent("tool_result", {
            toolUseId: item.id ?? null,
            isError: (item.exitCode ?? 0) !== 0,
            content: (item.aggregatedOutput ?? "").slice(0, 4000),
          });
        } else {
          onEvent("item", { itemType: item.type, id: item.id ?? null });
        }
        return;
      }
      case "thread/tokenUsage/updated":
        this.lastUsage = (params as { tokenUsage: { last: CodexTokenUsage } }).tokenUsage.last;
        return;
      case "account/rateLimits/updated": {
        const merged = mergeCodexRateLimits(this.rateLimits, (params as { rateLimits: CodexRateLimitSnapshot }).rateLimits);
        if (!merged) return;
        this.rateLimits = merged;
        onRateLimit(codexRateLimit(merged));
        return;
      }
      case "turn/completed":
        this.completeTurn((params as { turn: CodexTurn }).turn);
        return;
      default:
        return;
    }
  }

  private completeTurn(turn: CodexTurn): void {
    this.activeTurnId = null;
    this.completedTurnIds.add(turn.id);
    const usage = this.lastUsage;
    let result: WorkerTurnResult["result"] = "completed";
    let error: WorkerTurnResult["error"] = null;
    if (turn.status === "interrupted") {
      result = "interrupted";
    } else if (turn.status === "failed") {
      result = "failed";
      const message = turn.error?.message ?? "Codex turn failed";
      error = isUsageLimit(turn.error?.codexErrorInfo)
        ? {
            code: "Worker.UsageLimit",
            message,
            resetAt: this.rateLimits ? codexRateLimit(this.rateLimits).resetsAt : null,
          }
        : { code: "Worker.Error", message, resetAt: null };
    }
    const durationMs =
      turn.durationMs ??
      (typeof turn.startedAt === "number" && typeof turn.completedAt === "number"
        ? Math.round((turn.completedAt - turn.startedAt) * 1000)
        : 0);
    this.context.handlers.onTurnEnd({
      result,
      summary: result === "failed" && error ? error.message : this.lastText,
      error,
      usage: usage
        ? {
            inputTokens: usage.inputTokens,
            outputTokens: usage.outputTokens,
            cacheReadInputTokens: usage.cachedInputTokens,
            cacheCreationInputTokens: usage.cacheWriteInputTokens ?? 0,
          }
        : emptyUsage(),
      totalCostUsd: null,
      durationMs,
    });
    this.startNextTurn();
  }
}
