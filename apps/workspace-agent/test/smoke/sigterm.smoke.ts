import { createRequire } from "node:module";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, describe, expect, it } from "vite-plus/test";

import { FakeControl } from "../../src/testing/fake-control.ts";
import { spawnAgent } from "../../src/testing/spawn-agent.ts";

const TOOL_USE_ID = "toolu_sigterm_smoke";

function claudePath(): string {
  const require = createRequire(import.meta.url);
  const sdk = require.resolve("@anthropic-ai/claude-agent-sdk");
  return require.resolve(`@anthropic-ai/claude-agent-sdk-linux-${process.arch}/claude`, { paths: [sdk] });
}

type Block = { type: "text"; text: string } | { type: "tool_use"; id: string; name: string; input: Record<string, unknown> };

interface UpstreamRequest {
  path: string;
  body: { messages?: Array<{ role: string; content: unknown }> };
}

function contentText(content: unknown): string {
  return JSON.stringify(content);
}

class FakeUpstream {
  readonly requests: UpstreamRequest[] = [];
  private readonly server: Server;

  private constructor() {
    this.server = createServer((request, response) => void this.handle(request, response));
  }

  static async listen(): Promise<FakeUpstream> {
    const upstream = new FakeUpstream();
    await new Promise<void>((resolve) => upstream.server.listen(0, "127.0.0.1", resolve));
    return upstream;
  }

  get url(): string {
    return `http://127.0.0.1:${(this.server.address() as AddressInfo).port}`;
  }

  close(): Promise<void> {
    this.server.closeAllConnections();
    return new Promise((resolve) => this.server.close(() => resolve()));
  }

  private async handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(chunk as Buffer);
    const raw = Buffer.concat(chunks).toString("utf8");
    const body = raw === "" ? {} : (JSON.parse(raw) as UpstreamRequest["body"] & { stream?: boolean });
    const path = (request.url ?? "/").split("?")[0] ?? "/";
    this.requests.push({ path, body });
    if (path.endsWith("/count_tokens")) {
      response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ input_tokens: 10 }));
      return;
    }
    if (!path.endsWith("/v1/messages")) {
      response.writeHead(404, { "content-type": "application/json" }).end(JSON.stringify({ type: "error", error: { type: "not_found_error", message: path } }));
      return;
    }
    const messages = body.messages ?? [];
    const users = messages.filter((message) => message.role === "user");
    const conversation = contentText(users.map((message) => message.content));
    const answered = conversation.includes("tool_result");
    let blocks: Block[];
    if (conversation.includes("RESUME-CHECK")) {
      blocks = [{ type: "text", text: "RESUMED" }];
    } else if (conversation.includes("START-LONG") && !answered) {
      blocks = [{ type: "tool_use", id: TOOL_USE_ID, name: "Bash", input: { command: "node -e \"setTimeout(() => {}, 120000)\"", description: "Run the long job" } }];
    } else {
      blocks = [{ type: "text", text: "OK" }];
    }
    const stop = blocks.some((block) => block.type === "tool_use") ? "tool_use" : "end_turn";
    const message = {
      id: `msg_${this.requests.length}`,
      type: "message",
      role: "assistant",
      model: "fake-model",
      content: [] as Block[],
      stop_reason: null as string | null,
      stop_sequence: null,
      usage: { input_tokens: 10, output_tokens: 1, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 },
    };
    if (!body.stream) {
      response
        .writeHead(200, { "content-type": "application/json" })
        .end(JSON.stringify({ ...message, content: blocks, stop_reason: stop }));
      return;
    }
    response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
    const send = (event: string, data: unknown) => response.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
    send("message_start", { type: "message_start", message });
    blocks.forEach((block, index) => {
      if (block.type === "text") {
        send("content_block_start", { type: "content_block_start", index, content_block: { type: "text", text: "" } });
        send("content_block_delta", { type: "content_block_delta", index, delta: { type: "text_delta", text: block.text } });
      } else {
        send("content_block_start", { type: "content_block_start", index, content_block: { ...block, input: {} } });
        send("content_block_delta", {
          type: "content_block_delta",
          index,
          delta: { type: "input_json_delta", partial_json: JSON.stringify(block.input) },
        });
      }
      send("content_block_stop", { type: "content_block_stop", index });
    });
    send("message_delta", { type: "message_delta", delta: { stop_reason: stop, stop_sequence: null }, usage: { output_tokens: 5 } });
    send("message_stop", { type: "message_stop" });
    response.end();
  }
}

const cleanups: Array<() => Promise<void>> = [];

afterAll(async () => {
  for (const cleanup of cleanups.reverse()) await cleanup();
});

function agentEnv(root: string, workspaceDir: string, upstream: FakeUpstream, control: FakeControl): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (name === "CLAUDECODE" || name.startsWith("CLAUDE_CODE_") || name.startsWith("ANTHROPIC_") || name === "CLAUDE_AGENT_SDK_VERSION") continue;
    env[name] = value;
  }
  return {
    ...env,
    ANTHROPIC_BASE_URL: upstream.url,
    ANTHROPIC_API_KEY: "sk-ant-api03-fake",
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_AUTOUPDATER: "1",
    CLAUDE_CONFIG_DIR: join(root, "claude"),
    SANCHORIS_CLAUDE_PATH: claudePath(),
    SANCHORIS_RUN_ID: "sigterm-run",
    SANCHORIS_CONTROL_URL: control.url,
    SANCHORIS_SOCKET_PATH: join(root, "agent.sock"),
    SANCHORIS_WORKSPACE_DIR: workspaceDir,
    SANCHORIS_CREDENTIAL_SOURCE: "environment",
  };
}

async function workspace(label: string): Promise<{ root: string; workspaceDir: string }> {
  const root = await mkdtemp(join(tmpdir(), `sanchoris-sigterm-${label}-`));
  const workspaceDir = join(root, "workspace");
  await mkdir(workspaceDir);
  await mkdir(join(root, "claude"));
  cleanups.push(() => rm(root, { recursive: true, force: true }));
  return { root, workspaceDir };
}

describe("Claude Code on SIGTERM", () => {
  it("interrupts the turn, uploads a record that resumes in a new workspace, and exits 0", async () => {
    const upstream = await FakeUpstream.listen();
    cleanups.push(() => upstream.close());
    const control = await FakeControl.listen({
      runId: "sigterm-run",
      bootstrapTokens: ["sigterm-bootstrap-a", "sigterm-bootstrap-b"],
      runToken: "sigterm-run-token",
      onSkillRequest: () => ({ ok: false, error: { code: "unused", message: "unused" } }),
    });
    cleanups.push(() => control.close());

    const first = await workspace("a");
    const agentA = spawnAgent(agentEnv(first.root, first.workspaceDir, upstream, control), "sigterm-bootstrap-a", (line) =>
      process.stderr.write(`[a] ${line}\n`),
    );
    await control.waitFor("hello", () => true, 30_000);
    control.send({
      type: "start",
      agent: "claude",
      model: "claude-sonnet-5-5",
      systemPrompt: null,
      skills: [],
      instruction: "START-LONG: run the long command.",
      resume: null,
      heartbeatSeconds: 30,
    });
    await control.waitFor("ready", () => true, 60_000);
    await control.waitFor("event", (message) => message.kind === "tool_use", 60_000);
    await new Promise((resolve) => setTimeout(resolve, 2_000));

    const signaledAt = Date.now();
    agentA.child.kill("SIGTERM");
    const stopping = await control.waitFor("stopping", () => true, 10_000);
    const interrupted = await control.waitFor("turn_end", (message) => message.turn === 1, 60_000);
    const record = await control.waitFor("session_record", (message) => message.turn === 1, 60_000);
    const exitCode = await agentA.exitCode;
    const exitedAt = Date.now();
    process.stderr.write(`sigterm: exit ${exitCode} after ${exitedAt - signaledAt} ms; turn_end ${JSON.stringify(interrupted)}\n`);
    expect(stopping.signal).toBe("SIGTERM");
    expect(interrupted.result).toBe("interrupted");
    expect(exitCode).toBe(0);
    expect(exitedAt - signaledAt).toBeLessThan(30_000);
    const lines = (control.records.get(record.key) as Buffer).toString("utf8").split("\n").filter((line) => line !== "");
    process.stderr.write(`record: ${lines.length} lines\n${lines.map((line) => line.slice(0, 300)).join("\n")}\n`);
    expect(lines.some((line) => line.includes(TOOL_USE_ID))).toBe(true);

    const received = control.received.length;
    const second = await workspace("b");
    const agentB = spawnAgent(agentEnv(second.root, second.workspaceDir, upstream, control), "sigterm-bootstrap-b", (line) =>
      process.stderr.write(`[b] ${line}\n`),
    );
    await control.waitFor("hello", (message) => control.received.indexOf(message) >= received, 30_000);
    control.send({
      type: "start",
      agent: "claude",
      model: "claude-sonnet-5-5",
      systemPrompt: null,
      skills: [],
      instruction: "RESUME-CHECK: continue where you left off.",
      resume: {
        sessionId: record.sessionId,
        url: control.recordUrl(record.key),
        relativePath: record.relativePath,
        sha256: record.sha256,
        lastTurn: record.turn,
      },
      heartbeatSeconds: 30,
    });
    const ready = await control.waitFor("ready", (message) => message.resumed, 60_000);
    expect(ready.sessionId).toBe(record.sessionId);
    const resumed = await control.waitFor("turn_end", (message) => message.turn === 2, 60_000);
    expect(resumed.result).toBe("completed");
    expect(resumed.summary).toContain("RESUMED");
    const resumeRequest = upstream.requests.filter((request) => contentText(request.body.messages ?? []).includes("RESUME-CHECK")).at(-1);
    const history = JSON.stringify(resumeRequest?.body.messages ?? []);
    process.stderr.write(`resumed request messages: ${history.slice(0, 2000)}\n`);
    expect(history).toContain("START-LONG");
    expect(history).toContain(TOOL_USE_ID);
    control.send({ type: "shutdown", reason: "done" });
    expect(await agentB.exitCode).toBe(0);
  });
});
