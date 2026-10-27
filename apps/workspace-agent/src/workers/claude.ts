import { randomUUID } from "node:crypto";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join, relative } from "node:path";

import {
  query,
  type Query,
  type SDKMessage,
  type SDKRateLimitInfo,
  type SDKResultMessage,
  type SDKUserMessage,
} from "@anthropic-ai/claude-agent-sdk";

import { AsyncQueue } from "../async-queue.ts";
import type { RateLimitWindow } from "../protocol.ts";
import {
  requireEnv,
  type RestoredRecord,
  type Worker,
  type WorkerContext,
  type WorkerRateLimit,
  type WorkerRecord,
  type WorkerTurnResult,
} from "../worker.ts";

const MAX_TOOL_RESULT_CHARS = 4000;

type UnifiedWindow = { utilization?: number; resetsAt?: number; status?: string };
type RateLimitInfoWithWindows = SDKRateLimitInfo & { unifiedWindows?: Record<string, UnifiedWindow> };

export function claudeProjectDirName(cwd: string): string {
  return cwd.replace(/[^a-zA-Z0-9]/g, "-");
}

function toWindow(window: UnifiedWindow): RateLimitWindow {
  return {
    utilization: window.utilization ?? null,
    resetsAt: window.resetsAt ?? null,
    status: window.status ?? null,
  };
}

export function claudeRateLimit(info: RateLimitInfoWithWindows): WorkerRateLimit {
  const windows: Record<string, RateLimitWindow> = {};
  if (info.unifiedWindows) {
    for (const [name, window] of Object.entries(info.unifiedWindows)) windows[name] = toWindow(window);
  } else if (info.rateLimitType) {
    windows[info.rateLimitType] = toWindow(info);
  }
  return {
    status: info.status,
    resetsAt: info.resetsAt ?? null,
    rateLimitType: info.rateLimitType ?? null,
    windows,
  };
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((block: { type?: string; text?: string }) => (block.type === "text" ? (block.text ?? "") : ""))
      .join("");
  }
  return "";
}

export class ClaudeWorker implements Worker {
  private readonly context: WorkerContext;
  private readonly configDir: string;
  private readonly executable: string;
  private readonly input = new AsyncQueue<SDKUserMessage>();
  private session: Query | null = null;
  private sessionId: string | null = null;
  private interrupting = false;
  private closing = false;
  private rejectedRateLimit: SDKRateLimitInfo | null = null;

  constructor(context: WorkerContext) {
    this.context = context;
    this.configDir = requireEnv(context.env, "CLAUDE_CONFIG_DIR");
    this.executable = requireEnv(context.env, "SANCHORIS_CLAUDE_PATH");
  }

  async start(restore: RestoredRecord | null): Promise<{ sessionId: string }> {
    const sessionId = restore?.sessionId ?? randomUUID();
    if (restore) {
      const dir = join(this.configDir, "projects", claudeProjectDirName(this.context.workspaceDir));
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, `${sessionId}.jsonl`), restore.bytes);
    }
    this.sessionId = sessionId;
    this.session = query({
      prompt: this.input,
      options: {
        cwd: this.context.workspaceDir,
        model: this.context.model,
        env: this.context.env,
        pathToClaudeCodeExecutable: this.executable,
        ...(restore ? { resume: sessionId } : { sessionId }),
        permissionMode: "bypassPermissions",
        allowDangerouslySkipPermissions: true,
        settingSources: ["user", "project", "local"],
        systemPrompt:
          this.context.systemPrompt === null
            ? { type: "preset", preset: "claude_code" }
            : { type: "preset", preset: "claude_code", append: this.context.systemPrompt },
        stderr: (data) => {
          for (const line of data.split("\n")) if (line !== "") this.context.handlers.onEvent("log", { line });
        },
      },
    });
    void this.consume(this.session);
    return { sessionId };
  }

  send(text: string): void {
    this.input.push({
      type: "user",
      message: { role: "user", content: text },
      parent_tool_use_id: null,
    });
  }

  async interrupt(): Promise<void> {
    if (!this.session) throw new Error("Claude session is not running");
    this.interrupting = true;
    await this.session.interrupt();
  }

  async readRecord(): Promise<WorkerRecord> {
    const sessionId = this.sessionId;
    if (!sessionId) throw new Error("Claude session is not running");
    const projects = join(this.configDir, "projects");
    const dirs = await readdir(projects);
    for (const dir of dirs) {
      const path = join(projects, dir, `${sessionId}.jsonl`);
      const bytes = await readFile(path).catch((error: NodeJS.ErrnoException) => {
        if (error.code === "ENOENT") return null;
        throw error;
      });
      if (bytes) return { relativePath: relative(this.configDir, path), bytes };
    }
    throw new Error(`Claude session record ${sessionId}.jsonl was not found under ${projects}`);
  }

  async close(): Promise<void> {
    this.closing = true;
    this.input.end();
    this.session?.close();
  }

  private async consume(session: Query): Promise<void> {
    try {
      for await (const message of session) this.handle(message);
      if (!this.closing) this.context.handlers.onFatal(new Error("Claude session ended unexpectedly"));
    } catch (error) {
      if (!this.closing) this.context.handlers.onFatal(error as Error);
    }
  }

  private handle(message: SDKMessage): void {
    const { onEvent, onRateLimit, onTurnEnd } = this.context.handlers;
    switch (message.type) {
      case "system":
        onEvent("system", { subtype: message.subtype });
        return;
      case "assistant":
        if (message.parent_tool_use_id !== null) return;
        for (const block of message.message.content) {
          if (block.type === "text") {
            onEvent("text", { text: block.text });
          } else if (block.type === "tool_use") {
            onEvent("tool_use", { id: block.id, name: block.name, input: block.input });
          }
        }
        return;
      case "user": {
        if (message.parent_tool_use_id !== null) return;
        const content = message.message.content;
        if (!Array.isArray(content)) return;
        for (const block of content) {
          if (block.type !== "tool_result") continue;
          onEvent("tool_result", {
            toolUseId: block.tool_use_id,
            isError: block.is_error ?? false,
            content: textOf(block.content).slice(0, MAX_TOOL_RESULT_CHARS),
          });
        }
        return;
      }
      case "rate_limit_event":
        if (message.rate_limit_info.status === "rejected") this.rejectedRateLimit = message.rate_limit_info;
        onRateLimit(claudeRateLimit(message.rate_limit_info));
        return;
      case "result":
        onTurnEnd(this.turnResult(message));
        return;
      default:
        return;
    }
  }

  private turnResult(message: SDKResultMessage): WorkerTurnResult {
    const failed = message.subtype !== "success" || message.is_error;
    const summary = message.subtype === "success" ? message.result : message.errors.join("\n");
    const rejected = this.rejectedRateLimit;
    const interrupted = this.interrupting;
    this.rejectedRateLimit = null;
    this.interrupting = false;
    let result: WorkerTurnResult["result"] = "completed";
    let error: WorkerTurnResult["error"] = null;
    if (interrupted) {
      result = "interrupted";
    } else if (failed) {
      result = "failed";
      error = rejected
        ? { code: "Worker.UsageLimit", message: summary, resetAt: rejected.resetsAt ?? null }
        : { code: "Worker.Error", message: summary, resetAt: null };
    }
    return {
      result,
      summary,
      error,
      usage: {
        inputTokens: message.usage.input_tokens,
        outputTokens: message.usage.output_tokens,
        cacheReadInputTokens: message.usage.cache_read_input_tokens,
        cacheCreationInputTokens: message.usage.cache_creation_input_tokens,
      },
      totalCostUsd: message.total_cost_usd,
      durationMs: message.duration_ms,
    };
  }
}
