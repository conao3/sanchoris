import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { createServer, type IncomingHttpHeaders, type Server } from "node:http";
import type { AddressInfo } from "node:net";

import { afterEach, describe, expect, it } from "vite-plus/test";

import { type CredentialProxy, providerCredential, startCredentialProxy } from "../src/credential-proxy.ts";
import { parseSecrets } from "../src/secrets.ts";
import { FakeControl } from "../src/testing/fake-control.ts";
import type { Worker, WorkerContext } from "../src/worker.ts";
import { BOOTSTRAP_TOKEN, CONTROL_TOKENS, makeSandbox, RUN_ID, RUN_TOKEN, startAgent, startMessage } from "./helpers.ts";

const OAUTH = "sk-ant-oat01-real-secret";
const API_KEY = "sk-ant-api03-real-secret";

interface Seen {
  method: string;
  url: string;
  headers: IncomingHttpHeaders;
  body: string;
}

const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function fakeProvider(): Promise<{ url: string; seen: Seen[] }> {
  const seen: Seen[] = [];
  const server: Server = createServer((request, response) => {
    let body = "";
    request.setEncoding("utf8");
    request.on("data", (chunk: string) => {
      body += chunk;
    });
    request.on("end", () => {
      seen.push({ method: request.method ?? "", url: request.url ?? "", headers: request.headers, body });
      response.writeHead(200, { "content-type": "text/event-stream", "anthropic-ratelimit-unified-status": "allowed" });
      response.write("event: message_start\ndata: {}\n\n");
      response.end("event: message_stop\ndata: {}\n\n");
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  cleanups.push(() => new Promise((resolve) => server.close(() => resolve())));
  return { url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, seen };
}

async function proxyFor(upstream: string, env: Record<string, string>): Promise<CredentialProxy> {
  const proxy = await startCredentialProxy({ upstream, credential: providerCredential(env), proxyEnv: {}, onError: () => {} });
  cleanups.push(proxy.close);
  return proxy;
}

describe("providerCredential", () => {
  it("takes exactly one Claude credential", () => {
    expect(providerCredential({ CLAUDE_CODE_OAUTH_TOKEN: OAUTH })).toEqual({ kind: "oauth", token: OAUTH });
    expect(providerCredential({ ANTHROPIC_API_KEY: API_KEY })).toEqual({ kind: "api_key", key: API_KEY });
    expect(() => providerCredential({})).toThrow(/one worker credential/);
    expect(() => providerCredential({ OPENAI_API_KEY: "sk" })).toThrow(/does not handle OPENAI_API_KEY/);
    expect(() => providerCredential({ ANTHROPIC_API_KEY: API_KEY, CLAUDE_CODE_OAUTH_TOKEN: OAUTH })).toThrow(/one worker credential/);
  });
});

describe("credential proxy", () => {
  it("replaces the placeholder bearer with the OAuth token and keeps the OAuth beta", async () => {
    const provider = await fakeProvider();
    const proxy = await proxyFor(provider.url, { CLAUDE_CODE_OAUTH_TOKEN: OAUTH });
    expect(proxy.workerEnv.CLAUDE_CODE_OAUTH_TOKEN).not.toBe(OAUTH);
    const response = await fetch(`${proxy.workerEnv.ANTHROPIC_BASE_URL}/v1/messages?beta=true`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${proxy.workerEnv.CLAUDE_CODE_OAUTH_TOKEN}`,
        "anthropic-beta": "claude-code-20250219,interleaved-thinking-2025-05-14",
        "content-type": "application/json",
      },
      body: JSON.stringify({ stream: true }),
    });
    expect(response.status).toBe(200);
    expect(response.headers.get("anthropic-ratelimit-unified-status")).toBe("allowed");
    expect(await response.text()).toContain("message_stop");
    expect(provider.seen).toHaveLength(1);
    const [request] = provider.seen;
    expect(request?.url).toBe("/v1/messages?beta=true");
    expect(request?.headers.authorization).toBe(`Bearer ${OAUTH}`);
    expect(request?.headers["anthropic-beta"]).toBe("claude-code-20250219,interleaved-thinking-2025-05-14,oauth-2025-04-20");
    expect(request?.headers["x-api-key"]).toBeUndefined();
    expect(request?.body).toBe('{"stream":true}');
  });

  it("sends an API key as x-api-key and drops any bearer the client sent", async () => {
    const provider = await fakeProvider();
    const proxy = await proxyFor(provider.url, { ANTHROPIC_API_KEY: API_KEY });
    await fetch(`${proxy.workerEnv.ANTHROPIC_BASE_URL}/v1/messages`, {
      method: "POST",
      headers: { "x-api-key": proxy.workerEnv.ANTHROPIC_API_KEY ?? "", authorization: "Bearer stolen" },
      body: "{}",
    }).then((response) => response.text());
    const [request] = provider.seen;
    expect(request?.headers["x-api-key"]).toBe(API_KEY);
    expect(request?.headers.authorization).toBeUndefined();
  });

  it("answers 502 when the provider is unreachable", async () => {
    const proxy = await proxyFor("http://127.0.0.1:9", { ANTHROPIC_API_KEY: API_KEY });
    const response = await fetch(`${proxy.workerEnv.ANTHROPIC_BASE_URL}/v1/messages`, { method: "POST", body: "{}" });
    expect(response.status).toBe(502);
  });
});

describe("agent secrets", () => {
  it("reads the bootstrap token and the relay URL from one JSON document", () => {
    expect(parseSecrets('{"bootstrapToken":"t","relayUrl":"http://run:x@10.0.0.1:3128"}')).toEqual({
      bootstrapToken: "t",
      relayUrl: "http://run:x@10.0.0.1:3128",
    });
    expect(parseSecrets('{"bootstrapToken":"t"}\n')).toEqual({ bootstrapToken: "t", relayUrl: null });
    expect(() => parseSecrets("")).toThrow(/not JSON/);
    expect(() => parseSecrets('{"bootstrapToken":""}')).toThrow();
    expect(() => parseSecrets('{"runToken":"t"}')).toThrow();
  });

  it("keeps the run token and the provider credential out of the worker's and exec's environment", async () => {
    const provider = await fakeProvider();
    const control = await FakeControl.listen({ runId: RUN_ID, ...CONTROL_TOKENS, onSkillRequest: () => ({ ok: true, result: {} }) });
    cleanups.push(() => control.close());
    const sandbox = await makeSandbox({ HTTPS_PROXY: "http://127.0.0.1:3129", NO_PROXY: "127.0.0.1,localhost" });
    cleanups.push(sandbox.cleanup);
    let workerEnv: NodeJS.ProcessEnv | null = null;
    const factory = (context: WorkerContext): Worker => {
      workerEnv = context.env;
      return {
        start: async () => ({ sessionId: "session-1" }),
        send: () => {},
        interrupt: async () => {},
        readRecord: async () => ({ relativePath: "r", bytes: Buffer.alloc(0) }),
        close: async () => {},
      };
    };
    const { exitCode } = startAgent(sandbox, control, factory, {
      credentialSource: { kind: "channel", gitUrl: "https://github.com", branch: "sanchoris/task-1", repositories: [] },
      providerUpstream: provider.url,
    });
    await control.waitFor("hello");
    control.send(startMessage());
    const request = await control.waitFor("worker_credential_request");
    control.send({ type: "worker_credential_response", requestId: request.requestId, ok: true, env: { CLAUDE_CODE_OAUTH_TOKEN: OAUTH } });
    await control.waitFor("ready");

    const env = workerEnv as NodeJS.ProcessEnv | null;
    if (env === null) throw new Error("the worker did not start");
    const serialized = JSON.stringify(env);
    expect(serialized).not.toContain(OAUTH);
    expect(serialized).not.toContain(BOOTSTRAP_TOKEN);
    expect(serialized).not.toContain(RUN_TOKEN);
    expect(env.ANTHROPIC_BASE_URL).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
    expect(env.CLAUDE_CODE_OAUTH_TOKEN).toBeDefined();
    expect(env.HTTPS_PROXY).toBe("http://127.0.0.1:3129");

    await fetch(`${env.ANTHROPIC_BASE_URL}/v1/messages`, {
      method: "POST",
      headers: { authorization: `Bearer ${env.CLAUDE_CODE_OAUTH_TOKEN}` },
      body: "{}",
    }).then((response) => response.text());
    expect(provider.seen[0]?.headers.authorization).toBe(`Bearer ${OAUTH}`);

    const requestId = randomUUID();
    control.send({ type: "exec", requestId, purpose: "exec", commands: ["env"] });
    const result = await control.waitFor("exec_result", (message) => message.requestId === requestId);
    const output = result.results[0]?.stdout ?? "";
    expect(output).toContain("SANCHORIS_SOCKET_PATH=");
    expect(output).not.toContain(OAUTH);
    expect(output).not.toContain(BOOTSTRAP_TOKEN);
    expect(output).not.toContain(RUN_TOKEN);

    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
  });
});
