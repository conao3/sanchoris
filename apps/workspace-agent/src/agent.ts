import { randomUUID } from "node:crypto"
import { unlink } from "node:fs/promises"
import type { Server } from "node:net"

import { downloadAttachments, workerMessage } from "./attachments.ts"
import { RunChannel } from "./channel.ts"
import { capture, checkpointRef, diff, repositories, restore } from "./checkpoint.ts"
import { type Checkout, checkout } from "./checkout.ts"
import {
  type CredentialProxy,
  providerCredential,
  startCredentialProxy,
} from "./credential-proxy.ts"
import { type EgressForwarder, type Relay, startEgressForwarder } from "./egress-forwarder.ts"
import { runCommands } from "./exec.ts"
import type { Logger } from "./log.ts"
import type {
  ApprovalDecision,
  ApprovalRequest,
  Attachment,
  CredentialsMessage,
  PartialKind,
  RewindMessage,
  SessionModes,
  ExecMessage,
  InboundMessage,
  SocketRequest,
  SocketResponse,
  StartMessage,
  UserInputAnswers,
  UserInputQuestion,
} from "./protocol.ts"
import { downloadRecord, redactTokens, sha256, uploadRecord } from "./session-record.ts"
import { validateSkillRun } from "./skills.ts"
import { listenSocket } from "./socket-server.ts"
import type {
  RestoredRecord,
  Worker,
  WorkerContext,
  WorkerFactory,
  WorkerTurnResult,
} from "./worker.ts"

export type CredentialSource = { kind: "environment" } | ({ kind: "channel" } & Checkout)

const GIT_TOKEN_MARGIN_MS = 5 * 60_000
const PARTIAL_INTERVAL_MS = 500
const MAX_PARTIAL_CHARS = 16_000

export interface AgentConfig {
  runId: string
  bootstrapToken: string
  controlUrl: string
  socketPath: string
  workspaceDir: string
  credentialSource: CredentialSource
  egress: { relay: Relay; port: number } | null
  providerUpstream: string
  env: NodeJS.ProcessEnv
  createWorker: WorkerFactory
  logger: Logger
  initialBackoffMs: number
  maxBackoffMs: number
  requestTimeoutMs: number
  drainTimeoutMs: number
  stopTurnTimeoutMs: number
}

export class WorkspaceAgent {
  private readonly config: AgentConfig
  private readonly channel: RunChannel
  private server: Server | null = null
  private forwarder: EgressForwarder | null = null
  private credentialProxy: CredentialProxy | null = null
  private start: StartMessage | null = null
  private worker: Worker | null = null
  private sessionId: string | null = null
  private lastTurn = 0
  private busy = false
  private baseline: Promise<{ dirs: string[]; ref: string }> | null = null
  private ready: Promise<void> | null = null
  private turnChain: Promise<void> = Promise.resolve()
  private execChain: Promise<void> = Promise.resolve()
  private readonly seenMessageIds = new Set<string>()
  private readonly seenExecRequestIds = new Set<string>()
  private readonly questions = new Map<string, (answers: UserInputAnswers) => void>()
  private readonly approvals = new Map<string, (decision: ApprovalDecision) => void>()
  private deliveries: Promise<void> = Promise.resolve()
  private workerContext: WorkerContext | null = null
  private modes: SessionModes | null = null
  private partial: { kind: PartialKind; text: string } | null = null
  private partialTimer: NodeJS.Timeout | null = null
  private partialSentAt = 0
  private finished = false
  private stopping = false
  private turnEnded: (() => void) | null = null
  private gitToken: { token: string; expiresAt: number } | null = null
  private resolveExit: (code: number) => void = () => {}
  readonly exitCode: Promise<number>

  constructor(config: AgentConfig) {
    this.config = config
    this.exitCode = new Promise((resolve) => {
      this.resolveExit = resolve
    })
    this.channel = new RunChannel({
      url: config.controlUrl,
      bootstrapToken: config.bootstrapToken,
      runId: config.runId,
      env: config.env,
      logger: config.logger,
      initialBackoffMs: config.initialBackoffMs,
      maxBackoffMs: config.maxBackoffMs,
      hello: () => ({ started: this.start !== null, sessionId: this.sessionId }),
      onCommand: (message) => this.handleCommand(message),
      onCredentials: (message) => this.useCredentials(message),
      onFatal: (error) => void this.fail(error),
    })
  }

  async run(): Promise<number> {
    const egress = this.config.egress
    if (egress) {
      this.forwarder = await startEgressForwarder(egress.relay, egress.port, (error) =>
        this.config.logger("warn", "relay connection failed", { error: error.message }),
      )
    }
    this.server = await listenSocket(this.config.socketPath, (request) =>
      this.handleSocket(request),
    )
    this.channel.connect()
    return this.exitCode
  }

  private handleCommand(
    message: Exclude<
      InboundMessage,
      {
        type:
          | "ack"
          | "close"
          | "credentials"
          | "skill_response"
          | "record_upload_response"
          | "git_token_response"
          | "worker_credential_response"
      }
    >,
  ): void {
    switch (message.type) {
      case "start":
        if (this.start) {
          void this.fail(new Error("Received a second start message"))
          return
        }
        this.start = message
        this.ready = this.begin(message)
        this.ready.catch((error: Error) => this.fail(error))
        return
      case "user_message":
        if (this.stopping) {
          this.config.logger("info", "message ignored while stopping", {
            messageId: message.messageId,
          })
          return
        }
        if (this.seenMessageIds.has(message.messageId)) return
        this.seenMessageIds.add(message.messageId)
        this.deliveries = this.deliveries.then(() =>
          this.whenReady(() => this.deliver(message.text, message.steer, message.attachments)),
        )
        return
      case "session_mode":
        this.deliveries = this.deliveries.then(() =>
          this.whenReady(() => {
            this.modes = message.modes
            return this.requireWorker().setModes(message.modes)
          }),
        )
        return
      case "rewind":
        this.deliveries = this.deliveries.then(() => this.whenReady(() => this.rewind(message)))
        return
      case "interrupt":
        void this.whenReady(() => this.requireWorker().interrupt())
        return
      case "user_input_response": {
        const resolve = this.questions.get(message.requestId)
        if (!resolve) {
          this.config.logger("info", "answer ignored for a question that is not open", {
            requestId: message.requestId,
          })
          return
        }
        this.questions.delete(message.requestId)
        resolve(message.answers)
        return
      }
      case "approval_response": {
        const resolve = this.approvals.get(message.requestId)
        if (!resolve) {
          this.config.logger("info", "decision ignored for an approval that is not open", {
            requestId: message.requestId,
          })
          return
        }
        this.approvals.delete(message.requestId)
        resolve(message.decision)
        return
      }
      case "exec":
        if (this.stopping) {
          this.config.logger("info", "command ignored while stopping", {
            requestId: message.requestId,
          })
          return
        }
        if (this.seenExecRequestIds.has(message.requestId)) return
        this.seenExecRequestIds.add(message.requestId)
        this.exec(message)
        return
      case "shutdown":
        this.config.logger("info", "shutdown requested", { reason: message.reason })
        void this.shutdown()
        return
    }
  }

  private useCredentials(message: CredentialsMessage): void {
    if (message.relayToken === null) {
      if (this.forwarder)
        void this.fail(new Error("The credentials carry no relay token for the egress relay"))
      return
    }
    if (!this.forwarder) {
      void this.fail(
        new Error("The credentials carry a relay token, but this workspace has no egress relay"),
      )
      return
    }
    this.forwarder.usePassword(message.relayToken)
  }

  private async whenReady(action: () => void | Promise<void>): Promise<void> {
    if (!this.ready) {
      await this.fail(new Error("Received a command before start"))
      return
    }
    try {
      await this.ready
      await action()
    } catch (error) {
      await this.fail(error as Error)
    }
  }

  private async begin(start: StartMessage): Promise<void> {
    let env = this.config.env
    const source = this.config.credentialSource
    if (source.kind === "channel") {
      this.credentialProxy = await startCredentialProxy({
        upstream: this.config.providerUpstream,
        credential: providerCredential(await this.workerCredential()),
        proxyEnv: this.config.env,
        onError: (error) =>
          this.config.logger("warn", "provider request failed", { error: error.message }),
      })
      env = { ...env, ...this.credentialProxy.workerEnv }
      await checkout(source, this.config.workspaceDir, this.config.env)
      this.config.logger("info", "repositories checked out", {
        repositories: source.repositories,
        branch: source.branch,
      })
    }
    let restored: RestoredRecord | null = null
    if (start.resume) {
      const bytes = await downloadRecord(start.resume.url, start.resume.sha256)
      restored = {
        sessionId: start.resume.sessionId,
        relativePath: start.resume.relativePath,
        bytes,
      }
      this.lastTurn = start.resume.lastTurn
    }
    this.modes = start.modes
    this.workerContext = {
      agent: start.agent,
      model: start.model,
      systemPrompt: start.systemPrompt,
      modes: start.modes,
      workspaceDir: this.config.workspaceDir,
      env,
      handlers: {
        onEvent: (kind, data) => {
          if (kind === "log") this.config.logger("info", "worker log", data)
          else this.busy = true
          if (kind === "text" || kind === "thinking") this.clearPartial()
          this.channel.send({
            type: "event",
            turn: this.lastTurn + 1,
            kind,
            data,
            at: new Date().toISOString(),
          })
        },
        onPartial: (kind, text) => this.showPartial(kind, text),
        onTurnEnd: (result) => this.endTurn(result),
        onRateLimit: (rateLimit) => {
          this.channel.send({ type: "rate_limit", ...rateLimit })
        },
        onUserInput: (questions, signal) => this.ask(questions, signal),
        onApproval: (request, signal) => this.approve(request, signal),
        onFatal: (error) => void this.fail(error),
      },
    }
    this.worker = this.config.createWorker(this.workerContext)
    const { sessionId } = await this.worker.start(restored)
    this.sessionId = sessionId
    this.channel.send({
      type: "ready",
      sessionId,
      agent: start.agent,
      model: start.model,
      resumed: restored !== null,
    })
    this.channel.startHeartbeat(start.heartbeatSeconds, () => this.busy)
    if (start.instruction !== null) await this.deliver(start.instruction, false, start.attachments)
  }

  private ask(questions: UserInputQuestion[], signal: AbortSignal): Promise<UserInputAnswers> {
    const requestId = randomUUID()
    return new Promise((resolve, reject) => {
      const cancel = () => {
        this.questions.delete(requestId)
        reject(new Error("The question was cancelled before anyone answered it."))
      }
      if (signal.aborted) {
        cancel()
        return
      }
      signal.addEventListener("abort", cancel, { once: true })
      this.questions.set(requestId, (answers) => {
        signal.removeEventListener("abort", cancel)
        resolve(answers)
      })
      this.channel.send({
        type: "user_input_request",
        requestId,
        turn: this.lastTurn + 1,
        questions,
      })
    })
  }

  private approve(request: ApprovalRequest, signal: AbortSignal): Promise<ApprovalDecision> {
    const requestId = randomUUID()
    return new Promise((resolve, reject) => {
      const cancel = () => {
        this.approvals.delete(requestId)
        reject(new Error("The approval was cancelled before anyone decided."))
      }
      if (signal.aborted) {
        cancel()
        return
      }
      signal.addEventListener("abort", cancel, { once: true })
      this.approvals.set(requestId, (decision) => {
        signal.removeEventListener("abort", cancel)
        resolve(decision)
      })
      this.channel.send({
        type: "approval_request",
        requestId,
        turn: this.lastTurn + 1,
        ...request,
      })
    })
  }

  private async workerCredential(): Promise<Record<string, string>> {
    const reply = await this.channel.request(
      (requestId) => ({ type: "worker_credential_request", requestId }),
      this.config.requestTimeoutMs,
    )
    if (reply.type !== "worker_credential_response")
      throw new Error(`Unexpected reply ${reply.type}`)
    if (!reply.ok)
      throw new Error(
        `Control plane refused the worker credential: ${reply.error.code} ${reply.error.message}`,
      )
    return reply.env
  }

  private async gitCredential(host: string): Promise<SocketResponse> {
    const source = this.config.credentialSource
    if (source.kind !== "channel") {
      return {
        ok: false,
        error: {
          code: "invalid_request",
          message: "This workspace takes its credentials from the environment",
        },
      }
    }
    if (host !== new URL(source.gitUrl).host) {
      return { ok: false, error: { code: "invalid_request", message: `No credential for ${host}` } }
    }
    if (!this.gitToken || this.gitToken.expiresAt - GIT_TOKEN_MARGIN_MS <= Date.now()) {
      const reply = await this.channel.request(
        (requestId) => ({ type: "git_token_request", requestId }),
        this.config.requestTimeoutMs,
      )
      if (reply.type !== "git_token_response") throw new Error(`Unexpected reply ${reply.type}`)
      if (!reply.ok) return { ok: false, error: reply.error }
      this.gitToken = { token: reply.token, expiresAt: Date.parse(reply.expiresAt) }
    }
    return { ok: true, result: { username: "x-access-token", password: this.gitToken.token } }
  }

  private exec(message: ExecMessage): void {
    this.config.logger("info", "exec requested", {
      requestId: message.requestId,
      purpose: message.purpose,
    })
    this.execChain = this.execChain
      .then(() => this.ready)
      .then(() => runCommands(message.commands, this.config.workspaceDir, this.config.env))
      .then((outcome) => {
        this.channel.send({ type: "exec_result", requestId: message.requestId, ...outcome })
      })
      .catch((error: Error) => this.fail(error))
  }

  private async deliver(
    text: string,
    steer: boolean,
    attachments: readonly Attachment[],
  ): Promise<void> {
    const images = await downloadAttachments(attachments)
    if (!this.busy) {
      this.busy = true
      this.baseline = this.checkpoint(checkpointRef(this.lastTurn + 1, "start"))
    }
    await this.baseline
    this.requireWorker().send(workerMessage(text, steer, images))
  }

  private async rewind(message: RewindMessage): Promise<void> {
    if (this.busy) {
      this.config.logger("info", "rewind ignored while a turn runs", {})
      return
    }
    const context = this.workerContext
    if (!context || !this.modes) throw new Error("Worker is not running")
    if (message.restoreTurn !== null) {
      const dirs = await repositories(this.config.workspaceDir, this.config.env)
      await restore(dirs, checkpointRef(message.restoreTurn, "start"), this.config.env)
    }
    const restored: RestoredRecord | null = message.resume
      ? {
          sessionId: message.resume.sessionId,
          relativePath: message.resume.relativePath,
          bytes: await downloadRecord(message.resume.url, message.resume.sha256),
        }
      : null
    await this.requireWorker().close()
    this.baseline = null
    this.worker = this.config.createWorker({ ...context, modes: this.modes })
    const { sessionId } = await this.worker.start(restored)
    this.sessionId = sessionId
    this.config.logger("info", "session rewound", { sessionId, restoreTurn: message.restoreTurn })
  }

  private async checkpoint(ref: string): Promise<{ dirs: string[]; ref: string }> {
    const dirs = await repositories(this.config.workspaceDir, this.config.env)
    await capture(dirs, ref, this.config.env)
    return { dirs, ref }
  }

  private async endCheckpoint(
    started: Promise<{ dirs: string[]; ref: string }>,
    turn: number,
  ): Promise<{ dirs: string[]; ref: string }> {
    const { dirs } = await started
    const ref = checkpointRef(turn, "end")
    await capture(dirs, ref, this.config.env)
    return { dirs, ref }
  }

  private async sendDiff(
    turn: number,
    started: Promise<{ dirs: string[]; ref: string }> | null,
    ended: Promise<{ dirs: string[]; ref: string }> | null,
  ): Promise<void> {
    if (!started || !ended) return
    const from = await started
    const to = await ended
    const changes = await diff(from.dirs, from.ref, to.ref, this.config.env)
    if (changes.files.length === 0) return
    this.channel.send({
      type: "event",
      turn,
      kind: "turn_diff",
      data: { ...changes },
      at: new Date().toISOString(),
    })
  }

  private showPartial(kind: PartialKind, text: string): void {
    this.busy = true
    this.partial = { kind, text }
    if (this.partialTimer) return
    const wait = Math.max(0, this.partialSentAt + PARTIAL_INTERVAL_MS - Date.now())
    this.partialTimer = setTimeout(() => this.flushPartial(), wait)
  }

  private flushPartial(): void {
    this.partialTimer = null
    const partial = this.partial
    if (!partial) return
    this.partialSentAt = Date.now()
    this.channel.post({
      type: "partial",
      runId: this.config.runId,
      turn: this.lastTurn + 1,
      kind: partial.kind,
      text: partial.text.slice(0, MAX_PARTIAL_CHARS),
      at: new Date().toISOString(),
    })
  }

  private clearPartial(): void {
    if (this.partialTimer) clearTimeout(this.partialTimer)
    this.partialTimer = null
    this.partial = null
  }

  private endTurn(result: WorkerTurnResult): void {
    this.clearPartial()
    this.lastTurn += 1
    const turn = this.lastTurn
    this.busy = false
    const started = this.baseline
    const ended = started ? this.endCheckpoint(started, turn) : null
    ended?.catch(() => {})
    this.baseline = ended
    this.turnChain = this.turnChain
      .then(() => this.sendDiff(turn, started, ended))
      .then(() => this.saveRecord(turn))
      .then(() => {
        this.channel.send({
          type: "turn_end",
          turn,
          sessionId: this.requireSessionId(),
          ...result,
          busy: this.busy,
        })
      })
      .catch((error: Error) => this.fail(error))
    this.turnEnded?.()
    this.turnEnded = null
  }

  async stop(signal: "SIGTERM"): Promise<void> {
    if (this.finished || this.stopping) return
    this.stopping = true
    this.config.logger("info", "stopping on signal", { signal, busy: this.busy })
    this.channel.send({ type: "stopping", signal })
    try {
      await this.ready
      if (this.busy && this.worker) {
        const ended = new Promise<void>((resolve) => {
          this.turnEnded = resolve
        })
        await this.worker.interrupt()
        let timer: NodeJS.Timeout | undefined
        const timedOut = new Promise<"timeout">((resolve) => {
          timer = setTimeout(() => resolve("timeout"), this.config.stopTurnTimeoutMs)
        })
        const outcome = await Promise.race([ended.then(() => "ended" as const), timedOut])
        clearTimeout(timer)
        if (outcome === "timeout") {
          throw new Error(
            `The interrupted turn did not end within ${this.config.stopTurnTimeoutMs} ms`,
          )
        }
      }
    } catch (error) {
      await this.fail(error as Error)
      return
    }
    await this.shutdown(false)
  }

  private async saveRecord(turn: number): Promise<void> {
    const sessionId = this.requireSessionId()
    const read = await this.requireWorker().readRecord()
    const record = { ...read, bytes: redactTokens(read.bytes) }
    const digest = sha256(record.bytes)
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
    )
    if (reply.type !== "record_upload_response") throw new Error(`Unexpected reply ${reply.type}`)
    if (!reply.ok)
      throw new Error(
        `Control plane refused the record upload: ${reply.error.code} ${reply.error.message}`,
      )
    await uploadRecord(reply.url, reply.headers, record.bytes)
    this.channel.send({
      type: "session_record",
      sessionId,
      turn,
      key: reply.key,
      relativePath: record.relativePath,
      contentLength: record.bytes.length,
      sha256: digest,
    })
  }

  private async handleSocket(request: SocketRequest): Promise<SocketResponse> {
    if (request.action === "git_credential") return this.gitCredential(request.host)
    const start = this.start
    if (!start || !this.ready) {
      return {
        ok: false,
        error: {
          code: "not_started",
          message: "The workspace agent has not started a session yet",
        },
      }
    }
    await this.ready
    if (request.action === "run") {
      const invalid = validateSkillRun(start.skills, request)
      if (invalid) return { ok: false, error: invalid }
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
          }
        case "continue":
          return {
            type: "skill_request",
            requestId,
            action: "continue",
            skillRunId: request.skillRunId,
            output: request.output,
          }
        case "status":
          return {
            type: "skill_request",
            requestId,
            action: "status",
            skillRunId: request.skillRunId,
          }
      }
    }, this.config.requestTimeoutMs)
    if (reply.type !== "skill_response") throw new Error(`Unexpected reply ${reply.type}`)
    return reply.ok ? { ok: true, result: reply.result } : { ok: false, error: reply.error }
  }

  private requireWorker(): Worker {
    if (!this.worker) throw new Error("Worker is not running")
    return this.worker
  }

  private requireSessionId(): string {
    if (!this.sessionId) throw new Error("Session is not ready")
    return this.sessionId
  }

  private async shutdown(waitForCommands = true): Promise<void> {
    if (this.finished) return
    try {
      await this.turnChain
      if (waitForCommands) await this.execChain
      await this.worker?.close()
      await this.channel.drain(this.config.drainTimeoutMs)
    } catch (error) {
      await this.fail(error as Error)
      return
    }
    await this.finish(0)
  }

  private async fail(error: Error): Promise<void> {
    if (this.finished) return
    this.config.logger("error", "workspace agent failed", { error: error.message })
    await this.worker?.close().catch(() => {})
    await this.finish(1)
  }

  private async finish(code: number): Promise<void> {
    if (this.finished) return
    this.finished = true
    this.channel.close()
    if (this.server) {
      await new Promise<void>((resolve) => this.server?.close(() => resolve()))
      await unlink(this.config.socketPath).catch(() => {})
    }
    await this.credentialProxy?.close()
    await this.forwarder?.close()
    this.resolveExit(code)
  }
}
