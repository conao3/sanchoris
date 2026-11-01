import { execFile } from "node:child_process"
import { copyFile, mkdtemp, readdir, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, join } from "node:path"
import { promisify } from "node:util"

const run = promisify(execFile)

const MAX_PATCH_CHARS = 48_000
const REF_PREFIX = "refs/sanchoris/turns"

export interface ChangedFile {
  repo: string
  path: string
  additions: number | null
  deletions: number | null
}

export interface TurnDiff {
  files: ChangedFile[]
  patch: string
  truncated: boolean
}

async function git(dir: string, args: string[], env: NodeJS.ProcessEnv): Promise<string> {
  try {
    const { stdout } = await run("git", ["-C", dir, ...args], {
      env,
      maxBuffer: 64 * 1024 * 1024,
    })
    return stdout
  } catch (error) {
    const stderr = (error as { stderr?: string }).stderr ?? ""
    throw new Error(`git ${args.join(" ")} failed: ${stderr.trim() || (error as Error).message}`, {
      cause: error,
    })
  }
}

async function isRepository(dir: string, env: NodeJS.ProcessEnv): Promise<boolean> {
  try {
    const { stdout } = await run("git", ["-C", dir, "rev-parse", "--show-toplevel"], { env })
    return stdout.trim() === dir
  } catch {
    return false
  }
}

export async function repositories(
  workspaceDir: string,
  env: NodeJS.ProcessEnv,
): Promise<string[]> {
  if (await isRepository(workspaceDir, env)) return [workspaceDir]
  const entries = await readdir(workspaceDir, { withFileTypes: true })
  const dirs: string[] = []
  for (const entry of entries) {
    const dir = join(workspaceDir, entry.name)
    if (entry.isDirectory() && (await isRepository(dir, env))) dirs.push(dir)
  }
  return dirs.toSorted()
}

export function checkpointRef(turn: number, edge: "start" | "end"): string {
  return `${REF_PREFIX}/${turn}/${edge}`
}

async function captureTree(dir: string, env: NodeJS.ProcessEnv): Promise<string> {
  const scratch = await mkdtemp(join(tmpdir(), "sanchoris-index-"))
  const indexEnv = { ...env, GIT_INDEX_FILE: join(scratch, "index") }
  try {
    const index = (
      await git(dir, ["rev-parse", "--path-format=absolute", "--git-path", "index"], env)
    ).trim()
    await copyFile(index, indexEnv.GIT_INDEX_FILE).catch((error: NodeJS.ErrnoException) => {
      if (error.code !== "ENOENT") throw error
    })
    await git(dir, ["add", "--all", "--", "."], indexEnv)
    return (await git(dir, ["write-tree"], indexEnv)).trim()
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
}

export async function capture(
  dirs: readonly string[],
  ref: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  for (const dir of dirs) await git(dir, ["update-ref", ref, await captureTree(dir, env)], env)
}

function count(value: string): number | null {
  return value === "-" ? null : Number(value)
}

export async function diff(
  dirs: readonly string[],
  from: string,
  to: string,
  env: NodeJS.ProcessEnv,
): Promise<TurnDiff> {
  const files: ChangedFile[] = []
  let patch = ""
  let truncated = false
  const prefix = dirs.length > 1
  for (const dir of dirs) {
    const repo = basename(dir)
    const numstat = await git(dir, ["diff", "--numstat", "-z", "--no-renames", from, to], env)
    for (const line of numstat.split("\0")) {
      if (line === "") continue
      const [additions = "0", deletions = "0", path = ""] = line.split("\t")
      files.push({ repo, path, additions: count(additions), deletions: count(deletions) })
    }
    if (truncated) continue
    const options = prefix ? [`--src-prefix=a/${repo}/`, `--dst-prefix=b/${repo}/`] : []
    const text = await git(
      dir,
      ["diff", "--no-color", "--no-renames", "--no-ext-diff", ...options, from, to],
      env,
    )
    if (patch.length + text.length > MAX_PATCH_CHARS) {
      patch += text.slice(0, MAX_PATCH_CHARS - patch.length)
      truncated = true
    } else {
      patch += text
    }
  }
  return { files, patch, truncated }
}

export async function restore(
  dirs: readonly string[],
  ref: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  for (const dir of dirs) {
    const current = await captureTree(dir, env)
    const added = await git(
      dir,
      ["diff", "--name-only", "-z", "--no-renames", "--diff-filter=A", ref, current],
      env,
    )
    for (const path of added.split("\0"))
      if (path !== "") await rm(join(dir, path), { force: true })
    const scratch = await mkdtemp(join(tmpdir(), "sanchoris-index-"))
    const indexEnv = { ...env, GIT_INDEX_FILE: join(scratch, "index") }
    try {
      await git(dir, ["read-tree", ref], indexEnv)
      await git(dir, ["checkout-index", "--all", "--force"], indexEnv)
    } finally {
      await rm(scratch, { recursive: true, force: true })
    }
  }
}
