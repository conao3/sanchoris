import { execFile } from "node:child_process";
import { randomUUID } from "node:crypto";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

import {
  emptyUsage,
  requireEnv,
  type RestoredRecord,
  type Worker,
  type WorkerContext,
  type WorkerFactory,
  type WorkerRecord,
} from "../worker.ts";

interface TranscriptLine {
  role: "user" | "assistant";
  text: string;
}

export interface CommandResult {
  code: number;
  stdout: string;
  stderr: string;
}

interface FakeTurnContext {
  history: readonly TranscriptLine[];
  exec: (command: string, args: string[]) => Promise<CommandResult>;
}

export type FakeScript = (text: string, context: FakeTurnContext) => Promise<string>;

class FakeWorker implements Worker {
  private readonly context: WorkerContext;
  private readonly script: FakeScript;
  private readonly home: string;
  private sessionId: string | null = null;
  private history: TranscriptLine[] = [];
  private chain: Promise<void> = Promise.resolve();
  private interruptTurn: (() => void) | null = null;

  constructor(context: WorkerContext, script: FakeScript) {
    this.context = context;
    this.script = script;
    this.home = requireEnv(context.env, "FAKE_WORKER_HOME");
  }

  async start(restore: RestoredRecord | null): Promise<{ sessionId: string }> {
    const sessionId = restore?.sessionId ?? randomUUID();
    this.sessionId = sessionId;
    await mkdir(join(this.home, "sessions"), { recursive: true });
    if (restore) {
      await writeFile(join(this.home, restore.relativePath), restore.bytes);
      this.history = restore.bytes
        .toString("utf8")
        .split("\n")
        .filter((line) => line !== "")
        .map((line) => JSON.parse(line) as TranscriptLine);
    } else {
      await writeFile(this.recordPath(), "");
    }
    return { sessionId };
  }

  send(text: string): void {
    this.chain = this.chain.then(() => this.turn(text)).catch((error: Error) => this.context.handlers.onFatal(error));
  }

  async interrupt(): Promise<void> {
    this.interruptTurn?.();
  }

  async readRecord(): Promise<WorkerRecord> {
    return { relativePath: this.relativePath(), bytes: await readFile(this.recordPath()) };
  }

  async close(): Promise<void> {
    await this.chain;
  }

  private relativePath(): string {
    if (!this.sessionId) throw new Error("Fake session is not running");
    return join("sessions", `${this.sessionId}.jsonl`);
  }

  private recordPath(): string {
    return join(this.home, this.relativePath());
  }

  private async append(line: TranscriptLine): Promise<void> {
    this.history.push(line);
    await appendFile(this.recordPath(), `${JSON.stringify(line)}\n`);
  }

  private async turn(text: string): Promise<void> {
    const started = Date.now();
    await this.append({ role: "user", text });
    const interrupted = new Promise<null>((resolve) => {
      this.interruptTurn = () => resolve(null);
    });
    const reply = await Promise.race([
      this.script(text, {
        history: this.history,
        exec: (command, args) => this.exec(command, args),
      }),
      interrupted,
    ]);
    this.interruptTurn = null;
    if (reply === null) {
      this.context.handlers.onTurnEnd({
        result: "interrupted",
        summary: "",
        error: null,
        usage: emptyUsage(),
        totalCostUsd: 0,
        durationMs: Date.now() - started,
      });
      return;
    }
    await this.append({ role: "assistant", text: reply });
    this.context.handlers.onEvent("text", { text: reply });
    this.context.handlers.onTurnEnd({
      result: "completed",
      summary: reply,
      error: null,
      usage: emptyUsage(),
      totalCostUsd: 0,
      durationMs: Date.now() - started,
    });
  }

  private exec(command: string, args: string[]): Promise<CommandResult> {
    this.context.handlers.onEvent("tool_use", { name: "Bash", input: { command: [command, ...args].join(" ") } });
    return new Promise((resolve) => {
      execFile(command, args, { env: this.context.env, cwd: this.context.workspaceDir }, (error, stdout, stderr) => {
        const code = error ? (typeof error.code === "number" ? error.code : 1) : 0;
        this.context.handlers.onEvent("tool_result", { isError: code !== 0, content: stdout + stderr });
        resolve({ code, stdout, stderr });
      });
    });
  }
}

export function fakeWorkerFactory(script: FakeScript): WorkerFactory {
  return (context) => new FakeWorker(context, script);
}
