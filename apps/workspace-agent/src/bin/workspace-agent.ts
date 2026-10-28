#!/usr/bin/env node
import { type CredentialSource, WorkspaceAgent } from "../agent.ts";
import { parseCheckout } from "../checkout.ts";
import { loopbackProxyPort, parseRelayUrl } from "../egress-forwarder.ts";
import { stderrLogger } from "../log.ts";
import { readSecrets } from "../secrets.ts";
import { requireEnv } from "../worker.ts";
import { createWorker } from "../workers/index.ts";

const PROVIDER_UPSTREAM = "https://api.anthropic.com";

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

const secrets = await readSecrets(process.stdin);
process.stdin.destroy();

const agent = new WorkspaceAgent({
  runId: requireEnv(env, "SANCHORIS_RUN_ID"),
  token: secrets.runToken,
  controlUrl,
  socketPath: requireEnv(env, "SANCHORIS_SOCKET_PATH"),
  workspaceDir: requireEnv(env, "SANCHORIS_WORKSPACE_DIR"),
  credentialSource: credentialSource(),
  egress: secrets.relayUrl === null ? null : { relay: parseRelayUrl(secrets.relayUrl), port: loopbackProxyPort(env.HTTPS_PROXY) },
  providerUpstream: PROVIDER_UPSTREAM,
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
