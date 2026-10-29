import { createHash } from "node:crypto";

const KNOWN_TOKENS =
  /(?<![A-Za-z0-9])(?:(?:github_pat_|gh[psou]_|sk-)[A-Za-z0-9_-]{20,}|(?:AKIA|ASIA)[A-Z0-9]{16}(?![A-Za-z0-9]))/g;

export function redactTokens(bytes: Buffer): Buffer {
  return Buffer.from(bytes.toString("utf8").replace(KNOWN_TOKENS, "<redacted>"), "utf8");
}

export function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

export async function downloadRecord(url: string, expectedSha256: string): Promise<Buffer> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Session record download failed with HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const actual = sha256(bytes);
  if (actual !== expectedSha256) {
    throw new Error(`Session record checksum mismatch: expected ${expectedSha256}, got ${actual}`);
  }
  return bytes;
}

export async function uploadRecord(url: string, headers: Record<string, string>, bytes: Buffer): Promise<void> {
  const response = await fetch(url, { method: "PUT", headers, body: bytes });
  if (!response.ok) throw new Error(`Session record upload failed with HTTP ${response.status}`);
}
