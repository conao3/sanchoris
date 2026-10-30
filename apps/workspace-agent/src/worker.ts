import type { EventKind, RateLimitWindow, TurnError, TurnUsage } from "./protocol.ts"

export interface WorkerTurnResult {
  result: "completed" | "failed" | "interrupted"
  summary: string
  error: TurnError | null
  usage: TurnUsage
  totalCostUsd: number | null
  durationMs: number
}

export interface WorkerRateLimit {
  status: string
  resetsAt: number | null
  rateLimitType: string | null
  windows: Record<string, RateLimitWindow>
}

interface WorkerHandlers {
  onEvent: (kind: EventKind, data: Record<string, unknown>) => void
  onTurnEnd: (result: WorkerTurnResult) => void
  onRateLimit: (rateLimit: WorkerRateLimit) => void
  onFatal: (error: Error) => void
}

export interface WorkerRecord {
  relativePath: string
  bytes: Buffer
}

export interface RestoredRecord {
  sessionId: string
  relativePath: string
  bytes: Buffer
}

export interface Worker {
  start(restore: RestoredRecord | null): Promise<{ sessionId: string }>
  send(text: string): void
  interrupt(): Promise<void>
  readRecord(): Promise<WorkerRecord>
  close(): Promise<void>
}

export interface WorkerContext {
  agent: "claude" | "codex"
  model: string
  systemPrompt: string | null
  workspaceDir: string
  env: NodeJS.ProcessEnv
  handlers: WorkerHandlers
}

export type WorkerFactory = (context: WorkerContext) => Worker

export const emptyUsage = (): TurnUsage => ({
  inputTokens: 0,
  outputTokens: 0,
  cacheReadInputTokens: 0,
  cacheCreationInputTokens: 0,
})

export function requireEnv(env: NodeJS.ProcessEnv, name: string): string {
  const value = env[name]
  if (value === undefined || value === "")
    throw new Error(`Missing required environment variable ${name}`)
  return value
}
