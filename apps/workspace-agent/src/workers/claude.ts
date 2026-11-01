import { randomUUID } from "node:crypto"
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises"
import { join, relative } from "node:path"

import {
  query,
  type PermissionMode,
  type PermissionResult,
  type PermissionUpdate,
  type Query,
  type SDKMessage,
  type SDKRateLimitInfo,
  type SDKResultMessage,
  type SDKUserMessage,
} from "@anthropic-ai/claude-agent-sdk"

import { AsyncQueue } from "../async-queue.ts"
import type {
  ApprovalRequest,
  PlanStep,
  RateLimitWindow,
  SessionModes,
  UserInputAnswers,
  UserInputQuestion,
} from "../protocol.ts"
import {
  requireEnv,
  type RestoredRecord,
  type Worker,
  type WorkerContext,
  type WorkerMessage,
  type WorkerRateLimit,
  type WorkerRecord,
  type WorkerTurnResult,
} from "../worker.ts"

const MAX_TOOL_RESULT_CHARS = 4000

type UnifiedWindow = { utilization?: number; resetsAt?: number; status?: string }
type RateLimitInfoWithWindows = SDKRateLimitInfo & {
  unifiedWindows?: Record<string, UnifiedWindow>
}

export function claudeProjectDirName(cwd: string): string {
  return cwd.replace(/[^a-zA-Z0-9]/g, "-")
}

function toWindow(window: UnifiedWindow): RateLimitWindow {
  return {
    utilization: window.utilization ?? null,
    resetsAt: window.resetsAt ?? null,
    status: window.status ?? null,
  }
}

export function claudeRateLimit(info: RateLimitInfoWithWindows): WorkerRateLimit {
  const windows: Record<string, RateLimitWindow> = {}
  if (info.unifiedWindows) {
    for (const [name, window] of Object.entries(info.unifiedWindows))
      windows[name] = toWindow(window)
  } else if (info.rateLimitType) {
    windows[info.rateLimitType] = toWindow(info)
  }
  return {
    status: info.status,
    resetsAt: info.resetsAt ?? null,
    rateLimitType: info.rateLimitType ?? null,
    windows,
  }
}

interface ClaudeQuestion {
  question?: unknown
  header?: unknown
  multiSelect?: unknown
  options?: { label?: unknown; description?: unknown }[]
}

export function claudeQuestions(input: Record<string, unknown>): UserInputQuestion[] {
  const questions = input.questions
  if (!Array.isArray(questions) || questions.length === 0)
    throw new Error("AskUserQuestion has no questions")
  return (questions as ClaudeQuestion[]).map((question, index) => {
    if (typeof question.question !== "string" || question.question === "")
      throw new Error(`AskUserQuestion question ${index + 1} has no text`)
    return {
      id: question.question,
      header: typeof question.header === "string" ? question.header : "",
      question: question.question,
      multiSelect: question.multiSelect === true,
      options: (question.options ?? []).flatMap((option) =>
        typeof option.label === "string" && option.label !== ""
          ? [
              {
                label: option.label,
                description: typeof option.description === "string" ? option.description : "",
              },
            ]
          : [],
      ),
    }
  })
}

export function claudeAnswers(
  questions: UserInputQuestion[],
  answers: UserInputAnswers,
): Record<string, string> {
  return Object.fromEntries(
    questions.map((question) => [question.id, (answers[question.id] ?? []).join(", ")]),
  )
}

const PLAN_CAPTURED =
  "The client captured your proposed plan. Stop here and wait for the user's feedback or implementation request in a later turn."

export function claudePermissionMode(modes: SessionModes): PermissionMode {
  if (modes.interactionMode === "plan") return "plan"
  switch (modes.runtimeMode) {
    case "approval-required":
      return "default"
    case "auto-accept-edits":
      return "acceptEdits"
    case "full-access":
      return "bypassPermissions"
  }
}

const EDIT_TOOLS = new Set(["Edit", "Write", "MultiEdit", "NotebookEdit"])

export function claudeApproval(
  toolName: string,
  input: Record<string, unknown>,
  reason: string | undefined,
): ApprovalRequest {
  const field = (name: string) => (typeof input[name] === "string" ? input[name] : "")
  if (toolName === "Bash")
    return { kind: "command", toolName, detail: field("command"), reason: reason ?? null, input }
  if (EDIT_TOOLS.has(toolName))
    return {
      kind: "file-change",
      toolName,
      detail: field("file_path") || field("notebook_path"),
      reason: reason ?? null,
      input,
    }
  return { kind: "tool", toolName, detail: JSON.stringify(input), reason: reason ?? null, input }
}

const TODO_STATUS: Record<string, PlanStep["status"]> = {
  pending: "pending",
  in_progress: "running",
  completed: "completed",
}

export function claudeTodoSteps(input: Record<string, unknown>): PlanStep[] {
  const todos = Array.isArray(input.todos)
    ? (input.todos as { content?: unknown; status?: unknown }[])
    : []
  return todos.flatMap((todo) =>
    typeof todo.content === "string" && todo.content !== ""
      ? [{ text: todo.content, status: TODO_STATUS[String(todo.status)] ?? "pending" }]
      : [],
  )
}

function textOf(content: unknown): string {
  if (typeof content === "string") return content
  if (Array.isArray(content)) {
    return content
      .map((block: { type?: string; text?: string }) =>
        block.type === "text" ? (block.text ?? "") : "",
      )
      .join("")
  }
  return ""
}

export class ClaudeWorker implements Worker {
  private readonly context: WorkerContext
  private readonly configDir: string
  private readonly executable: string
  private readonly input = new AsyncQueue<SDKUserMessage>()
  private session: Query | null = null
  private sessionId: string | null = null
  private interrupting = false
  private closing = false
  private rejectedRateLimit: SDKRateLimitInfo | null = null
  private streaming: { kind: "text" | "thinking"; text: string } | null = null
  private modes: SessionModes

  constructor(context: WorkerContext) {
    this.context = context
    this.configDir = requireEnv(context.env, "CLAUDE_CONFIG_DIR")
    this.executable = requireEnv(context.env, "SANCHORIS_CLAUDE_PATH")
    this.modes = context.modes
  }

  async start(restore: RestoredRecord | null): Promise<{ sessionId: string }> {
    const sessionId = restore?.sessionId ?? randomUUID()
    if (restore) {
      const dir = join(this.configDir, "projects", claudeProjectDirName(this.context.workspaceDir))
      await mkdir(dir, { recursive: true })
      await writeFile(join(dir, `${sessionId}.jsonl`), restore.bytes)
    }
    this.sessionId = sessionId
    this.session = query({
      prompt: this.input,
      options: {
        cwd: this.context.workspaceDir,
        model: this.context.model,
        env: this.context.env,
        pathToClaudeCodeExecutable: this.executable,
        ...(restore ? { resume: sessionId } : { sessionId }),
        permissionMode: claudePermissionMode(this.modes),
        allowDangerouslySkipPermissions: true,
        canUseTool: (toolName, input, options) => this.canUseTool(toolName, input, options),
        thinking: { type: "adaptive", display: "summarized" },
        includePartialMessages: true,
        settings: { showThinkingSummaries: true },
        settingSources: ["user", "project", "local"],
        systemPrompt:
          this.context.systemPrompt === null
            ? { type: "preset", preset: "claude_code" }
            : { type: "preset", preset: "claude_code", append: this.context.systemPrompt },
        stderr: (data) => {
          for (const line of data.split("\n"))
            if (line !== "") this.context.handlers.onEvent("log", { line })
        },
      },
    })
    void this.consume(this.session)
    return { sessionId }
  }

  send({ text, steer, images }: WorkerMessage): void {
    this.input.push({
      type: "user",
      message: {
        role: "user",
        content: [
          ...images.map((image) => ({
            type: "image" as const,
            source: {
              type: "base64" as const,
              media_type: image.mimeType as "image/png" | "image/jpeg" | "image/gif" | "image/webp",
              data: image.data,
            },
          })),
          { type: "text" as const, text },
        ],
      },
      parent_tool_use_id: null,
      ...(steer ? { priority: "now" as const } : {}),
    })
  }

  async setModes(modes: SessionModes): Promise<void> {
    if (!this.session) throw new Error("Claude session is not running")
    if (claudePermissionMode(modes) !== claudePermissionMode(this.modes))
      await this.session.setPermissionMode(claudePermissionMode(modes))
    this.modes = modes
  }

  async interrupt(): Promise<void> {
    if (!this.session) throw new Error("Claude session is not running")
    this.interrupting = true
    await this.session.interrupt()
  }

  async readRecord(): Promise<WorkerRecord> {
    const sessionId = this.sessionId
    if (!sessionId) throw new Error("Claude session is not running")
    const projects = join(this.configDir, "projects")
    const dirs = await readdir(projects)
    for (const dir of dirs) {
      const path = join(projects, dir, `${sessionId}.jsonl`)
      const bytes = await readFile(path).catch((error: NodeJS.ErrnoException) => {
        if (error.code === "ENOENT") return null
        throw error
      })
      if (bytes) return { relativePath: relative(this.configDir, path), bytes }
    }
    throw new Error(`Claude session record ${sessionId}.jsonl was not found under ${projects}`)
  }

  async close(): Promise<void> {
    this.closing = true
    this.input.end()
    this.session?.close()
  }

  private async canUseTool(
    toolName: string,
    input: Record<string, unknown>,
    options: { signal: AbortSignal; suggestions?: PermissionUpdate[]; decisionReason?: string },
  ): Promise<PermissionResult> {
    if (toolName === "AskUserQuestion") return this.askQuestions(input, options.signal)
    if (toolName === "ExitPlanMode") {
      if (typeof input.plan === "string" && input.plan.trim() !== "")
        this.context.handlers.onEvent("proposed_plan", { markdown: input.plan })
      return { behavior: "deny", message: PLAN_CAPTURED }
    }
    if (claudePermissionMode(this.modes) === "bypassPermissions")
      return { behavior: "allow", updatedInput: input }
    if (this.modes.interactionMode === "plan" && this.modes.runtimeMode === "full-access")
      return {
        behavior: "deny",
        message:
          "This tool requires approval in Plan mode, but approvals are disabled for this thread.",
      }
    const decision = await this.context.handlers.onApproval(
      claudeApproval(toolName, input, options.decisionReason),
      options.signal,
    )
    switch (decision) {
      case "accept":
        return { behavior: "allow", updatedInput: input }
      case "acceptForSession":
        return {
          behavior: "allow",
          updatedInput: input,
          updatedPermissions: (options.suggestions ?? []).map((update) => ({
            ...update,
            destination: "session",
          })),
        }
      case "decline":
        return { behavior: "deny", message: "User declined tool execution." }
      case "cancel":
        this.interrupting = true
        return { behavior: "deny", message: "User cancelled tool execution.", interrupt: true }
    }
  }

  private async askQuestions(
    input: Record<string, unknown>,
    signal: AbortSignal,
  ): Promise<PermissionResult> {
    const questions = claudeQuestions(input)
    try {
      const answers = await this.context.handlers.onUserInput(questions, signal)
      return {
        behavior: "allow",
        updatedInput: { ...input, answers: claudeAnswers(questions, answers) },
      }
    } catch (error) {
      return { behavior: "deny", message: (error as Error).message }
    }
  }

  private async consume(session: Query): Promise<void> {
    try {
      for await (const message of session) this.handle(message)
      if (!this.closing)
        this.context.handlers.onFatal(new Error("Claude session ended unexpectedly"))
    } catch (error) {
      if (!this.closing) this.context.handlers.onFatal(error as Error)
    }
  }

  private handle(message: SDKMessage): void {
    const { onEvent, onRateLimit, onTurnEnd } = this.context.handlers
    switch (message.type) {
      case "system":
        onEvent("system", { subtype: message.subtype })
        return
      case "assistant":
        if (message.parent_tool_use_id !== null) return
        for (const block of message.message.content) {
          if (block.type === "thinking") {
            if (block.thinking !== "") onEvent("thinking", { text: block.thinking })
          } else if (block.type === "text") {
            onEvent("text", { text: block.text })
          } else if (block.type === "tool_use") {
            onEvent("tool_use", { id: block.id, name: block.name, input: block.input })
            if (block.name === "TodoWrite")
              onEvent("plan", {
                steps: claudeTodoSteps(block.input as Record<string, unknown>),
                explanation: null,
              })
          }
        }
        return
      case "user": {
        if (message.parent_tool_use_id !== null) return
        const content = message.message.content
        if (!Array.isArray(content)) return
        for (const block of content) {
          if (block.type !== "tool_result") continue
          onEvent("tool_result", {
            toolUseId: block.tool_use_id,
            isError: block.is_error ?? false,
            content: textOf(block.content).slice(0, MAX_TOOL_RESULT_CHARS),
          })
        }
        return
      }
      case "stream_event": {
        if (message.parent_tool_use_id !== null) return
        const event = message.event
        if (event.type === "content_block_start") {
          const type = event.content_block.type
          this.streaming = type === "text" || type === "thinking" ? { kind: type, text: "" } : null
        } else if (event.type === "content_block_delta" && this.streaming) {
          const delta = event.delta
          if (delta.type === "text_delta") this.streaming.text += delta.text
          else if (delta.type === "thinking_delta") this.streaming.text += delta.thinking
          else return
          this.context.handlers.onPartial(this.streaming.kind, this.streaming.text)
        }
        return
      }
      case "rate_limit_event":
        if (message.rate_limit_info.status === "rejected")
          this.rejectedRateLimit = message.rate_limit_info
        onRateLimit(claudeRateLimit(message.rate_limit_info))
        return
      case "result":
        onTurnEnd(this.turnResult(message))
        return
      default:
        return
    }
  }

  private turnResult(message: SDKResultMessage): WorkerTurnResult {
    const failed = message.subtype !== "success" || message.is_error
    let summary = message.subtype === "success" ? message.result : message.errors.join("\n")
    const rejected = this.rejectedRateLimit
    const interrupted = this.interrupting
    this.rejectedRateLimit = null
    this.interrupting = false
    let result: WorkerTurnResult["result"] = "completed"
    let error: WorkerTurnResult["error"] = null
    if (interrupted) {
      result = "interrupted"
      if (message.subtype !== "success") summary = ""
    } else if (failed) {
      result = "failed"
      error = rejected
        ? { code: "Worker.UsageLimit", message: summary, resetAt: rejected.resetsAt ?? null }
        : { code: "Worker.Error", message: summary, resetAt: null }
    }
    return {
      result,
      summary,
      error,
      usage: {
        inputTokens: message.usage.input_tokens,
        outputTokens: message.usage.output_tokens,
        cacheReadInputTokens: message.usage.cache_read_input_tokens,
        cacheCreationInputTokens: message.usage.cache_creation_input_tokens,
      },
      totalCostUsd: message.total_cost_usd,
      durationMs: message.duration_ms,
    }
  }
}
