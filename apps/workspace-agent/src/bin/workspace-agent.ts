#!/usr/bin/env node
import { type CredentialSource, WorkspaceAgent } from "../agent.ts";
import { parseCheckout } from "../checkout.ts";
import { stderrLogger } from "../log.ts";
import { requireEnv } from "../worker.ts";
import { createWorker } from "../workers/index.ts";

const env = process.env;
const controlUrl = requireEnv(env, "SANCHORIS_CONTROL_URL");
if (!/^wss?:\/\//.test(controlUrl)) throw new Error("SANCHORIS_CONTROL_URL must be a ws:// or wss:// URL");

function credentialSource(): CredentialSource {
  const source = requireEnv(env, "SANCHORIS_CREDENTIAL_SOURCE");
  switch (source) {
    case "environment":
      return { kind: "environment" };
    case "channel":
      return { kind: "channel", ...parseCheckout((name) => requireEnv(env, name)) };
    default:
      throw new Error(`SANCHORIS_CREDENTIAL_SOURCE must be environment or channel, not ${source}`);
  }
}

const agent = new WorkspaceAgent({
  runId: requireEnv(env, "SANCHORIS_RUN_ID"),
  token: requireEnv(env, "SANCHORIS_RUN_TOKEN"),
  controlUrl,
  socketPath: requireEnv(env, "SANCHORIS_SOCKET_PATH"),
  workspaceDir: requireEnv(env, "SANCHORIS_WORKSPACE_DIR"),
  credentialSource: credentialSource(),
  env,
  createWorker,
  logger: stderrLogger,
  initialBackoffMs: 500,
  maxBackoffMs: 30_000,
  requestTimeoutMs: 30_000,
  drainTimeoutMs: 10_000,
});

process.exitCode = await agent.run();
process.exit();
