#!/usr/bin/env node
import { connect } from "node:net";

import { createRelay, hostList, resolveHost } from "../relay.ts";

function requireEnv(name: string): string {
  const value = process.env[name];
  if (value === undefined || value === "") throw new Error(`${name} must be set`);
  return value;
}

const port = Number(requireEnv("SANCHORIS_RELAY_PORT"));
if (!Number.isInteger(port) || port <= 0 || port > 65535) throw new Error("SANCHORIS_RELAY_PORT must be a TCP port");
const signingKey = Buffer.from(requireEnv("RELAY_TOKEN_SIGNING_KEY"), "utf8");
if (signingKey.length < 32) throw new Error("RELAY_TOKEN_SIGNING_KEY must be at least 32 bytes");

const relay = createRelay({
  signingKey,
  platformHosts: hostList(requireEnv("SANCHORIS_RELAY_PLATFORM_HOSTS")),
  denyHosts: hostList(requireEnv("SANCHORIS_RELAY_DENY_HOSTS")),
  log: (entry) => process.stdout.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`),
  resolve: resolveHost,
  connect: (port, address) => connect(port, address),
  now: Date.now,
});

relay.listen(port, () => process.stdout.write(`${JSON.stringify({ event: "listening", port })}\n`));
for (const signal of ["SIGTERM", "SIGINT"] as const) process.on(signal, () => relay.close(() => process.exit(0)));
