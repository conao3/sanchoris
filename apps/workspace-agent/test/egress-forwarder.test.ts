import { once } from "node:events";
import { createServer, type IncomingMessage, type Server } from "node:http";
import { connect, createServer as createTcpServer, type AddressInfo, type Socket } from "node:net";

import { afterEach, describe, expect, it } from "vitest";

import { loopbackProxyPort, parseRelayUrl, startEgressForwarder } from "../src/egress-forwarder.ts";

const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function listen(server: Server | ReturnType<typeof createTcpServer>): Promise<number> {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  cleanups.push(() => new Promise((resolve) => server.close(() => resolve())));
  return (server.address() as AddressInfo).port;
}

async function fakeRelay(): Promise<{ port: number; seen: Array<{ url: string; auth: string | undefined }> }> {
  const echo = createTcpServer((socket) => socket.pipe(socket));
  const echoPort = await listen(echo);
  const seen: Array<{ url: string; auth: string | undefined }> = [];
  const relay = createServer();
  relay.on("connect", (request: IncomingMessage, client: Socket) => {
    seen.push({ url: request.url ?? "", auth: request.headers["proxy-authorization"] });
    if (request.headers["proxy-authorization"] !== `Basic ${Buffer.from("run:relay-token").toString("base64")}`) {
      client.end("HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n");
      return;
    }
    if (request.url === "blocked.example:443") {
      client.end("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
      return;
    }
    const upstream = connect(echoPort, "127.0.0.1", () => {
      client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
      upstream.pipe(client);
      client.pipe(upstream);
    });
    client.on("close", () => upstream.destroy());
  });
  return { port: await listen(relay), seen };
}

async function tunnel(port: number, target: string, payload: string): Promise<string> {
  const socket = connect(port, "127.0.0.1");
  await once(socket, "connect");
  socket.setEncoding("utf8");
  socket.write(`CONNECT ${target} HTTP/1.1\r\nHost: ${target}\r\n\r\n`);
  let received = "";
  let sent = false;
  await new Promise<void>((resolve) => {
    socket.on("data", (chunk: string) => {
      received += chunk;
      if (!sent && received.startsWith("HTTP/1.1 200") && received.includes("\r\n\r\n")) {
        sent = true;
        socket.write(payload);
      }
      if (received.endsWith(payload)) socket.end();
    });
    socket.on("close", () => resolve());
    setTimeout(() => socket.destroy(), 2000);
  });
  return received;
}

describe("egress forwarder", () => {
  it("adds the relay credentials to each CONNECT and passes the relay's answer through", async () => {
    const relay = await fakeRelay();
    const forwarder = await startEgressForwarder(parseRelayUrl(`http://run:relay-token@127.0.0.1:${relay.port}`), 0, () => {});
    cleanups.push(forwarder.close);
    const port = (forwarder.server.address() as AddressInfo).port;

    expect(await tunnel(port, "github.com:443", "ping")).toBe("HTTP/1.1 200 Connection Established\r\n\r\nping");
    expect(await tunnel(port, "blocked.example:443", "ping")).toMatch(/^HTTP\/1\.1 403 Forbidden\r\n/);
    expect(relay.seen.map((entry) => entry.url)).toEqual(["github.com:443", "blocked.example:443"]);
  });

  it("parses the relay URL and requires a loopback HTTPS_PROXY", () => {
    expect(parseRelayUrl("http://run:a.b@10.0.1.5:3128")).toEqual({
      host: "10.0.1.5",
      port: 3128,
      authorization: `Basic ${Buffer.from("run:a.b").toString("base64")}`,
    });
    expect(() => parseRelayUrl("http://10.0.1.5:3128")).toThrow(/relay URL/);
    expect(loopbackProxyPort("http://127.0.0.1:3129")).toBe(3129);
    expect(() => loopbackProxyPort("http://run:x@10.0.1.5:3128")).toThrow(/HTTPS_PROXY/);
    expect(() => loopbackProxyPort(undefined)).toThrow(/HTTPS_PROXY/);
  });
});
