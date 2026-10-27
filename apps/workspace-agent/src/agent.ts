import { unlink } from "node:fs/promises";
import type { Server } from "node:net";

import { RunChannel } from "./channel.ts";
import { runCommands } from "./exec.ts";
import type { Logger } from "./log.ts";
import type { ExecMessage, InboundMessage, SocketRequest, SocketResponse, StartMessage } from "./protocol.ts";
import { downloadRecord, sha256, uploadRecord } from "./session-record.ts";
import { validateSkillRun } from "./skills.ts";
import { listenSocket } from "./socket-server.ts";
import type { RestoredRecord, Worker, WorkerFactory, WorkerTurnResult } from "./worker.ts";

export interface AgentConfig {
  runId: string;
  token: string;
  controlUrl: string;
  socketPath: string;
  workspaceDir: string;
  env: NodeJS.ProcessEnv;
  createWorker: WorkerFactory;
  logger: Logger;
  initialBackoffMs: number;
  maxBackoffMs: number;
  requestTimeoutMs: number;
  drainTimeoutMs: number;
}

export class WorkspaceAgent {
  private readonly config: AgentConfig;
  private readonly channel: RunChannel;
  private server: Server | null = null;
  private start: StartMessage | null = null;
  private worker: Worker | null = null;
  private sessionId: string | null = null;
  private lastTurn = 0;
  private busy = false;
  private ready: Promise<void> | null = null;
  private turnChain: Promise<void> = Promise.resolve();
  private execChain: Promise<void> = Promise.resolve();
  private readonly seenMessageIds = new Set<string>();
  private readonly seenExecRequestIds = new Set<string>();
  private finished = false;
  private resolveExit: (code: number) => void = () => {};
  readonly exitCode: Promise<number>;

  constructor(config: AgentConfig) {
    this.config = config;
    this.exitCode = new Promise((resolve) => {
      this.resolveExit = resolve;
    });
    this.channel = new RunChannel({
      url: config.controlUrl,
      token: config.token,
      runId: config.runId,
      logger: config.logger,
      initialBackoffMs: config.initialBackoffMs,
      maxBackoffMs: config.maxBackoffMs,
      hello: () => ({ started: this.start !== null, sessionId: this.sessionId }),
      onCommand: (message) => this.handleCommand(message),
      onFatal: (error) => void this.fail(error),
    });
  }

  async run(): Promise<number> {
    this.server = await listenSocket(this.config.socketPath, (request) => this.handleSocket(request));
    this.channel.connect();
    return this.exitCode;
  }

  private handleCommand(message: Exclude<InboundMessage, { type: "ack" | "close" | "skill_response" | "record_upload_response" }>): void {
    switch (message.type) {
      case "start":
        if (this.start) {
          void this.fail(new Error("Received a second start message"));
          return;
        }
        this.start = message;
        this.ready = this.begin(message);
        this.ready.catch((error: Error) => this.fail(error));
        return;
      case "user_message":
        if (this.seenMessageIds.has(message.messageId)) return;
        this.seenMessageIds.add(message.messageId);
        void this.whenReady(() => this.deliver(message.text));
        return;
      case "interrupt":
        void this.whenReady(() => this.requireWorker().interrupt());
        return;
      case "exec":
        if (this.seenExecRequestIds.has(message.requestId)) return;
        this.seenExecRequestIds.add(message.requestId);
        this.exec(message);
        return;
      case "shutdown":
        this.config.logger("info", "shutdown requested", { reason: message.reason });
        void this.shutdown();
        return;
    }
  }

  private async whenReady(action: () => void | Promise<void>): Promise<void> {
    if (!this.ready) {
      await this.fail(new Error("Received a command before start"));
      return;
    }
    try {
      await this.ready;
      await action();
    } catch (error) {
      await this.fail(error as Error);
    }
  }

  private async begin(start: StartMessage): Promise<void> {
    let restored: RestoredRecord | null = null;
    if (start.resume) {
      const bytes = await downloadRecord(start.resume.url, start.resume.sha256);
      restored = { sessionId: start.resume.sessionId, relativePath: start.resume.relativePath, bytes };
      this.lastTurn = start.resume.lastTurn;
    }
    this.worker = this.config.createWorker({
      agent: start.agent,
      model: start.model,
      systemPrompt: start.systemPrompt,
      workspaceDir: this.config.workspaceDir,
      env: this.config.env,
      handlers: {
        onEvent: (kind, data) => {
          if (kind === "log") this.config.logger("info", "worker log", data);
          this.channel.send({ type: "event", turn: this.lastTurn + 1, kind, data, at: new Date().toISOString() });
        },
        onTurnEnd: (result) => this.endTurn(result),
        onRateLimit: (rateLimit) => {
          this.channel.send({ type: "rate_limit", ...rateLimit });
        },
        onFatal: (error) => void this.fail(error),
      },
    });
    const { sessionId } = await this.worker.start(restored);
    this.sessionId = sessionId;
    this.channel.send({ type: "ready", sessionId, agent: start.agent, model: start.model, resumed: restored !== null });
    this.channel.startHeartbeat(start.heartbeatSeconds, () => this.busy);
    if (start.instruction !== null) this.deliver(start.instruction);
  }

  private exec(message: ExecMessage): void {
    this.config.logger("info", "exec requested", { requestId: message.requestId, purpose: message.purpose });
    this.execChain = this.execChain
      .then(() => runCommands(message.commands, this.config.workspaceDir, this.config.env))
      .then((outcome) => {
        this.channel.send({ type: "exec_result", requestId: message.requestId, ...outcome });
      })
      .catch((error: Error) => this.fail(error));
  }

  private deliver(text: string): void {
    this.busy = true;
    this.requireWorker().send(text);
  }

  private endTurn(result: WorkerTurnResult): void {
    this.lastTurn += 1;
    const turn = this.lastTurn;
    this.busy = false;
    this.turnChain = this.turnChain
      .then(() => this.saveRecord(turn))
      .then(() => {
        this.channel.send({ type: "turn_end", turn, sessionId: this.requireSessionId(), ...result });
      })
      .catch((error: Error) => this.fail(error));
  }

  private async saveRecord(turn: number): Promise<void> {
    const sessionId = this.requireSessionId();
    const record = await this.requireWorker().readRecord();
    const digest = sha256(record.bytes);
    const reply = await this.channel.request(
      (requestId) => ({
        type: "record_upload_request",
        requestId,
        sessionId,
        turn,
        contentLength: record.bytes.length,
        sha256: digest,
      }),
      this.config.requestTimeoutMs,
    );
    if (reply.type !== "record_upload_response") throw new Error(`Unexpected reply ${reply.type}`);
    if (!reply.ok) throw new Error(`Control plane refused the record upload: ${reply.error.code} ${reply.error.message}`);
    await uploadRecord(reply.url, reply.headers, record.bytes);
    this.channel.send({
      type: "session_record",
      sessionId,
      turn,
      key: reply.key,
      relativePath: record.relativePath,
      contentLength: record.bytes.length,
      sha256: digest,
    });
  }

  private async handleSocket(request: SocketRequest): Promise<SocketResponse> {
    const start = this.start;
    if (!start || !this.ready) {
      return { ok: false, error: { code: "not_started", message: "The workspace agent has not started a session yet" } };
    }
    await this.ready;
    if (request.action === "run") {
      const invalid = validateSkillRun(start.skills, request);
      if (invalid) return { ok: false, error: invalid };
    }
    const reply = await this.channel.request((requestId) => {
      switch (request.action) {
        case "run":
          return {
            type: "skill_request",
            requestId,
            action: "run",
            name: request.name,
            input: request.input as Record<string, unknown>,
          };
        case "continue":
          return { type: "skill_request", requestId, action: "continue", skillRunId: request.skillRunId, output: request.output };
        case "status":
          return { type: "skill_request", requestId, action: "status", skillRunId: request.skillRunId };
      }
    }, this.config.requestTimeoutMs);
    if (reply.type !== "skill_response") throw new Error(`Unexpected reply ${reply.type}`);
    return reply.ok ? { ok: true, result: reply.result } : { ok: false, error: reply.error };
  }

  private requireWorker(): Worker {
    if (!this.worker) throw new Error("Worker is not running");
    return this.worker;
  }

  private requireSessionId(): string {
    if (!this.sessionId) throw new Error("Session is not ready");
    return this.sessionId;
  }

  private async shutdown(): Promise<void> {
    if (this.finished) return;
    try {
      await this.turnChain;
      await this.execChain;
      await this.worker?.close();
      await this.channel.drain(this.config.drainTimeoutMs);
    } catch (error) {
      await this.fail(error as Error);
      return;
    }
    await this.finish(0);
  }

  private async fail(error: Error): Promise<void> {
    if (this.finished) return;
    this.config.logger("error", "workspace agent failed", { error: error.message });
    await this.worker?.close().catch(() => {});
    await this.finish(1);
  }

  private async finish(code: number): Promise<void> {
    if (this.finished) return;
    this.finished = true;
    this.channel.close();
    if (this.server) {
      await new Promise<void>((resolve) => this.server?.close(() => resolve()));
      await unlink(this.config.socketPath).catch(() => {});
    }
    this.resolveExit(code);
  }
}
