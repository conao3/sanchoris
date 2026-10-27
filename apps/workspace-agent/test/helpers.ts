import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { WorkspaceAgent } from "../src/agent.ts";
import type { Logger } from "../src/log.ts";
import type { StartMessage } from "../src/protocol.ts";
import { writeCliWrapper } from "../src/testing/cli-wrapper.ts";
import type { FakeControl } from "../src/testing/fake-control.ts";
import type { WorkerFactory } from "../src/worker.ts";

export const RUN_ID = "run-1";
export const TOKEN = "test-token";

export interface Sandbox {
  root: string;
  binDir: string;
  workspaceDir: string;
  socketPath: string;
  env: NodeJS.ProcessEnv;
  cleanup: () => Promise<void>;
}

export async function makeSandbox(extraEnv: NodeJS.ProcessEnv = {}): Promise<Sandbox> {
  const root = await mkdtemp(join(tmpdir(), "sanchoris-"));
  const binDir = join(root, "bin");
  const workspaceDir = join(root, "workspace");
  await writeCliWrapper(binDir);
  await mkdir(workspaceDir);
  const socketPath = join(root, "agent.sock");
  return {
    root,
    binDir,
    workspaceDir,
    socketPath,
    env: {
      ...process.env,
      PATH: `${binDir}:${process.env.PATH ?? ""}`,
      SANCHORIS_SOCKET_PATH: socketPath,
      ...extraEnv,
    },
    cleanup: () => rm(root, { recursive: true, force: true }),
  };
}

export const silentLogger: Logger = () => {};

export function startAgent(
  sandbox: Sandbox,
  control: FakeControl,
  createWorker: WorkerFactory,
  overrides: { token?: string } = {},
): { agent: WorkspaceAgent; exitCode: Promise<number> } {
  const agent = new WorkspaceAgent({
    runId: RUN_ID,
    token: overrides.token ?? TOKEN,
    controlUrl: control.url,
    socketPath: sandbox.socketPath,
    workspaceDir: sandbox.workspaceDir,
    env: sandbox.env,
    createWorker,
    logger: silentLogger,
    initialBackoffMs: 50,
    maxBackoffMs: 200,
    requestTimeoutMs: 5_000,
    drainTimeoutMs: 5_000,
  });
  return { agent, exitCode: agent.run() };
}

export function startMessage(overrides: Partial<Omit<StartMessage, "type" | "runId">> = {}): Omit<StartMessage, "runId"> {
  return {
    type: "start",
    agent: "claude",
    model: "fake-model",
    systemPrompt: null,
    skills: [{ name: "open-pr-green-ci", required: true, inputs: { Repos: "array", Issue: "number" } }],
    instruction: null,
    resume: null,
    heartbeatSeconds: 1,
    ...overrides,
  };
}
