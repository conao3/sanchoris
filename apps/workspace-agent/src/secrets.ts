import { z } from "zod";

const AgentSecrets = z
  .object({
    runToken: z.string().min(1),
    relayUrl: z.string().min(1).optional(),
  })
  .strict();

export interface AgentSecrets {
  runToken: string;
  relayUrl: string | null;
}

export function parseSecrets(text: string): AgentSecrets {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`The agent's secrets on stdin are not JSON: ${(error as Error).message}`);
  }
  const parsed = AgentSecrets.parse(value);
  return { runToken: parsed.runToken, relayUrl: parsed.relayUrl ?? null };
}

export async function readSecrets(stream: NodeJS.ReadableStream): Promise<AgentSecrets> {
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  return parseSecrets(Buffer.concat(chunks).toString("utf8"));
}
