import { execFile } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

import { afterEach, describe, expect, it } from "vite-plus/test";

import { checkout, parseCheckout } from "../src/checkout.ts";

const git = promisify(execFile);

function reader(values: Record<string, string>): (name: string) => string {
  return (name) => {
    const value = values[name];
    if (value === undefined) throw new Error(`missing ${name}`);
    return value;
  };
}

describe("parseCheckout", () => {
  const base = { SANCHORIS_GIT_URL: "https://github.com", SANCHORIS_BRANCH: "sanchoris/task-1" };

  it("reads owner/name repositories and the branch", () => {
    expect(parseCheckout(reader({ ...base, SANCHORIS_REPOSITORIES: "acme/web, acme/api" }))).toEqual({
      gitUrl: "https://github.com",
      branch: "sanchoris/task-1",
      repositories: ["acme/web", "acme/api"],
    });
  });

  it("rejects repository names and branches that git would read as options", () => {
    for (const repo of ["-acme/web", "acme/-web", "acme/--upload-pack=touch", "acme/..", "acme"]) {
      expect(() => parseCheckout(reader({ ...base, SANCHORIS_REPOSITORIES: repo }))).toThrow(/is not owner\/name/);
    }
    expect(() => parseCheckout(reader({ ...base, SANCHORIS_REPOSITORIES: "acme/web", SANCHORIS_BRANCH: "--orphan" }))).toThrow(
      /must not start with -/,
    );
  });
});

describe("checkout", () => {
  let root: string | null = null;

  afterEach(async () => {
    if (root) await rm(root, { recursive: true, force: true });
    root = null;
  });

  it("clones and checks out the remote branch when it exists and creates it otherwise", async () => {
    root = await mkdtemp(join(tmpdir(), "sanchoris-checkout-"));
    const upstream = join(root, "remote", "acme", "web.git");
    await git("git", ["init", "--quiet", "-b", "main", upstream]);
    await git("git", ["-C", upstream, "-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "--quiet", "--allow-empty", "-m", "a"]);
    await git("git", ["-C", upstream, "branch", "sanchoris/task-1"]);
    const workspace = join(root, "workspace");
    await checkout({ gitUrl: join(root, "remote"), branch: "sanchoris/task-1", repositories: ["acme/web"] }, workspace, process.env);
    const { stdout } = await git("git", ["-C", join(workspace, "web"), "branch", "--show-current"]);
    expect(stdout.trim()).toBe("sanchoris/task-1");

    const fresh = join(root, "fresh");
    await checkout({ gitUrl: join(root, "remote"), branch: "sanchoris/task-2", repositories: ["acme/web"] }, fresh, process.env);
    const created = await git("git", ["-C", join(fresh, "web"), "branch", "--show-current"]);
    expect(created.stdout.trim()).toBe("sanchoris/task-2");
  });
});
