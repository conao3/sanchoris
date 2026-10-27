import { spawn } from "node:child_process";

import type { ExecCommandResult } from "./protocol.ts";

const MAX_OUTPUT_CHARS = 16_000;

export interface ExecOutcome {
  ok: boolean;
  results: ExecCommandResult[];
}

function tail(text: string): string {
  return text.length > MAX_OUTPUT_CHARS ? text.slice(text.length - MAX_OUTPUT_CHARS) : text;
}

function runCommand(command: string, cwd: string, env: NodeJS.ProcessEnv): Promise<ExecCommandResult> {
  return new Promise((resolve) => {
    const child = spawn("/bin/sh", ["-c", command], { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout = tail(stdout + chunk);
    });
    child.stderr.on("data", (chunk: string) => {
      stderr = tail(stderr + chunk);
    });
    child.on("error", (error) => {
      resolve({ command, exitCode: 127, stdout, stderr: tail(stderr + error.message) });
    });
    child.on("close", (code, signal) => {
      resolve({ command, exitCode: code ?? (signal ? 128 : 1), stdout, stderr });
    });
  });
}

export async function runCommands(commands: readonly string[], cwd: string, env: NodeJS.ProcessEnv): Promise<ExecOutcome> {
  const results: ExecCommandResult[] = [];
  for (const command of commands) {
    const result = await runCommand(command, cwd, env);
    results.push(result);
    if (result.exitCode !== 0) return { ok: false, results };
  }
  return { ok: true, results };
}
