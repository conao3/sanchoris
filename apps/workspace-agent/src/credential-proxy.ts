import http, { type IncomingHttpHeaders, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import https from "node:https";
import type { AddressInfo } from "node:net";

const OAUTH_BETA = "oauth-2025-04-20";
const OAUTH_PLACEHOLDER = "sk-ant-oat01-sanchoris-credential-proxy";
const API_KEY_PLACEHOLDER = "sk-ant-api03-sanchoris-credential-proxy";

const DROPPED_REQUEST_HEADERS = new Set([
  "authorization",
  "x-api-key",
  "host",
  "connection",
  "keep-alive",
  "proxy-authorization",
  "proxy-connection",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);
const DROPPED_RESPONSE_HEADERS = new Set(["connection", "keep-alive", "transfer-encoding"]);

export type ProviderCredential = { kind: "oauth"; token: string } | { kind: "api_key"; key: string };

export function providerCredential(env: Record<string, string>): ProviderCredential {
  const names = Object.keys(env);
  if (names.length !== 1) throw new Error(`Expected one worker credential, got ${names.join(", ") || "none"}`);
  const token = env.CLAUDE_CODE_OAUTH_TOKEN;
  if (token !== undefined && token !== "") return { kind: "oauth", token };
  const key = env.ANTHROPIC_API_KEY;
  if (key !== undefined && key !== "") return { kind: "api_key", key };
  throw new Error(`The credential proxy does not handle ${names[0]}`);
}

function upstreamHeaders(headers: IncomingHttpHeaders, credential: ProviderCredential): Record<string, string | string[]> {
  const result: Record<string, string | string[]> = {};
  for (const [name, value] of Object.entries(headers)) {
    if (value === undefined || DROPPED_REQUEST_HEADERS.has(name)) continue;
    result[name] = value;
  }
  if (credential.kind === "api_key") {
    result["x-api-key"] = credential.key;
    return result;
  }
  result.authorization = `Bearer ${credential.token}`;
  const beta = [result["anthropic-beta"] ?? []]
    .flat()
    .flatMap((value) => value.split(","))
    .map((value) => value.trim())
    .filter((value) => value !== "");
  if (!beta.includes(OAUTH_BETA)) beta.push(OAUTH_BETA);
  result["anthropic-beta"] = beta.join(",");
  return result;
}

function responseHeaders(headers: IncomingHttpHeaders): Record<string, string | string[]> {
  const result: Record<string, string | string[]> = {};
  for (const [name, value] of Object.entries(headers)) {
    if (value !== undefined && !DROPPED_RESPONSE_HEADERS.has(name)) result[name] = value;
  }
  return result;
}

export interface CredentialProxyOptions {
  upstream: string;
  credential: ProviderCredential;
  proxyEnv: NodeJS.ProcessEnv;
  onError: (error: Error) => void;
}

export interface CredentialProxy {
  server: Server;
  workerEnv: Record<string, string>;
  close: () => Promise<void>;
}

export async function startCredentialProxy(options: CredentialProxyOptions): Promise<CredentialProxy> {
  const upstream = new URL(options.upstream);
  const secure = upstream.protocol === "https:";
  if (!secure && upstream.protocol !== "http:") throw new Error(`The credential proxy cannot reach ${options.upstream}`);
  const agentOptions = { keepAlive: true, proxyEnv: options.proxyEnv } as http.AgentOptions;
  const agent = secure ? new https.Agent(agentOptions) : new http.Agent(agentOptions);
  const forward = (request: IncomingMessage, response: ServerResponse) => {
    const path = request.url ?? "/";
    if (!path.startsWith("/")) {
      response.writeHead(400, { "content-length": "0" });
      response.end();
      return;
    }
    const target = new URL(path, upstream);
    const outbound = (secure ? https : http).request(target, {
      method: request.method,
      headers: upstreamHeaders(request.headers, options.credential),
      agent,
    });
    outbound.on("response", (reply) => {
      response.writeHead(reply.statusCode ?? 502, responseHeaders(reply.headers));
      reply.pipe(response);
    });
    outbound.on("error", (error) => {
      options.onError(error);
      if (!response.headersSent) {
        response.writeHead(502, { "content-type": "application/json" });
        response.end(JSON.stringify({ type: "error", error: { type: "api_error", message: "The credential proxy could not reach the provider" } }));
      } else {
        response.destroy();
      }
    });
    response.on("close", () => outbound.destroy());
    request.pipe(outbound);
  };
  const server = http.createServer(forward);
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve());
  });
  const { port } = server.address() as AddressInfo;
  const workerEnv: Record<string, string> = {
    ANTHROPIC_BASE_URL: `http://127.0.0.1:${port}`,
    ...(options.credential.kind === "oauth" ? { CLAUDE_CODE_OAUTH_TOKEN: OAUTH_PLACEHOLDER } : { ANTHROPIC_API_KEY: API_KEY_PLACEHOLDER }),
  };
  return {
    server,
    workerEnv,
    close: () =>
      new Promise<void>((resolve) => {
        server.close(() => resolve());
        server.closeAllConnections();
        agent.destroy();
      }),
  };
}
