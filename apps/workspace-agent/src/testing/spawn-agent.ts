import { spawn, type ChildProcess } from "node:child_process"
import { resolve } from "node:path"

const agentBin = resolve(import.meta.dirname, "../bin/workspace-agent.ts")

export interface SpawnedAgent {
  child: ChildProcess
  exitCode: Promise<number>
}

export function spawnAgent(
  env: NodeJS.ProcessEnv,
  bootstrapToken: string,
  onStderr: (line: string) => void,
): SpawnedAgent {
  const child = spawn(process.execPath, ["--no-warnings", agentBin], {
    env,
    stdio: ["pipe", "inherit", "pipe"],
  })
  child.stdin?.end(JSON.stringify({ bootstrapToken }))
  child.stderr?.setEncoding("utf8")
  child.stderr?.on("data", (data: string) => {
    for (const line of data.split("\n")) if (line !== "") onStderr(line)
  })
  const exitCode = new Promise<number>((resolveExit) => {
    child.on("exit", (code) => resolveExit(code ?? 1))
  })
  return { child, exitCode }
}
