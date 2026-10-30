import { chmod } from "node:fs/promises"
import { createServer, type Server, type Socket } from "node:net"

import { SocketRequest, type SocketResponse } from "./protocol.ts"

const MAX_REQUEST_BYTES = 1024 * 1024

export type SocketHandler = (request: SocketRequest) => Promise<SocketResponse>

function respond(socket: Socket, response: SocketResponse): void {
  socket.end(`${JSON.stringify(response)}\n`)
}

export async function listenSocket(path: string, handler: SocketHandler): Promise<Server> {
  const server = createServer((socket) => {
    let buffer = ""
    let handled = false
    socket.setEncoding("utf8")
    socket.on("data", (chunk: string) => {
      if (handled) return
      buffer += chunk
      if (buffer.length > MAX_REQUEST_BYTES) {
        handled = true
        respond(socket, {
          ok: false,
          error: { code: "invalid_request", message: "Request is too large" },
        })
        return
      }
      const newline = buffer.indexOf("\n")
      if (newline === -1) return
      handled = true
      const line = buffer.slice(0, newline)
      let request: SocketRequest
      try {
        request = SocketRequest.parse(JSON.parse(line))
      } catch (error) {
        respond(socket, {
          ok: false,
          error: { code: "invalid_request", message: (error as Error).message },
        })
        return
      }
      handler(request).then(
        (response) => respond(socket, response),
        (error: Error) =>
          respond(socket, { ok: false, error: { code: "agent_error", message: error.message } }),
      )
    })
    socket.on("error", () => {})
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(path, () => {
      server.off("error", reject)
      resolve()
    })
  })
  await chmod(path, 0o600)
  return server
}
