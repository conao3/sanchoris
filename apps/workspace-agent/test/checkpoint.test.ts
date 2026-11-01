import { execFile } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { promisify } from "node:util"

import { afterEach, describe, expect, it } from "vite-plus/test"

import { capture, checkpointRef, diff, repositories, restore } from "../src/checkpoint.ts"

const run = promisify(execFile)

const env = {
  ...process.env,
  GIT_AUTHOR_NAME: "test",
  GIT_AUTHOR_EMAIL: "test@example.com",
  GIT_COMMITTER_NAME: "test",
  GIT_COMMITTER_EMAIL: "test@example.com",
}

async function git(dir: string, ...args: string[]): Promise<string> {
  return (await run("git", ["-C", dir, ...args], { env })).stdout
}

async function repo(dir: string): Promise<void> {
  await mkdir(dir, { recursive: true })
  await git(dir, "init", "--quiet")
  await writeFile(join(dir, "a.txt"), "one\ntwo\n")
  await git(dir, "add", "a.txt")
  await git(dir, "commit", "--quiet", "-m", "init")
}

describe("checkpoints", () => {
  let root = ""

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it("lists the files a turn changed in each repository and leaves the index alone", async () => {
    root = await mkdtemp(join(tmpdir(), "sanchoris-checkpoint-"))
    const web = join(root, "web")
    const api = join(root, "api")
    await repo(web)
    await repo(api)
    await mkdir(join(root, "notes"))
    const dirs = await repositories(root, env)
    expect(dirs).toEqual([api, web])
    const start = checkpointRef(1, "start")
    const end = checkpointRef(1, "end")
    await capture(dirs, start, env)
    await writeFile(join(web, "a.txt"), "one\n2\nthree\n")
    await writeFile(join(web, "b.txt"), "new\n")
    await capture(dirs, end, env)
    const changes = await diff(dirs, start, end, env)
    expect(changes.files).toEqual([
      { repo: "web", path: "a.txt", additions: 2, deletions: 1 },
      { repo: "web", path: "b.txt", additions: 1, deletions: 0 },
    ])
    expect(changes.patch).toContain("+++ b/web/b.txt")
    expect(changes.truncated).toBe(false)
    expect(await git(web, "status", "--porcelain")).toBe(" M a.txt\n?? b.txt\n")
    await restore([web], start, env)
    expect(await readFile(join(web, "a.txt"), "utf8")).toBe("one\ntwo\n")
    expect(await git(web, "status", "--porcelain")).toBe("")
  })

  it("treats a workspace that is itself a repository as the only one", async () => {
    root = await mkdtemp(join(tmpdir(), "sanchoris-checkpoint-"))
    await repo(root)
    expect(await repositories(root, env)).toEqual([root])
  })
})
