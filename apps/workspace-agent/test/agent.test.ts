import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { promisify } from "node:util";

import { afterEach, describe, expect, it } from "vitest";

import { sha256 } from "../src/session-record.ts";
import { FakeControl } from "../src/testing/fake-control.ts";
import { fakeWorkerFactory, type CommandResult, type FakeScript } from "../src/testing/fake-worker.ts";
import { makeSandbox, RUN_ID, startAgent, startMessage, TOKEN, type Sandbox } from "./helpers.ts";

const execFileAsync = promisify(execFile);

async function runCli(sandbox: Sandbox, args: string[]): Promise<CommandResult> {
  try {
    const { stdout, stderr } = await execFileAsync(join(sandbox.binDir, "sanchoris"), args, { env: sandbox.env });
    return { code: 0, stdout, stderr };
  } catch (error) {
    const failure = error as { code: number; stdout: string; stderr: string };
    return { code: failure.code, stdout: failure.stdout, stderr: failure.stderr };
  }
}

const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

async function setup(script: FakeScript, worker = "home") {
  const control = await FakeControl.listen({
    runId: RUN_ID,
    token: TOKEN,
    onSkillRequest: (request) => {
      if (request.action === "run" && request.name === "open-pr-green-ci") {
        return { ok: true, result: { RunId: 1, Status: "RUNNING" } };
      }
      if (request.action === "status") {
        return { ok: true, result: { RunId: request.skillRunId, Skill: "open-pr-green-ci", Status: "RUNNING" } };
      }
      return { ok: false, error: { code: "rejected", message: "rejected by control" } };
    },
  });
  cleanups.push(() => control.close());
  const sandbox = await makeSandbox();
  cleanups.push(sandbox.cleanup);
  sandbox.env.FAKE_WORKER_HOME = join(sandbox.root, worker);
  const { exitCode } = startAgent(sandbox, control, fakeWorkerFactory(script));
  return { control, sandbox, exitCode };
}

describe("workspace agent with a fake worker", () => {
  it("relays sanchoris skill run to the control plane and feeds the notification back as a user turn", async () => {
    const script: FakeScript = async (text, { exec }) => {
      if (text.startsWith("[sanchoris] Trigger")) {
        const result = await exec("sanchoris", [
          "skill",
          "run",
          "open-pr-green-ci",
          "--input",
          JSON.stringify({ Repos: ["acme/shop-web"], Issue: 35 }),
        ]);
        return `exit ${result.code}: ${result.stdout.trim()}`;
      }
      if (text.startsWith("[sanchoris] Skill open-pr-green-ci (run 1) SUCCEEDED")) return "Task finished.";
      return `unexpected: ${text}`;
    };
    const { control, sandbox, exitCode } = await setup(script);
    await control.waitFor("hello");
    control.send(
      startMessage({
        instruction:
          "[sanchoris] Trigger labeled-issue started this task: Fix flaky checkout test (acme/shop-web#35).\nInstruction: Resolve issue 35 in acme/shop-web.\nSkills: open-pr-green-ci (required). Run it with `sanchoris skill run open-pr-green-ci`.",
      }),
    );
    const ready = await control.waitFor("ready");
    expect(ready.resumed).toBe(false);

    const request = await control.waitFor("skill_request");
    expect(request).toMatchObject({
      action: "run",
      name: "open-pr-green-ci",
      input: { Repos: ["acme/shop-web"], Issue: 35 },
    });

    const firstRecord = await control.waitFor("session_record", (message) => message.turn === 1);
    const firstEnd = await control.waitFor("turn_end", (message) => message.turn === 1);
    expect(firstEnd.result).toBe("completed");
    expect(firstEnd.summary).toBe('exit 0: {"RunId":1,"Status":"RUNNING"}');
    expect(firstEnd.sessionId).toBe(ready.sessionId);
    expect(control.received.indexOf(firstRecord)).toBeLessThan(control.received.indexOf(firstEnd));
    expect(sha256(control.records.get(firstRecord.key) as Buffer)).toBe(firstRecord.sha256);

    const unknown = await runCli(sandbox, ["skill", "run", "deploy-prod"]);
    expect(unknown.code).toBe(1);
    expect(unknown.stderr).toContain("unknown_skill");

    const invalid = await runCli(sandbox, ["skill", "run", "open-pr-green-ci", "--input", '{"Issue":"35"}']);
    expect(invalid.code).toBe(1);
    expect(invalid.stderr).toContain("invalid_input");
    expect(invalid.stderr).toContain("Repos is required");
    expect(invalid.stderr).toContain("Issue must be number, got string");

    const rejected = await runCli(sandbox, ["skill", "continue", "7", "--output", "{}"]);
    expect(rejected.code).toBe(1);
    expect(rejected.stderr).toContain("rejected: rejected by control");

    const status = await runCli(sandbox, ["skill", "status", "1"]);
    expect(status.code).toBe(0);
    expect(JSON.parse(status.stdout)).toEqual({ RunId: 1, Skill: "open-pr-green-ci", Status: "RUNNING" });

    control.send({
      type: "user_message",
      messageId: "m-1",
      text: '[sanchoris] Skill open-pr-green-ci (run 1) SUCCEEDED: {"PullRequests":["acme/shop-web#52"]}',
    });
    const secondEnd = await control.waitFor("turn_end", (message) => message.turn === 2);
    expect(secondEnd.summary).toBe("Task finished.");
    await control.waitFor("session_record", (message) => message.turn === 2);
    await control.waitFor("heartbeat");

    const events = control.received.filter((message) => message.type === "event");
    expect(events.some((event) => event.kind === "tool_use" && event.turn === 1)).toBe(true);

    control.send({ type: "shutdown", reason: "task completed" });
    expect(await exitCode).toBe(0);
  });

  it("ignores a repeated user_message with the same messageId", async () => {
    const { control, exitCode } = await setup(async (text) => `echo ${text}`);
    await control.waitFor("hello");
    control.send(startMessage());
    await control.waitFor("ready");
    control.send({ type: "user_message", messageId: "same", text: "first" });
    control.send({ type: "user_message", messageId: "same", text: "first" });
    control.send({ type: "user_message", messageId: "other", text: "second" });
    const second = await control.waitFor("turn_end", (message) => message.turn === 2);
    expect(second.summary).toBe("echo second");
    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
    expect(control.received.filter((message) => message.type === "turn_end")).toHaveLength(2);
  });

  it("runs exec commands in the workspace in order and stops at the first failure", async () => {
    const { control, sandbox, exitCode } = await setup(async () => "unused");
    await control.waitFor("hello");
    control.send(startMessage());
    await control.waitFor("ready");
    control.send({
      type: "exec",
      requestId: "exec-1",
      purpose: "exec",
      commands: ["printf hello > made.txt", "cat made.txt", "echo oops >&2; exit 3", "touch never.txt"],
    });
    control.send({ type: "exec", requestId: "exec-1", purpose: "exec", commands: ["touch duplicate.txt"] });
    control.send({ type: "exec", requestId: "exec-2", purpose: "git_push", commands: ["pwd"] });
    const failed = await control.waitFor("exec_result", (message) => message.requestId === "exec-1");
    expect(failed.ok).toBe(false);
    expect(failed.results).toEqual([
      { command: "printf hello > made.txt", exitCode: 0, stdout: "", stderr: "" },
      { command: "cat made.txt", exitCode: 0, stdout: "hello", stderr: "" },
      { command: "echo oops >&2; exit 3", exitCode: 3, stdout: "", stderr: "oops\n" },
    ]);
    const succeeded = await control.waitFor("exec_result", (message) => message.requestId === "exec-2");
    expect(succeeded.ok).toBe(true);
    expect(succeeded.results[0]?.stdout.trim()).toBe(sandbox.workspaceDir);
    expect(control.received.indexOf(failed)).toBeLessThan(control.received.indexOf(succeeded));
    expect(existsSync(join(sandbox.workspaceDir, "never.txt"))).toBe(false);
    expect(existsSync(join(sandbox.workspaceDir, "duplicate.txt"))).toBe(false);
    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
    expect(control.received.filter((message) => message.type === "exec_result")).toHaveLength(2);
  });

  it("reconnects after the channel drops and reports that the session has started", async () => {
    const { control, exitCode } = await setup(async (text) => `echo ${text}`);
    await control.waitFor("hello");
    control.send(startMessage());
    const ready = await control.waitFor("ready");
    control.dropConnection();
    await control.waitForConnection(2);
    const hello = await control.waitFor("hello", (message) => message.started);
    expect(hello.sessionId).toBe(ready.sessionId);
    expect(hello.instanceId).toBe((await control.waitFor("hello")).instanceId);
    control.send({ type: "user_message", messageId: "after-reconnect", text: "still there?" });
    const end = await control.waitFor("turn_end");
    expect(end.summary).toBe("echo still there?");
    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
  });

  it("answers sanchoris with not_started before the start message", async () => {
    const { control, sandbox, exitCode } = await setup(async () => "unused");
    await control.waitFor("hello");
    const result = await runCli(sandbox, ["skill", "run", "open-pr-green-ci"]);
    expect(result.code).toBe(1);
    expect(result.stderr).toContain("not_started");
    control.send(startMessage());
    await control.waitFor("ready");
    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
  });

  it("exits with an error when the token is rejected", async () => {
    const control = await FakeControl.listen({
      runId: RUN_ID,
      token: TOKEN,
      onSkillRequest: () => ({ ok: false, error: { code: "unused", message: "unused" } }),
    });
    cleanups.push(() => control.close());
    const sandbox = await makeSandbox();
    cleanups.push(sandbox.cleanup);
    const { exitCode } = startAgent(sandbox, control, fakeWorkerFactory(async () => "unused"), { token: "wrong" });
    expect(await exitCode).toBe(1);
  });

  it("exits with an error on an invalid inbound message", async () => {
    const { control, exitCode } = await setup(async () => "unused");
    await control.waitFor("hello");
    control.sendRaw(JSON.stringify({ type: "start", runId: RUN_ID }));
    expect(await exitCode).toBe(1);
  });
});

describe("session record round trip", () => {
  const rememberScript: FakeScript = async (text, { history }) => {
    const remember = /Remember the code word (\S+?)\.?$/.exec(text);
    if (remember) return "OK";
    if (text.includes("What was the code word?")) {
      for (const line of history) {
        const match = line.role === "user" ? /Remember the code word (\S+?)\.?$/.exec(line.text) : null;
        if (match) return `${match[1]}.`;
      }
      return "I do not know.";
    }
    return "unexpected";
  };

  it("restores the conversation in a new workspace from the uploaded record", async () => {
    const first = await setup(rememberScript, "home-a");
    await first.control.waitFor("hello");
    first.control.send(startMessage({ instruction: "Remember the code word BLUEBERRY-17." }));
    const ready = await first.control.waitFor("ready");
    const record = await first.control.waitFor("session_record", (message) => message.turn === 1);
    await first.control.waitFor("turn_end", (message) => message.turn === 1);
    first.control.send({ type: "shutdown", reason: "idle" });
    expect(await first.exitCode).toBe(0);

    const control = first.control;
    const received = control.received.length;
    const sandbox = await makeSandbox();
    cleanups.push(sandbox.cleanup);
    sandbox.env.FAKE_WORKER_HOME = join(sandbox.root, "home-b");
    const second = { control, exitCode: startAgent(sandbox, control, fakeWorkerFactory(rememberScript)).exitCode };
    await control.waitFor("hello", (message) => control.received.indexOf(message) >= received);
    second.control.send(
      startMessage({
        instruction: "[sanchoris] Message from Mika: What was the code word?",
        resume: {
          sessionId: record.sessionId,
          url: second.control.recordUrl(record.key),
          relativePath: record.relativePath,
          sha256: record.sha256,
          lastTurn: record.turn,
        },
      }),
    );
    const resumed = await second.control.waitFor("ready", (message) => message.resumed);
    expect(resumed.resumed).toBe(true);
    expect(resumed.sessionId).toBe(ready.sessionId);
    const end = await second.control.waitFor("turn_end", (message) => message.turn === 2);
    expect(end.turn).toBe(2);
    expect(end.summary).toBe("BLUEBERRY-17.");
    second.control.send({ type: "shutdown", reason: "done" });
    expect(await second.exitCode).toBe(0);
  });

  it("does not remember without the record", async () => {
    const { control, exitCode } = await setup(rememberScript);
    await control.waitFor("hello");
    control.send(startMessage({ instruction: "[sanchoris] Message from Mika: What was the code word?" }));
    const end = await control.waitFor("turn_end");
    expect(end.summary).toBe("I do not know.");
    control.send({ type: "shutdown", reason: "done" });
    expect(await exitCode).toBe(0);
  });

  it("fails fast when the downloaded record does not match its checksum", async () => {
    const { control, exitCode } = await setup(rememberScript);
    control.records.set("broken", Buffer.from("tampered\n"));
    await control.waitFor("hello");
    control.send(
      startMessage({
        resume: {
          sessionId: "00000000-0000-4000-8000-000000000000",
          url: control.recordUrl("broken"),
          relativePath: "sessions/00000000-0000-4000-8000-000000000000.jsonl",
          sha256: "0".repeat(64),
          lastTurn: 1,
        },
      }),
    );
    expect(await exitCode).toBe(1);
  });
});
