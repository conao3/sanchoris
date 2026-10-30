import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import { type CredentialSource, WorkspaceAgent } from "../../apps/workspace-agent/src/agent.ts";
import { stderrLogger } from "../../apps/workspace-agent/src/log.ts";
import { readSecrets } from "../../apps/workspace-agent/src/secrets.ts";
import { writeCliWrapper } from "../../apps/workspace-agent/src/testing/cli-wrapper.ts";
import { fakeWorkerFactory, type CommandResult, type FakeScript } from "../../apps/workspace-agent/src/testing/fake-worker.ts";
import { requireEnv } from "../../apps/workspace-agent/src/worker.ts";

const SKILL = "open-pr-green-ci";

interface Target {
  repo: string;
  dir: string;
  issue: number;
}

function targetOf(text: string): Target | null {
  const match = /\(([\w.-]+\/([\w.-]+))#(\d+)\)/.exec(text);
  if (!match) return null;
  return { repo: match[1] as string, dir: match[2] as string, issue: Number(match[3]) };
}

function check(step: string, result: CommandResult): CommandResult {
  if (result.code !== 0) throw new Error(`${step} failed with ${result.code}: ${result.stderr}`);
  return result;
}

const workspaceDir = requireEnv(process.env, "SANCHORIS_WORKSPACE_DIR");
const home = join(workspaceDir, ".fake-agent");
const binDir = join(home, "bin");
await writeCliWrapper(binDir);
await mkdir(join(home, "worker"), { recursive: true });
const env: NodeJS.ProcessEnv = {
  ...process.env,
  PATH: `${binDir}:${process.env.PATH ?? ""}`,
  FAKE_WORKER_HOME: join(home, "worker"),
};

let attempt = 0;

const script: FakeScript = async (text, { history, exec }) => {
  const first = history.find((line) => line.role === "user" && line.text.startsWith("[sanchoris] Trigger"));
  const target = first ? targetOf(first.text) : null;
  if (!target) return `No task target in: ${first?.text ?? "(no trigger message)"}`;

  const commit = async (reason: string) => {
    attempt += 1;
    await writeFile(join(workspaceDir, target.dir, `attempt-${attempt}.txt`), `${reason} at ${new Date().toISOString()}\n`);
    check("git add", await exec("git", ["-C", target.dir, "add", "."]));
    check(
      "git commit",
      await exec("git", [
        "-C",
        target.dir,
        "-c",
        "user.name=sanchoris e2e",
        "-c",
        "user.email=e2e@sanchoris.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "-m",
        reason,
      ]),
    );
  };

  const runSkill = async () => {
    const input = JSON.stringify({ Repos: [target.repo], Repo: target.repo, Issue: target.issue });
    const result = check("sanchoris skill run", await exec("sanchoris", ["skill", "run", SKILL, "--input", input]));
    return result.stdout.trim();
  };

  if (text.startsWith("[sanchoris] Trigger")) {
    await commit(`Fix issue ${target.issue}`);
    return `Started ${SKILL}: ${await runSkill()}`;
  }
  const needsYou = new RegExp(`^\\[sanchoris\\] Skill ${SKILL} \\(run (\\d+)\\) needs you`).exec(text);
  if (needsYou) {
    await commit(`Fix CI for issue ${target.issue}`);
    const result = check(
      "sanchoris skill continue",
      await exec("sanchoris", ["skill", "continue", needsYou[1] as string, "--output", '{"Fixed":true}']),
    );
    return `Continued: ${result.stdout.trim()}`;
  }
  if (text.startsWith(`[sanchoris] Skill ${SKILL}`) && text.includes(") succeeded.")) return "The pull request is open and green.";
  if (text.startsWith(`[sanchoris] Skill ${SKILL}`) && text.includes(") failed:")) return "The skill failed. Waiting for help.";
  if (text.startsWith("[sanchoris] Message from")) {
    await commit(`Retry issue ${target.issue} after a reply`);
    return `Started ${SKILL} again: ${await runSkill()}`;
  }
  return `Nothing to do for: ${text}`;
};

function environmentSource(): CredentialSource {
  const source = requireEnv(env, "SANCHORIS_CREDENTIAL_SOURCE");
  if (source !== "environment") throw new Error(`the scripted agent takes credentials from the environment, not ${source}`);
  return { kind: "environment" };
}

const secrets = await readSecrets(process.stdin);
process.stdin.destroy();
if (secrets.relayUrl !== null) throw new Error("the scripted agent runs without the egress relay");

const agent = new WorkspaceAgent({
  runId: requireEnv(env, "SANCHORIS_RUN_ID"),
  bootstrapToken: secrets.bootstrapToken,
  controlUrl: requireEnv(env, "SANCHORIS_CONTROL_URL"),
  socketPath: requireEnv(env, "SANCHORIS_SOCKET_PATH"),
  workspaceDir,
  credentialSource: environmentSource(),
  egress: null,
  providerUpstream: "https://api.anthropic.com",
  env,
  createWorker: fakeWorkerFactory(script),
  logger: stderrLogger,
  initialBackoffMs: 200,
  maxBackoffMs: 2_000,
  requestTimeoutMs: 30_000,
  drainTimeoutMs: 10_000,
  stopTurnTimeoutMs: 5_000,
});

process.exitCode = await agent.run();
process.exit();
