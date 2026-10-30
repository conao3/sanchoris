import type { ChildProcessWithoutNullStreams } from "node:child_process"
import { createInterface } from "node:readline"

type JsonRpcId = number | string

interface JsonRpcError {
  code: number
  message: string
  data?: unknown
}

class CodexRpcError extends Error {
  readonly code: number
  readonly method: string

  constructor(method: string, error: JsonRpcError) {
    super(`${method} failed: ${error.message} (${error.code})`)
    this.code = error.code
    this.method = method
  }
}

export interface CodexRpcHandlers {
  onNotification: (method: string, params: unknown) => void
  onRequest: (method: string, params: unknown) => Promise<unknown>
  onProtocolError: (error: Error) => void
}

interface Pending {
  method: string
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null
}

function isId(value: unknown): value is JsonRpcId {
  return typeof value === "number" || typeof value === "string"
}

export class CodexAppServerClient {
  private readonly child: ChildProcessWithoutNullStreams
  private readonly handlers: CodexRpcHandlers
  private readonly pending = new Map<string, Pending>()
  private nextId = 1

  constructor(child: ChildProcessWithoutNullStreams, handlers: CodexRpcHandlers) {
    this.child = child
    this.handlers = handlers
    const lines = createInterface({ input: child.stdout })
    lines.on("line", (line) => this.receive(line))
  }

  request(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextId
    this.nextId += 1
    return new Promise((resolve, reject) => {
      this.pending.set(String(id), { method, resolve, reject })
      this.write({ id, method, ...(params === undefined ? {} : { params }) })
    })
  }

  notify(method: string, params?: unknown): void {
    this.write({ method, ...(params === undefined ? {} : { params }) })
  }

  rejectAll(error: Error): void {
    for (const pending of this.pending.values()) pending.reject(error)
    this.pending.clear()
  }

  private write(message: Record<string, unknown>): void {
    this.child.stdin.write(`${JSON.stringify(message)}\n`)
  }

  private receive(line: string): void {
    if (line.trim() === "") return
    let message: unknown
    try {
      message = JSON.parse(line)
    } catch {
      this.handlers.onProtocolError(
        new Error(`Codex app-server wrote a non-JSON line: ${line.slice(0, 200)}`),
      )
      return
    }
    if (!isObject(message)) {
      this.handlers.onProtocolError(new Error("Codex app-server wrote a non-object message"))
      return
    }
    const { id, method } = message
    if (typeof method === "string" && isId(id)) {
      this.handlers.onRequest(method, message.params).then(
        (result) => this.write({ id, result }),
        (error: Error) => this.write({ id, error: { code: -32601, message: error.message } }),
      )
      return
    }
    if (typeof method === "string") {
      this.handlers.onNotification(method, message.params)
      return
    }
    if (isId(id)) {
      const pending = this.pending.get(String(id))
      if (!pending) {
        this.handlers.onProtocolError(
          new Error(`Codex app-server replied to unknown request ${String(id)}`),
        )
        return
      }
      this.pending.delete(String(id))
      if (isObject(message.error)) {
        pending.reject(new CodexRpcError(pending.method, message.error as unknown as JsonRpcError))
      } else {
        pending.resolve(message.result)
      }
      return
    }
    this.handlers.onProtocolError(new Error("Codex app-server wrote an unrecognized message"))
  }
}
