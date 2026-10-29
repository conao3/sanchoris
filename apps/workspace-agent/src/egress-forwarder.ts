import { createServer, type IncomingMessage, type Server } from "node:http";
import { connect, type Socket } from "node:net";

const HEAD_LIMIT = 16 * 1024;

export interface EgressForwarder {
  server: Server;
  usePassword: (password: string) => void;
  close: () => Promise<void>;
}

export interface Relay {
  host: string;
  port: number;
  username: string;
  password: string;
}

export function parseRelayUrl(value: string): Relay {
  const url = new URL(value);
  if (url.protocol !== "http:" || url.username === "" || url.password === "" || url.port === "") {
    throw new Error("The relay URL must be http://<user>:<token>@<host>:<port>");
  }
  return {
    host: url.hostname.replace(/^\[|\]$/g, ""),
    port: Number(url.port),
    username: decodeURIComponent(url.username),
    password: decodeURIComponent(url.password),
  };
}

export function loopbackProxyPort(value: string | undefined): number {
  const match = /^http:\/\/127\.0\.0\.1:(\d{1,5})\/?$/.exec(value ?? "");
  const port = Number(match?.[1]);
  if (!match || port <= 0 || port > 65535) throw new Error("HTTPS_PROXY must be http://127.0.0.1:<port> when the agent forwards to the relay");
  return port;
}

function refuse(socket: Socket, status: string): void {
  socket.end(`HTTP/1.1 ${status}\r\nContent-Length: 0\r\n\r\n`);
}

function forward(relay: Relay, request: IncomingMessage, client: Socket, head: Buffer, sockets: Set<Socket>, onError: (error: Error) => void): void {
  sockets.add(client);
  client.on("close", () => sockets.delete(client));
  client.on("error", () => client.destroy());
  const target = request.url ?? "";
  if (!/^[^\s/]+:\d{1,5}$/.test(target)) {
    refuse(client, "400 Bad Request");
    return;
  }
  const upstream = connect(relay.port, relay.host);
  sockets.add(upstream);
  upstream.on("close", () => {
    sockets.delete(upstream);
    client.destroy();
  });
  client.on("close", () => upstream.destroy());
  let buffered = Buffer.alloc(0);
  let established = false;
  upstream.on("connect", () => {
    upstream.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\nProxy-Authorization: Basic ${Buffer.from(`${relay.username}:${relay.password}`).toString("base64")}\r\n\r\n`);
  });
  const onData = (chunk: Buffer) => {
    buffered = Buffer.concat([buffered, chunk]);
    const end = buffered.indexOf("\r\n\r\n");
    if (end < 0) {
      if (buffered.length > HEAD_LIMIT) upstream.destroy();
      return;
    }
    upstream.off("data", onData);
    const responseHead = buffered.subarray(0, end + 4);
    const rest = buffered.subarray(end + 4);
    client.write(responseHead);
    if (!responseHead.toString("latin1").startsWith("HTTP/1.1 200")) {
      client.end();
      return;
    }
    established = true;
    if (rest.length > 0) client.write(rest);
    if (head.length > 0) upstream.write(head);
    upstream.pipe(client);
    client.pipe(upstream);
  };
  upstream.on("data", onData);
  upstream.on("error", (error) => {
    onError(error);
    if (!established && !client.destroyed && client.writable) refuse(client, "502 Bad Gateway");
    else client.destroy();
  });
}

export async function startEgressForwarder(initial: Relay, port: number, onError: (error: Error) => void): Promise<EgressForwarder> {
  let relay = initial;
  const sockets = new Set<Socket>();
  const server = createServer((_request, response) => {
    response.writeHead(405, { Allow: "CONNECT", "Content-Length": "0" });
    response.end();
  });
  server.on("connect", (request: IncomingMessage, client: Socket, head: Buffer) => forward(relay, request, client, head, sockets, onError));
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve());
  });
  return {
    server,
    usePassword: (password) => {
      relay = { ...relay, password };
    },
    close: () =>
      new Promise<void>((resolve) => {
        server.close(() => resolve());
        server.closeAllConnections();
        for (const socket of sockets) socket.destroy();
      }),
  };
}
