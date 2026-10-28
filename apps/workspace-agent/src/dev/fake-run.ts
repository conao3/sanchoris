import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";

import { WorkerAgent } from "../protocol.ts";
import { writeCliWrapper } from "../testing/cli-wrapper.ts";
import { FakeControl } from "../testing/fake-control.ts";
import { spawnAgent } from "../testing/spawn-agent.ts";

const USAGE = `Usage: pnpm --filter workspace-agent dev:fake -- --agent <claude|codex> --model <model> [--skill <name>] [--notify-after-ms <ms>] <instruction>

Runs the workspace agent against a local fake control plane. Skill requests are accepted as RUNNING,
and each accepted run is reported back as SUCCEEDED after --notify-after-ms. The run is shut down
after a turn ends with every skill run reported and every notification answered.`;

const { values, positionals } = parseArgs({
  args: process.argv.slice(2).filter((arg) => arg !== "--"),
  options: {
    agent: { type: "string" },
    model: { type: "string" },
    skill: { type: "string", multiple: true },
    "notify-after-ms": { type: "string" },
  },
  allowPositionals: true,
});

const instruction = positionals.join(" ");
if (values.agent === undefined || values.model === undefined || instruction === "") {
  process.stderr.write(`${USAGE}\n`);
  process.exit(2);
}
const agent = WorkerAgent.parse(values.agent);
const notifyAfterMs = Number(values["notify-after-ms"] ?? "3000");

const runId = "dev-run";
const token = "dev-token";
let nextSkillRunId = 1;
const outstanding = new Set<number>();
let expectedTurns = 1;
let endedTurns = 0;

const control = await FakeControl.listen({
  runId,
  token,
  onSkillRequest: (request) => {
    if (request.action !== "run") return { ok: true, result: { RunId: request.skillRunId, Status: "RUNNING" } };
    const id = nextSkillRunId;
    nextSkillRunId += 1;
    outstanding.add(id);
    setTimeout(() => {
      outstanding.delete(id);
      expectedTurns += 1;
      control.send({
        type: "user_message",
        messageId: `skill-${id}`,
        text: `[sanchoris] Skill ${request.name} (run ${id}) SUCCEEDED: {}`,
      });
    }, notifyAfterMs);
    return { ok: true, result: { RunId: id, Status: "RUNNING" } };
  },
});

const root = await mkdtemp(join(tmpdir(), "sanchoris-dev-"));
const workspaceDir = join(root, "workspace");
await mkdir(workspaceDir);
await writeCliWrapper(join(root, "bin"));

const spawned = spawnAgent(
  {
    ...process.env,
    PATH: `${join(root, "bin")}:${process.env.PATH ?? ""}`,
    SANCHORIS_RUN_ID: runId,
    SANCHORIS_CONTROL_URL: control.url,
    SANCHORIS_SOCKET_PATH: join(root, "agent.sock"),
    SANCHORIS_WORKSPACE_DIR: workspaceDir,
  },
  token,
  (line) => process.stderr.write(`agent: ${line}\n`),
);

const print = (message: unknown) => process.stdout.write(`control <- ${JSON.stringify(message)}\n`);

await control.waitFor("hello").then(print);
control.send({
  type: "start",
  agent,
  model: values.model,
  systemPrompt: null,
  skills: (values.skill ?? []).map((name) => ({ name, required: true, inputs: {} })),
  instruction,
  resume: null,
  heartbeatSeconds: 30,
});

let printed = 1;
const timer = setInterval(() => {
  for (const message of control.received.slice(printed)) {
    print(message);
    if (message.type === "turn_end") endedTurns += 1;
    if (message.type === "turn_end" && outstanding.size === 0 && endedTurns >= expectedTurns) {
      control.send({ type: "shutdown", reason: "dev run finished" });
    }
  }
  printed = control.received.length;
}, 100);

const code = await spawned.exitCode;
clearInterval(timer);
for (const message of control.received.slice(printed)) print(message);
await control.close();
process.stdout.write(`workspace agent exited with ${code}\n`);
process.exit(code);
