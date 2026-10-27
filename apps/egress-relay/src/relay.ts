import { createHmac, timingSafeEqual } from "node:crypto";
import { lookup } from "node:dns/promises";
import { createServer, type IncomingMessage, type Server } from "node:http";
import { BlockList, isIP, type Socket } from "node:net";

export interface RelayClaims {
  run: string;
  exp: number;
  allow: string[];
}

export interface RelayOptions {
  signingKey: Buffer;
  platformHosts: string[];
  denyHosts: string[];
  log: (entry: Record<string, unknown>) => void;
  resolve: (host: string) => Promise<string>;
  connect: (port: number, address: string) => Socket;
  now: () => number;
}

export type Decision = { allowed: true; rule: "platform" | "session" } | { allowed: false; reason: string };

const ALLOWED_PORT = 443;

const privateAddresses = new BlockList();
privateAddresses.addSubnet("0.0.0.0", 8, "ipv4");
privateAddresses.addSubnet("10.0.0.0", 8, "ipv4");
privateAddresses.addSubnet("100.64.0.0", 10, "ipv4");
privateAddresses.addSubnet("127.0.0.0", 8, "ipv4");
privateAddresses.addSubnet("169.254.0.0", 16, "ipv4");
privateAddresses.addSubnet("172.16.0.0", 12, "ipv4");
privateAddresses.addSubnet("192.168.0.0", 16, "ipv4");
privateAddresses.addSubnet("::", 128, "ipv6");
privateAddresses.addSubnet("::1", 128, "ipv6");
privateAddresses.addSubnet("fc00::", 7, "ipv6");
privateAddresses.addSubnet("fe80::", 10, "ipv6");

export function isPrivateAddress(address: string): boolean {
  const family = isIP(address);
  if (family === 0) throw new Error(`${address} is not an IP address`);
  return privateAddresses.check(address, family === 4 ? "ipv4" : "ipv6");
}

export function hostMatches(host: string, pattern: string): boolean {
  const name = host.toLowerCase().replace(/\.$/, "");
  const rule = pattern.toLowerCase().replace(/\.$/, "");
  if (rule.startsWith("*.")) return name.endsWith(rule.slice(1)) && name.length > rule.length - 1;
  return name === rule;
}

export function verifyToken(token: string, key: Buffer, nowSeconds: number): RelayClaims | null {
  const dot = token.indexOf(".");
  if (dot <= 0) return null;
  const message = token.slice(0, dot);
  const signature = Buffer.from(token.slice(dot + 1), "hex");
  const expected = createHmac("sha256", key).update(message).digest();
  if (signature.length !== expected.length || !timingSafeEqual(signature, expected)) return null;
  let claims: unknown;
  try {
    claims = JSON.parse(Buffer.from(message, "hex").toString("utf8"));
  } catch {
    return null;
  }
  const { run, exp, allow } = claims as Partial<RelayClaims>;
  if (typeof run !== "string" || typeof exp !== "number" || !Array.isArray(allow)) return null;
  if (!allow.every((host) => typeof host === "string")) return null;
  if (nowSeconds >= exp) return null;
  return { run, exp, allow };
}

export function decide(host: string, port: number, claims: RelayClaims, options: Pick<RelayOptions, "platformHosts" | "denyHosts">): Decision {
  if (port !== ALLOWED_PORT) return { allowed: false, reason: `port ${port} is not ${ALLOWED_PORT}` };
  if (options.denyHosts.some((pattern) => hostMatches(host, pattern))) return { allowed: false, reason: "operator deny list" };
  if (options.platformHosts.some((pattern) => hostMatches(host, pattern))) return { allowed: true, rule: "platform" };
  if (claims.allow.some((pattern) => hostMatches(host, pattern))) return { allowed: true, rule: "session" };
  return { allowed: false, reason: "not in Network.Allow" };
}

export function proxyToken(header: string | undefined): string | null {
  if (header === undefined) return null;
  const match = /^Basic\s+([A-Za-z0-9+/=]+)$/i.exec(header.trim());
  if (!match?.[1]) return null;
  const decoded = Buffer.from(match[1], "base64").toString("utf8");
  const colon = decoded.indexOf(":");
  if (colon < 0) return null;
  return decoded.slice(colon + 1);
}

function target(url: string | undefined): { host: string; port: number } | null {
  const match = /^\[?([^\]]+?)\]?:(\d{1,5})$/.exec(url ?? "");
  if (!match?.[1] || !match[2]) return null;
  return { host: match[1], port: Number(match[2]) };
}

function refuse(socket: Socket, status: string, headers: string[] = []): void {
  socket.end([`HTTP/1.1 ${status}`, ...headers, "Content-Length: 0", "", ""].join("\r\n"));
}

function tunnel(request: IncomingMessage, client: Socket, head: Buffer, options: RelayOptions): void {
  client.on("error", () => client.destroy());
  const destination = target(request.url);
  if (!destination) {
    refuse(client, "400 Bad Request");
    return;
  }
  const token = proxyToken(request.headers["proxy-authorization"]);
  const claims = token === null ? null : verifyToken(token, options.signingKey, Math.floor(options.now() / 1000));
  if (!claims) {
    options.log({ event: "connect", host: destination.host, port: destination.port, allowed: false, reason: "proxy authentication" });
    refuse(client, "407 Proxy Authentication Required", ['Proxy-Authenticate: Basic realm="sanchoris"']);
    return;
  }
  const decision = decide(destination.host, destination.port, claims, options);
  const entry = { event: "connect", run: claims.run, host: destination.host, port: destination.port };
  if (!decision.allowed) {
    options.log({ ...entry, allowed: false, reason: decision.reason });
    refuse(client, "403 Forbidden");
    return;
  }
  options.resolve(destination.host).then(
    (address) => {
      if (isPrivateAddress(address)) {
        options.log({ ...entry, address, allowed: false, reason: "private address" });
        refuse(client, "403 Forbidden");
        return;
      }
      const upstream = options.connect(destination.port, address);
      let established = false;
      upstream.on("connect", () => {
        established = true;
        options.log({ ...entry, address, allowed: true, rule: decision.rule });
        client.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        if (head.length > 0) upstream.write(head);
        upstream.pipe(client);
        client.pipe(upstream);
      });
      upstream.on("error", (error) => {
        options.log({ ...entry, address, allowed: true, error: error.message });
        if (!established) refuse(client, "502 Bad Gateway");
        else client.destroy();
      });
      client.on("close", () => upstream.destroy());
    },
    (error: Error) => {
      options.log({ ...entry, allowed: true, error: error.message });
      refuse(client, "502 Bad Gateway");
    },
  );
}

export function createRelay(options: RelayOptions): Server {
  const server = createServer((_request, response) => {
    response.writeHead(405, { Allow: "CONNECT", "Content-Length": "0" });
    response.end();
  });
  server.on("connect", (request: IncomingMessage, client: Socket, head: Buffer) => tunnel(request, client, head, options));
  return server;
}

export async function resolveHost(host: string): Promise<string> {
  if (isIP(host) !== 0) return host;
  const { address } = await lookup(host, { family: 4 });
  return address;
}

export function hostList(value: string): string[] {
  return value
    .split(",")
    .map((host) => host.trim())
    .filter((host) => host !== "");
}
