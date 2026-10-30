import { execFile } from "node:child_process"
import { join } from "node:path"
import { promisify } from "node:util"

const run = promisify(execFile)

export interface Checkout {
  gitUrl: string
  branch: string
  repositories: string[]
}

function repositoryDir(workspaceDir: string, repo: string): string {
  const match = /^[^/\s-][^/\s]*\/([^/\s-][^/\s]*)$/.exec(repo)
  const name = match?.[1]
  if (name === undefined || name === "." || name === "..")
    throw new Error(`${JSON.stringify(repo)} is not owner/name`)
  return join(workspaceDir, name)
}

async function git(args: string[], env: NodeJS.ProcessEnv): Promise<void> {
  try {
    await run("git", args, { env })
  } catch (error) {
    const stderr = (error as { stderr?: string }).stderr ?? ""
    throw new Error(`git ${args.join(" ")} failed: ${stderr.trim() || (error as Error).message}`)
  }
}

async function hasRef(dir: string, ref: string, env: NodeJS.ProcessEnv): Promise<boolean> {
  try {
    await run("git", ["-C", dir, "rev-parse", "--verify", "--quiet", "--end-of-options", ref], {
      env,
    })
    return true
  } catch {
    return false
  }
}

export async function checkout(
  spec: Checkout,
  workspaceDir: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  for (const repo of spec.repositories) {
    const dir = repositoryDir(workspaceDir, repo)
    await git(["clone", "--quiet", "--", `${spec.gitUrl}/${repo}.git`, dir], env)
    const remote = `refs/remotes/origin/${spec.branch}`
    if (await hasRef(dir, remote, env)) {
      await git(
        ["-C", dir, "checkout", "--quiet", "-B", spec.branch, "--end-of-options", remote, "--"],
        env,
      )
    } else {
      await git(["-C", dir, "checkout", "--quiet", "-b", spec.branch, "--"], env)
    }
  }
}

export function parseCheckout(read: (name: string) => string): Checkout {
  const gitUrl = read("SANCHORIS_GIT_URL").replace(/\/+$/, "")
  if (!/^https?:\/\/[^/]+$/.test(gitUrl))
    throw new Error("SANCHORIS_GIT_URL must be an http(s) origin")
  const repositories = read("SANCHORIS_REPOSITORIES")
    .split(",")
    .map((repo) => repo.trim())
    .filter((repo) => repo !== "")
  for (const repo of repositories) repositoryDir("/", repo)
  const branch = read("SANCHORIS_BRANCH")
  if (branch.startsWith("-"))
    throw new Error(`SANCHORIS_BRANCH ${JSON.stringify(branch)} must not start with -`)
  return { gitUrl, branch, repositories }
}
