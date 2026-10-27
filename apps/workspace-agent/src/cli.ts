import { createConnection } from "node:net";

import { SocketResponse, type SocketRequest } from "./protocol.ts";

const USAGE = `Usage:
  sanchoris skill run <name> [--input '<json object>']
  sanchoris skill continue <skill-run-id> [--output '<json>']
  sanchoris skill status <skill-run-id>`;

const RESPONSE_TIMEOUT_MS = 60_000;

export class UsageError extends Error {}

function parseJson(flag: string, raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch (error) {
    throw new UsageError(`${flag} is not valid JSON: ${(error as Error).message}`);
  }
}

function parseSkillRunId(raw: string | undefined): number {
  if (raw === undefined || !/^[1-9][0-9]*$/.test(raw)) throw new UsageError(`Invalid skill run id: ${String(raw)}`);
  return Number(raw);
}

function parseFlags(args: string[], allowed: string[]): Map<string, string> {
  const flags = new Map<string, string>();
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] as string;
    const match = /^(--[a-z]+)(?:=(.*))?$/s.exec(arg);
    if (!match) throw new UsageError(`Unexpected argument: ${arg}`);
    const name = match[1] as string;
    if (!allowed.includes(name)) throw new UsageError(`Unknown option: ${name}`);
    if (flags.has(name)) throw new UsageError(`Duplicate option: ${name}`);
    let value = match[2];
    if (value === undefined) {
      index += 1;
      value = args[index];
      if (value === undefined) throw new UsageError(`${name} needs a value`);
    }
    flags.set(name, value);
  }
  return flags;
}

export function parseArgs(argv: string[]): SocketRequest {
  const [group, action, target, ...rest] = argv;
  if (group !== "skill") throw new UsageError(USAGE);
  switch (action) {
    case "run": {
      if (target === undefined || target.startsWith("--")) throw new UsageError("Missing skill name");
      const input = parseFlags(rest, ["--input"]).get("--input");
      return { action: "run", name: target, input: input === undefined ? {} : parseJson("--input", input) };
    }
    case "continue": {
      const skillRunId = parseSkillRunId(target);
      const output = parseFlags(rest, ["--output"]).get("--output");
      return { action: "continue", skillRunId, output: output === undefined ? null : parseJson("--output", output) };
    }
    case "status": {
      const skillRunId = parseSkillRunId(target);
      parseFlags(rest, []);
      return { action: "status", skillRunId };
    }
    default:
      throw new UsageError(USAGE);
  }
}

function callAgent(socketPath: string, request: SocketRequest): Promise<SocketResponse> {
  return new Promise((resolve, reject) => {
    const socket = createConnection(socketPath);
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error(`The workspace agent did not answer within ${RESPONSE_TIMEOUT_MS} ms`));
    }, RESPONSE_TIMEOUT_MS);
    socket.setEncoding("utf8");
    socket.on("connect", () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on("data", (chunk: string) => {
      buffer += chunk;
    });
    socket.on("error", (error) => {
      clearTimeout(timer);
      reject(new Error(`Cannot reach the workspace agent at ${socketPath}: ${error.message}`));
    });
    socket.on("end", () => {
      clearTimeout(timer);
      try {
        resolve(SocketResponse.parse(JSON.parse(buffer.trim())));
      } catch (error) {
        reject(new Error(`Invalid response from the workspace agent: ${(error as Error).message}`));
      }
    });
  });
}

export async function main(argv: string[], env: NodeJS.ProcessEnv): Promise<number> {
  let request: SocketRequest;
  try {
    request = parseArgs(argv);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    process.stderr.write(`sanchoris: ${error.message}\n`);
    return 2;
  }
  const socketPath = env.SANCHORIS_SOCKET_PATH;
  if (socketPath === undefined || socketPath === "") {
    process.stderr.write("sanchoris: SANCHORIS_SOCKET_PATH is not set\n");
    return 1;
  }
  let response: SocketResponse;
  try {
    response = await callAgent(socketPath, request);
  } catch (error) {
    process.stderr.write(`sanchoris: ${(error as Error).message}\n`);
    return 1;
  }
  if (!response.ok) {
    process.stderr.write(`sanchoris: ${response.error.code}: ${response.error.message}\n`);
    return 1;
  }
  process.stdout.write(`${JSON.stringify(response.result)}\n`);
  return 0;
}
