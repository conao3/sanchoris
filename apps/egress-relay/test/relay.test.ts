import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { once } from "node:events";
import { connect, createServer, type AddressInfo, type Server, type Socket } from "node:net";
import { test } from "node:test";

import { createRelay, decide, hostMatches, isPrivateAddress, proxyToken, verifyToken, type RelayOptions } from "../src/relay.ts";

const KEY = Buffer.alloc(32, 7);
const RUST_VECTOR =
  "7b2272756e223a2230303030303030302d303030302d343030302d383030302d303030303030303030303031222c22657870223a313739333036313030302c22616c6c6f77223a5b226769746875622e636f6d222c2272656769737472792e6e706d6a732e6f7267225d7d.059da54d7959fbfa409a87da6f695f81b67dee7f2fdc33d80d0b7299b5b95200";

function sign(claims: object): string {
  const message = Buffer.from(JSON.stringify(claims)).toString("hex");
  return `${message}.${createHmac("sha256", KEY).update(message).digest("hex")}`;
}

test("verifies the token the control plane signs", () => {
  assert.deepEqual(verifyToken(RUST_VECTOR, KEY, 1_793_060_000), {
    run: "00000000-0000-4000-8000-000000000001",
    exp: 1_793_061_000,
    allow: ["github.com", "registry.npmjs.org"],
  });
  assert.equal(verifyToken(RUST_VECTOR, KEY, 1_793_061_000), null);
  assert.equal(verifyToken(RUST_VECTOR, Buffer.alloc(32, 8), 1_793_060_000), null);
  assert.equal(verifyToken(RUST_VECTOR.replace("7b22", "7b23"), KEY, 1_793_060_000), null);
});

test("matches exact hosts and subdomain wildcards", () => {
  assert.equal(hostMatches("github.com", "github.com"), true);
  assert.equal(hostMatches("API.GitHub.com.", "api.github.com"), true);
  assert.equal(hostMatches("raw.githubusercontent.com", "*.githubusercontent.com"), true);
  assert.equal(hostMatches("githubusercontent.com", "*.githubusercontent.com"), false);
  assert.equal(hostMatches("evilgithub.com", "github.com"), false);
});

test("applies the deny list before the platform hosts and the session allow list", () => {
  const options = { platformHosts: ["api.anthropic.com"], denyHosts: ["*.pool.example"] };
  const claims = { run: "r", exp: 0, allow: ["github.com", "mine.pool.example"] };
  assert.deepEqual(decide("api.anthropic.com", 443, claims, options), { allowed: true, rule: "platform" });
  assert.deepEqual(decide("github.com", 443, claims, options), { allowed: true, rule: "session" });
  assert.deepEqual(decide("mine.pool.example", 443, claims, options), { allowed: false, reason: "operator deny list" });
  assert.deepEqual(decide("example.com", 443, claims, options), { allowed: false, reason: "not in Network.Allow" });
  assert.deepEqual(decide("github.com", 22, claims, options), { allowed: false, reason: "port 22 is not 443" });
});

test("reads the token from Basic proxy credentials", () => {
  assert.equal(proxyToken(`Basic ${Buffer.from("run:abc.def").toString("base64")}`), "abc.def");
  assert.equal(proxyToken("Bearer abc"), null);
  assert.equal(proxyToken(undefined), null);
});

test("treats private and link-local addresses as private", () => {
  assert.equal(isPrivateAddress("169.254.170.2"), true);
  assert.equal(isPrivateAddress("10.1.0.5"), true);
  assert.equal(isPrivateAddress("::1"), true);
  assert.equal(isPrivateAddress("140.82.112.3"), false);
});

async function listen(server: Server): Promise<number> {
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return (server.address() as AddressInfo).port;
}

async function exchange(port: number, request: string, payload: string | null): Promise<string> {
  const socket: Socket = connect(port, "127.0.0.1");
  await once(socket, "connect");
  socket.write(request);
  let received = "";
  socket.setEncoding("utf8");
  socket.on("data", (chunk: string) => {
    received += chunk;
    if (payload !== null && received.includes("\r\n\r\n") && received.startsWith("HTTP/1.1 200")) {
      socket.write(payload);
      payload = null;
    }
  });
  await new Promise<void>((resolve) => {
    socket.on("close", () => resolve());
    setTimeout(() => socket.end(), 300);
  });
  return received;
}

test("tunnels allowed hosts and refuses the rest", async (t) => {
  const echo = createServer((socket) => socket.pipe(socket));
  const echoPort = await listen(echo);
  const logs: Record<string, unknown>[] = [];
  const options: RelayOptions = {
    signingKey: KEY,
    platformHosts: ["api.anthropic.com"],
    denyHosts: ["*.pool.example"],
    log: (entry) => logs.push(entry),
    resolve: async (host) => (host === "metadata.example" ? "169.254.170.2" : "140.82.112.3"),
    connect: () => connect(echoPort, "127.0.0.1"),
    now: () => 1_000_000,
  };
  const relay = createRelay(options);
  const port = await listen(relay);
  t.after(() => {
    relay.close();
    echo.close();
  });
  const token = sign({ run: "run-1", exp: 2_000, allow: ["github.com", "metadata.example"] });
  const auth = `Proxy-Authorization: Basic ${Buffer.from(`run:${token}`).toString("base64")}\r\n`;

  const allowed = await exchange(port, `CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n${auth}\r\n`, "ping");
  assert.match(allowed, /^HTTP\/1\.1 200 Connection Established\r\n\r\nping$/);

  const anonymous = await exchange(port, "CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n", null);
  assert.match(anonymous, /^HTTP\/1\.1 407 /);

  const outside = await exchange(port, `CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n${auth}\r\n`, null);
  assert.match(outside, /^HTTP\/1\.1 403 /);

  const metadata = await exchange(port, `CONNECT metadata.example:443 HTTP/1.1\r\nHost: metadata.example:443\r\n${auth}\r\n`, null);
  assert.match(metadata, /^HTTP\/1\.1 403 /);

  const expired = sign({ run: "run-1", exp: 999, allow: ["github.com"] });
  const stale = `Proxy-Authorization: Basic ${Buffer.from(`run:${expired}`).toString("base64")}\r\n`;
  const late = await exchange(port, `CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n${stale}\r\n`, null);
  assert.match(late, /^HTTP\/1\.1 407 /);

  const plain = await exchange(port, "GET http://github.com/ HTTP/1.1\r\nHost: github.com\r\n\r\n", null);
  assert.match(plain, /^HTTP\/1\.1 405 /);

  assert.deepEqual(
    logs.map((entry) => [entry.host, entry.allowed, entry.reason ?? entry.rule]),
    [
      ["github.com", true, "session"],
      ["github.com", false, "proxy authentication"],
      ["example.com", false, "not in Network.Allow"],
      ["metadata.example", false, "private address"],
      ["github.com", false, "proxy authentication"],
    ],
  );
});
