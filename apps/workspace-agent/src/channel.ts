import { randomUUID } from "node:crypto";
import http from "node:http";
import https from "node:https";
import WebSocket from "ws";

import type { Logger } from "./log.ts";
import {
  type CredentialsMessage,
  type GitTokenResponseMessage,
  InboundMessage,
  type OutboundMessage,
  PROTOCOL_VERSION,
  type RecordUploadResponseMessage,
  type SequencedMessage,
  type SequencedPayload,
  type SkillResponseMessage,
  type WorkerCredentialResponseMessage,
} from "./protocol.ts";

type ReplyMessage =
  | SkillResponseMessage
  | RecordUploadResponseMessage
  | GitTokenResponseMessage
  | WorkerCredentialResponseMessage;
type InboundCommand = Exclude<InboundMessage, ReplyMessage | { type: "ack" | "close" | "credentials" }>;

interface PendingReply {
  resolve: (message: ReplyMessage) => void;
  reject: (error: Error) => void;
  timer: NodeJS.Timeout;
}

export interface RunChannelOptions {
  url: string;
  bootstrapToken: string;
  runId: string;
  env: NodeJS.ProcessEnv;
  logger: Logger;
  hello: () => { started: boolean; sessionId: string | null };
  onCommand: (message: InboundCommand) => void;
  onCredentials: (message: CredentialsMessage) => void;
  onFatal: (error: Error) => void;
  initialBackoffMs: number;
  maxBackoffMs: number;
}

class ProtocolError extends Error {}

const SEND_WINDOW = 1;
const PING_INTERVAL_MS = 30_000;

export class RunChannel {
  private readonly options: RunChannelOptions;
  private token: string;
  private credentialed = false;
  private socket: WebSocket | null = null;
  private seq = 0;
  private readonly instanceId = randomUUID();
  private unacked: SequencedMessage[] = [];
  private writtenThrough = 0;
  private helloAcked = false;
  private readonly replies = new Map<string, PendingReply>();
  private readonly expiredReplies = new Set<string>();
  private backoffMs: number;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private heartbeatTimer: NodeJS.Timeout | null = null;
  private pingTimer: NodeJS.Timeout | null = null;
  private closed = false;
  private readonly ackWaiters: Array<() => void> = [];

  constructor(options: RunChannelOptions) {
    this.options = options;
    this.token = options.bootstrapToken;
    this.backoffMs = options.initialBackoffMs;
  }

  connect(): void {
    if (this.closed) return;
    const agentOptions = { keepAlive: false, proxyEnv: this.options.env } as http.AgentOptions;
    const socket = new WebSocket(this.options.url, {
      headers: { Authorization: `Bearer ${this.token}` },
      agent: this.options.url.startsWith("wss:") ? new https.Agent(agentOptions) : new http.Agent(agentOptions),
    });
    this.socket = socket;
    socket.on("open", () => {
      this.backoffMs = this.options.initialBackoffMs;
      if (this.pingTimer) clearInterval(this.pingTimer);
      this.pingTimer = setInterval(() => socket.ping(), PING_INTERVAL_MS);
      const hello = this.options.hello();
      this.write({
        type: "hello",
        runId: this.options.runId,
        protocolVersion: PROTOCOL_VERSION,
        instanceId: this.instanceId,
        started: hello.started,
        sessionId: hello.sessionId,
      });
      this.writtenThrough = 0;
      this.helloAcked = false;
    });
    socket.on("message", (data) => {
      this.receive((data as Buffer).toString("utf8"));
    });
    socket.on("unexpected-response", (_request, response) => {
      const status = response.statusCode ?? 0;
      if (status === 401 || status === 403) {
        this.fail(new ProtocolError(`Run channel rejected the token with HTTP ${status}`));
        return;
      }
      this.options.logger("warn", "run channel upgrade failed", { status });
      socket.terminate();
    });
    socket.on("error", (error) => {
      this.options.logger("warn", "run channel error", { error: error.message });
    });
    socket.on("close", (code, reason) => {
      if (this.socket === socket) this.socket = null;
      if (this.socket === null && this.pingTimer) {
        clearInterval(this.pingTimer);
        this.pingTimer = null;
      }
      if (this.closed) return;
      if (code >= 4000 && code <= 4999) {
        this.fail(new ProtocolError(`Run channel closed by control plane: ${code} ${reason.toString()}`));
        return;
      }
      if (!this.credentialed) {
        this.fail(new ProtocolError("Run channel closed before the control plane sent the credentials"));
        return;
      }
      this.scheduleReconnect();
    });
  }

  send(payload: SequencedPayload): number {
    this.seq += 1;
    const message = { ...payload, runId: this.options.runId, seq: this.seq } as SequencedMessage;
    this.unacked.push(message);
    this.flush();
    return this.seq;
  }

  request(build: (requestId: string) => SequencedPayload, timeoutMs: number): Promise<ReplyMessage> {
    const requestId = randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.replies.delete(requestId);
        this.expiredReplies.add(requestId);
        reject(new Error(`No reply from the control plane within ${timeoutMs} ms`));
      }, timeoutMs);
      this.replies.set(requestId, { resolve, reject, timer });
      this.send(build(requestId));
    });
  }

  startHeartbeat(seconds: number, busy: () => boolean): void {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    const beat = () => {
      this.write({ type: "heartbeat", runId: this.options.runId, at: new Date().toISOString(), busy: busy() });
    };
    beat();
    this.heartbeatTimer = setInterval(beat, seconds * 1000);
  }

  async drain(timeoutMs: number): Promise<void> {
    if (this.unacked.length === 0) return;
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`${this.unacked.length} messages were not acknowledged within ${timeoutMs} ms`));
      }, timeoutMs);
      this.ackWaiters.push(() => {
        clearTimeout(timer);
        resolve();
      });
    });
  }

  close(): void {
    this.closed = true;
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    if (this.pingTimer) clearInterval(this.pingTimer);
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    for (const reply of this.replies.values()) {
      clearTimeout(reply.timer);
      reply.reject(new Error("Run channel closed"));
    }
    this.replies.clear();
    this.socket?.close(1000, "shutdown");
  }

  private flush(): void {
    if (this.socket?.readyState !== WebSocket.OPEN || !this.helloAcked) return;
    for (const message of this.unacked.slice(0, SEND_WINDOW)) {
      if (message.seq <= this.writtenThrough) continue;
      this.write(message);
      this.writtenThrough = message.seq;
    }
  }

  private write(message: OutboundMessage): void {
    if (this.socket?.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify(message));
  }

  private receive(raw: string): void {
    let parsed: InboundMessage;
    try {
      parsed = InboundMessage.parse(JSON.parse(raw));
    } catch (error) {
      this.fail(new ProtocolError(`Invalid inbound message: ${(error as Error).message}`));
      return;
    }
    if (parsed.runId !== this.options.runId) {
      this.fail(new ProtocolError(`Inbound message for run ${parsed.runId}, expected ${this.options.runId}`));
      return;
    }
    switch (parsed.type) {
      case "ack":
        this.helloAcked = true;
        this.unacked = this.unacked.filter((message) => message.seq > parsed.seq);
        this.flush();
        if (this.unacked.length === 0) for (const waiter of this.ackWaiters.splice(0)) waiter();
        return;
      case "close":
        this.fail(new ProtocolError(`Run channel closed by control plane: ${parsed.code} ${parsed.reason}`));
        return;
      case "credentials":
        if (this.credentialed) {
          this.fail(new ProtocolError("Received a second credentials message"));
          return;
        }
        this.credentialed = true;
        this.token = parsed.runToken;
        this.options.onCredentials(parsed);
        return;
      case "skill_response":
      case "record_upload_response":
      case "git_token_response":
      case "worker_credential_response": {
        const reply = this.replies.get(parsed.requestId);
        if (!reply) {
          if (this.expiredReplies.delete(parsed.requestId)) return;
          this.fail(new ProtocolError(`Reply for unknown request ${parsed.requestId}`));
          return;
        }
        this.replies.delete(parsed.requestId);
        clearTimeout(reply.timer);
        reply.resolve(parsed);
        return;
      }
      default:
        this.options.onCommand(parsed);
    }
  }

  private scheduleReconnect(): void {
    const delay = this.backoffMs;
    this.backoffMs = Math.min(this.backoffMs * 2, this.options.maxBackoffMs);
    this.options.logger("info", "run channel reconnecting", { delayMs: delay });
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.connect();
    }, delay);
  }

  private fail(error: Error): void {
    this.close();
    this.options.onFatal(error);
  }
}
