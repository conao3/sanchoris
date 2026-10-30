import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { afterAll, describe, expect, it } from "vite-plus/test"

import { FakeControl } from "../../src/testing/fake-control.ts"
import { spawnAgent } from "../../src/testing/spawn-agent.ts"
import { claudeProjectDirName } from "../../src/workers/claude.ts"
import { requireEnv } from "../../src/worker.ts"

const MODEL = requireEnv(process.env, "SANCHORIS_SMOKE_MODEL")
const CLAUDE_CONFIG_DIR = requireEnv(process.env, "CLAUDE_CONFIG_DIR")

function workerEnv(): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {}
  for (const [name, value] of Object.entries(process.env)) {
    if (
      name === "CLAUDECODE" ||
      name.startsWith("CLAUDE_CODE_") ||
      name === "CLAUDE_AGENT_SDK_VERSION"
    )
      continue
    env[name] = value
  }
  return env
}

const cleanups: Array<() => Promise<void>> = []

afterAll(async () => {
  for (const cleanup of cleanups.reverse()) await cleanup()
})

async function workspace(label: string): Promise<{ root: string; workspaceDir: string }> {
  const root = await mkdtemp(join(tmpdir(), `sanchoris-smoke-${label}-`))
  const workspaceDir = join(root, "workspace")
  await mkdir(workspaceDir)
  const projectDir = join(CLAUDE_CONFIG_DIR, "projects", claudeProjectDirName(workspaceDir))
  cleanups.push(() => rm(root, { recursive: true, force: true }))
  cleanups.push(() => rm(projectDir, { recursive: true, force: true }))
  return { root, workspaceDir }
}

async function run(label: string, control: FakeControl, root: string, workspaceDir: string) {
  return spawnAgent(
    {
      ...workerEnv(),
      SANCHORIS_RUN_ID: "smoke-run",
      SANCHORIS_CONTROL_URL: control.url,
      SANCHORIS_SOCKET_PATH: join(root, "agent.sock"),
      SANCHORIS_WORKSPACE_DIR: workspaceDir,
      SANCHORIS_CREDENTIAL_SOURCE: "environment",
    },
    `smoke-bootstrap-${label}`,
    (line) => process.stderr.write(`[${label}] ${line}\n`),
  )
}

describe("ClaudeWorker smoke", () => {
  it("answers a trivial instruction and resumes in a new workspace from the uploaded record", async () => {
    const control = await FakeControl.listen({
      runId: "smoke-run",
      bootstrapTokens: ["smoke-bootstrap-a", "smoke-bootstrap-b"],
      runToken: "smoke-run-token",
      onSkillRequest: () => ({ ok: false, error: { code: "unused", message: "unused" } }),
    })
    cleanups.push(() => control.close())

    const first = await workspace("a")
    const agentA = await run("a", control, first.root, first.workspaceDir)
    await control.waitFor("hello", () => true, 30_000)
    control.send({
      type: "start",
      agent: "claude",
      model: MODEL,
      systemPrompt: null,
      skills: [],
      instruction: "Remember the code word BLUEBERRY-17. Reply with OK and nothing else.",
      resume: null,
      heartbeatSeconds: 30,
    })
    const ready = await control.waitFor("ready", () => true, 60_000)
    const firstEnd = await control.waitFor("turn_end", () => true, 120_000)
    expect(firstEnd.result).toBe("completed")
    expect(firstEnd.summary).toMatch(/OK/)
    const record = await control.waitFor("session_record", (message) => message.turn === 1)
    process.stderr.write(`first turn: ${JSON.stringify(firstEnd)}\n`)
    control.send({ type: "shutdown", reason: "idle" })
    expect(await agentA.exitCode).toBe(0)
    await rm(join(CLAUDE_CONFIG_DIR, "projects", claudeProjectDirName(first.workspaceDir)), {
      recursive: true,
      force: true,
    })

    const received = control.received.length
    const second = await workspace("b")
    const agentB = await run("b", control, second.root, second.workspaceDir)
    await control.waitFor(
      "hello",
      (message) => control.received.indexOf(message) >= received,
      30_000,
    )
    control.send({
      type: "start",
      agent: "claude",
      model: MODEL,
      systemPrompt: null,
      skills: [],
      instruction:
        "[sanchoris] Message from Mika: What was the code word? Reply with the code word only.",
      resume: {
        sessionId: record.sessionId,
        url: control.recordUrl(record.key),
        relativePath: record.relativePath,
        sha256: record.sha256,
        lastTurn: record.turn,
      },
      heartbeatSeconds: 30,
    })
    const resumed = await control.waitFor("ready", (message) => message.resumed, 60_000)
    expect(resumed.sessionId).toBe(ready.sessionId)
    const secondEnd = await control.waitFor("turn_end", (message) => message.turn === 2, 120_000)
    process.stderr.write(`resumed turn: ${JSON.stringify(secondEnd)}\n`)
    expect(secondEnd.result).toBe("completed")
    expect(secondEnd.summary).toContain("BLUEBERRY-17")
    control.send({ type: "shutdown", reason: "done" })
    expect(await agentB.exitCode).toBe(0)
  })
})
