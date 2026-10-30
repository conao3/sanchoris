import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { type AgentConfig, type CredentialSource, WorkspaceAgent } from "../src/agent.ts"
import type { Logger } from "../src/log.ts"
import type { StartMessage } from "../src/protocol.ts"
import { writeCliWrapper } from "../src/testing/cli-wrapper.ts"
import type { FakeControl } from "../src/testing/fake-control.ts"
import type { WorkerFactory } from "../src/worker.ts"

export const RUN_ID = "run-1"
export const BOOTSTRAP_TOKEN = "test-bootstrap-token"
export const NEXT_BOOTSTRAP_TOKEN = "test-bootstrap-token-2"
export const RUN_TOKEN = "test-run-token"
export const CONTROL_TOKENS = {
  bootstrapTokens: [BOOTSTRAP_TOKEN, NEXT_BOOTSTRAP_TOKEN],
  runToken: RUN_TOKEN,
}

export interface Sandbox {
  root: string
  binDir: string
  workspaceDir: string
  socketPath: string
  env: NodeJS.ProcessEnv
  cleanup: () => Promise<void>
}

export async function makeSandbox(extraEnv: NodeJS.ProcessEnv = {}): Promise<Sandbox> {
  const root = await mkdtemp(join(tmpdir(), "sanchoris-"))
  const binDir = join(root, "bin")
  const workspaceDir = join(root, "workspace")
  await writeCliWrapper(binDir)
  await mkdir(workspaceDir)
  const socketPath = join(root, "agent.sock")
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
  }
}

export const silentLogger: Logger = () => {}

export function startAgent(
  sandbox: Sandbox,
  control: FakeControl,
  createWorker: WorkerFactory,
  overrides: {
    bootstrapToken?: string
    credentialSource?: CredentialSource
    egress?: AgentConfig["egress"]
    providerUpstream?: string
  } = {},
): { agent: WorkspaceAgent; exitCode: Promise<number> } {
  const agent = new WorkspaceAgent({
    runId: RUN_ID,
    bootstrapToken: overrides.bootstrapToken ?? BOOTSTRAP_TOKEN,
    controlUrl: control.url,
    socketPath: sandbox.socketPath,
    workspaceDir: sandbox.workspaceDir,
    credentialSource: overrides.credentialSource ?? { kind: "environment" },
    egress: overrides.egress ?? null,
    providerUpstream: overrides.providerUpstream ?? "http://127.0.0.1:9",
    env: sandbox.env,
    createWorker,
    logger: silentLogger,
    initialBackoffMs: 50,
    maxBackoffMs: 200,
    requestTimeoutMs: 5_000,
    drainTimeoutMs: 5_000,
    stopTurnTimeoutMs: 5_000,
  })
  return { agent, exitCode: agent.run() }
}

export function startMessage(
  overrides: Partial<Omit<StartMessage, "type" | "runId">> = {},
): Omit<StartMessage, "runId"> {
  return {
    type: "start",
    agent: "claude",
    model: "fake-model",
    systemPrompt: null,
    skills: [
      { name: "open-pr-green-ci", required: true, inputs: { Repos: "array", Issue: "number" } },
    ],
    instruction: null,
    resume: null,
    heartbeatSeconds: 1,
    ...overrides,
  }
}
