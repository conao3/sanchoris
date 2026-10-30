import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http"
import type { AddressInfo } from "node:net"
import { WebSocketServer, type WebSocket } from "ws"

import { OutboundMessage, type InboundMessage } from "../protocol.ts"

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never
type SkillRequest = Extract<OutboundMessage, { type: "skill_request" }>
type SkillReply =
  | { ok: true; result: Record<string, unknown> }
  | { ok: false; error: { code: string; message: string } }

export type ControlMessage = DistributiveOmit<InboundMessage, "runId">

interface Waiter {
  predicate: (message: OutboundMessage) => boolean
  resolve: (message: OutboundMessage) => void
}

export interface FakeControlOptions {
  runId: string
  bootstrapTokens: string[]
  runToken: string
  relayToken?: string | null
  deliverCredentials?: boolean
  onSkillRequest: (request: SkillRequest) => SkillReply
}

async function readBody(request: IncomingMessage): Promise<Buffer> {
  const chunks: Buffer[] = []
  for await (const chunk of request) chunks.push(chunk as Buffer)
  return Buffer.concat(chunks)
}

export class FakeControl {
  readonly received: OutboundMessage[] = []
  readonly records = new Map<string, Buffer>()
  onSkillRequest: (request: SkillRequest) => SkillReply
  private readonly options: FakeControlOptions
  private readonly http: Server
  private readonly wss = new WebSocketServer({ noServer: true })
  private socket: WebSocket | null = null
  private readonly spent = new Set<string>()
  readonly authorizations: string[] = []
  private instanceId: string | null = null
  private lastSeq = 0
  private waiters: Waiter[] = []
  private connectionWaiters: Array<() => void> = []
  connections = 0

  private constructor(options: FakeControlOptions) {
    this.options = options
    this.onSkillRequest = options.onSkillRequest
    this.http = createServer((request, response) => {
      void this.serveRecord(request, response)
    })
    this.http.on("upgrade", (request, socket, head) => {
      const token = request.headers.authorization?.replace(/^Bearer /, "") ?? ""
      const bootstrap = options.bootstrapTokens.includes(token)
      if (!bootstrap && token !== options.runToken) {
        socket.end("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
        return
      }
      if (this.spent.has(token)) {
        socket.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
        return
      }
      if (bootstrap) this.spent.add(token)
      this.authorizations.push(token)
      this.wss.handleUpgrade(request, socket, head, (ws) => this.attach(ws, bootstrap))
    })
  }

  static async listen(options: FakeControlOptions): Promise<FakeControl> {
    const control = new FakeControl(options)
    await new Promise<void>((resolve) => control.http.listen(0, "127.0.0.1", resolve))
    return control
  }

  get port(): number {
    return (this.http.address() as AddressInfo).port
  }

  get url(): string {
    return `ws://127.0.0.1:${this.port}/runs/${this.options.runId}/channel`
  }

  recordUrl(key: string): string {
    return `http://127.0.0.1:${this.port}/records/${key}`
  }

  send(message: ControlMessage): void {
    if (!this.socket) throw new Error("No agent is connected")
    this.socket.send(JSON.stringify({ ...message, runId: this.options.runId }))
  }

  sendRaw(raw: string): void {
    if (!this.socket) throw new Error("No agent is connected")
    this.socket.send(raw)
  }

  dropConnection(): void {
    this.socket?.terminate()
    this.socket = null
  }

  waitForConnection(count: number, timeoutMs = 10_000): Promise<void> {
    if (this.connections >= count) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`Timed out waiting for connection ${count}`)),
        timeoutMs,
      )
      this.connectionWaiters.push(() => {
        if (this.connections >= count) {
          clearTimeout(timer)
          resolve()
        }
      })
    })
  }

  waitFor<T extends OutboundMessage["type"]>(
    type: T,
    predicate: (message: Extract<OutboundMessage, { type: T }>) => boolean = () => true,
    timeoutMs = 10_000,
  ): Promise<Extract<OutboundMessage, { type: T }>> {
    const matches = (message: OutboundMessage): boolean =>
      message.type === type && predicate(message as Extract<OutboundMessage, { type: T }>)
    const existing = this.received.find(matches)
    if (existing) return Promise.resolve(existing as Extract<OutboundMessage, { type: T }>)
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((waiter) => waiter.resolve !== done)
        reject(new Error(`Timed out waiting for ${type}`))
      }, timeoutMs)
      const done = (message: OutboundMessage) => {
        clearTimeout(timer)
        resolve(message as Extract<OutboundMessage, { type: T }>)
      }
      this.waiters.push({ predicate: matches, resolve: done })
    })
  }

  async close(): Promise<void> {
    for (const client of this.wss.clients) client.terminate()
    this.wss.close()
    this.http.closeAllConnections()
    await new Promise<void>((resolve) => this.http.close(() => resolve()))
  }

  private attach(ws: WebSocket, bootstrap: boolean): void {
    this.socket = ws
    this.connections += 1
    for (const waiter of this.connectionWaiters) waiter()
    ws.on("message", (data) => {
      const message = OutboundMessage.parse(JSON.parse((data as Buffer).toString("utf8")))
      if (message.runId !== this.options.runId) throw new Error(`Unexpected run ${message.runId}`)
      if (message.type === "hello") {
        if (message.instanceId !== this.instanceId) {
          this.instanceId = message.instanceId
          this.lastSeq = 0
        }
        ws.send(JSON.stringify({ type: "ack", runId: this.options.runId, seq: this.lastSeq }))
        if (bootstrap && this.options.deliverCredentials !== false) {
          ws.send(
            JSON.stringify({
              type: "credentials",
              runId: this.options.runId,
              runToken: this.options.runToken,
              relayToken: this.options.relayToken ?? null,
            }),
          )
        }
      }
      if ("seq" in message) {
        if (message.seq <= this.lastSeq) {
          ws.send(JSON.stringify({ type: "ack", runId: this.options.runId, seq: this.lastSeq }))
          return
        }
        this.lastSeq = message.seq
        ws.send(JSON.stringify({ type: "ack", runId: this.options.runId, seq: message.seq }))
      }
      this.record(message)
      this.reply(message)
    })
  }

  private record(message: OutboundMessage): void {
    this.received.push(message)
    const remaining: Waiter[] = []
    for (const waiter of this.waiters) {
      if (waiter.predicate(message)) waiter.resolve(message)
      else remaining.push(waiter)
    }
    this.waiters = remaining
  }

  private reply(message: OutboundMessage): void {
    if (message.type === "skill_request") {
      this.send({
        type: "skill_response",
        requestId: message.requestId,
        ...this.onSkillRequest(message),
      })
    } else if (message.type === "record_upload_request") {
      const key = `runs/${this.options.runId}/${message.sessionId}/turn-${message.turn}.jsonl`
      this.send({
        type: "record_upload_response",
        requestId: message.requestId,
        ok: true,
        url: this.recordUrl(key),
        headers: { "content-type": "application/octet-stream" },
        key,
      })
    }
  }

  private async serveRecord(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const prefix = "/records/"
    const key = request.url?.startsWith(prefix)
      ? decodeURIComponent(request.url.slice(prefix.length))
      : null
    if (key === null) {
      response.writeHead(404).end()
      return
    }
    if (request.method === "PUT") {
      this.records.set(key, await readBody(request))
      response.writeHead(200).end()
      return
    }
    const record = this.records.get(key)
    if (request.method === "GET" && record) {
      response.writeHead(200, { "content-type": "application/octet-stream" }).end(record)
      return
    }
    response.writeHead(404).end()
  }
}
