import {
  Bot,
  Brain,
  Eye,
  Globe,
  Hammer,
  ListTodo,
  type LucideIcon,
  MessageCircleQuestion,
  Plug,
  Search,
  ShieldCheck,
  SquarePen,
  SquareTerminal,
} from "lucide-react"
import type {
  ChatApprovalDecision,
  ChatWorkFieldsFragment,
} from "../../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../../i18n/index.ts"

export type Work = ChatWorkFieldsFragment
type Tool = NonNullable<Work["tool"]>

export type WorkCategory =
  | "thinking"
  | "question"
  | "command"
  | "read"
  | "edit"
  | "search"
  | "web"
  | "agent"
  | "todo"
  | "approval"
  | "mcp"
  | "other"

const TOOL_CATEGORIES: Record<string, WorkCategory> = {
  bash: "command",
  commandexecution: "command",
  read: "read",
  edit: "edit",
  write: "edit",
  multiedit: "edit",
  notebookedit: "edit",
  filechange: "edit",
  grep: "search",
  glob: "search",
  ls: "search",
  webfetch: "web",
  websearch: "web",
  task: "agent",
  agent: "agent",
  todowrite: "todo",
}

const ICONS: Record<WorkCategory, LucideIcon> = {
  thinking: Brain,
  question: MessageCircleQuestion,
  command: SquareTerminal,
  read: Eye,
  edit: SquarePen,
  search: Search,
  web: Globe,
  agent: Bot,
  todo: ListTodo,
  approval: ShieldCheck,
  mcp: Plug,
  other: Hammer,
}

const SUMMARY_KEYS: Record<WorkCategory, [MessageKey, MessageKey]> = {
  thinking: ["work.sum.thinking.one", "work.sum.thinking.other"],
  question: ["work.sum.question.one", "work.sum.question.other"],
  command: ["work.sum.command.one", "work.sum.command.other"],
  read: ["work.sum.read.one", "work.sum.read.other"],
  edit: ["work.sum.edit.one", "work.sum.edit.other"],
  search: ["work.sum.search.one", "work.sum.search.other"],
  web: ["work.sum.web.one", "work.sum.web.other"],
  agent: ["work.sum.agent.one", "work.sum.agent.other"],
  todo: ["work.sum.todo.one", "work.sum.todo.other"],
  approval: ["work.sum.approval.one", "work.sum.approval.other"],
  mcp: ["work.sum.mcp.one", "work.sum.mcp.other"],
  other: ["work.sum.other.one", "work.sum.other.other"],
}

const DECISION_KEYS: Record<ChatApprovalDecision, MessageKey> = {
  ACCEPT: "work.approved",
  ACCEPT_FOR_SESSION: "work.approvedSession",
  DECLINE: "work.declined",
  CANCEL: "work.cancelled",
}

const WORKSPACE_PREFIX = /^\/workspace\//

export function toolCategory(name: string): WorkCategory {
  if (name.startsWith("mcp__")) {
    return "mcp"
  }
  return TOOL_CATEGORIES[name.toLowerCase()] ?? "other"
}

export function workCategory(work: Work): WorkCategory {
  if (work.kind === "THINKING") {
    return "thinking"
  }
  if (work.kind === "QUESTION") {
    return "question"
  }
  if (work.kind === "PLAN") {
    return "todo"
  }
  if (work.kind === "APPROVAL") {
    return "approval"
  }
  return toolCategory(work.tool?.name ?? "")
}

export function workIcon(work: Work): LucideIcon {
  return ICONS[workCategory(work)]
}

function firstLine(text: string, max = 90): string {
  const line = text.trim().split("\n")[0] ?? ""
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

export function toolInput(tool: Tool): Record<string, unknown> {
  const parsed: unknown = JSON.parse(tool.input)
  return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)
    ? (parsed as Record<string, unknown>)
    : {}
}

function field(input: Record<string, unknown>, ...names: string[]): string {
  for (const name of names) {
    const value = input[name]
    if (typeof value === "string" && value !== "") {
      return value
    }
  }
  return ""
}

function path(value: string): string {
  return value.replace(WORKSPACE_PREFIX, "")
}

export function toolCommand(tool: Tool): string {
  return field(toolInput(tool), "command")
}

export function toolLabel(tool: Tool): string {
  const input = toolInput(tool)
  switch (toolCategory(tool.name)) {
    case "command":
      return t("work.ran", { what: firstLine(field(input, "command")) })
    case "read":
      return t("work.read", { what: path(field(input, "file_path", "path")) })
    case "edit": {
      const changes = Array.isArray(input.changes)
        ? (input.changes as { path?: string }[]).map((change) => path(change.path ?? ""))
        : [path(field(input, "file_path", "notebook_path", "path"))]
      return t("work.edited", { what: changes.join(", ") })
    }
    case "search":
      return t("work.searched", { what: firstLine(field(input, "pattern", "path", "query")) })
    case "web":
      return field(input, "url")
        ? t("work.fetched", { what: field(input, "url") })
        : t("work.searchedWeb", { what: firstLine(field(input, "query")) })
    case "agent":
      return t("work.agent", { what: firstLine(field(input, "description", "prompt")) })
    case "todo":
      return t("work.todo")
    case "mcp": {
      const [, server = "", name = ""] = tool.name.split("__")
      return t("work.called", { what: `${server} ${name}` })
    }
    default:
      return t("work.used", { what: tool.name })
  }
}

export function answerText(question: NonNullable<Work["question"]>, id: string): string {
  return question.answers?.find((answer) => answer.questionId === id)?.answers.join(", ") ?? ""
}

export function workLabel(work: Work): string {
  if (work.kind === "THINKING") {
    return firstLine((work.text ?? "").replace(/\*\*/g, ""), 140) || t("work.thought")
  }
  if (work.kind === "PLAN" && work.plan) {
    const done = work.plan.steps.filter((step) => step.status === "COMPLETED").length
    return t("work.planUpdated", { done, total: work.plan.steps.length })
  }
  if (work.kind === "APPROVAL" && work.approval) {
    return t(DECISION_KEYS[work.approval.decision ?? "DECLINE"], {
      what: firstLine(work.approval.detail || work.approval.toolName),
    })
  }
  if (work.kind === "QUESTION" && work.question) {
    const question = work.question
    return question.items
      .map((item) => `${firstLine(item.question, 60)} · ${answerText(question, item.id)}`)
      .join(" / ")
  }
  return work.tool ? toolLabel(work.tool) : ""
}

export function groupSummary(works: Work[]): string {
  const counts = new Map<WorkCategory, number>()
  for (const work of works) {
    const category = workCategory(work)
    counts.set(category, (counts.get(category) ?? 0) + 1)
  }
  const parts = [...counts].map(([category, n]) =>
    t(SUMMARY_KEYS[category][n === 1 ? 0 : 1], { n }),
  )
  const text =
    parts.length > 1
      ? `${parts.slice(0, -1).join(t("work.separator"))}${t("work.and")}${parts[parts.length - 1]}`
      : (parts[0] ?? "")
  return text.charAt(0).toUpperCase() + text.slice(1)
}

export function groupIcon(works: Work[]): LucideIcon {
  const categories = new Set(works.map(workCategory))
  if (categories.size === 1) {
    return ICONS[[...categories][0] ?? "other"]
  }
  categories.delete("thinking")
  return categories.size === 1 ? ICONS[[...categories][0] ?? "other"] : Hammer
}

export function elapsed(seconds: number): string {
  const s = Math.max(0, Math.round(seconds))
  if (s < 60) {
    return t("work.elapsed.s", { s })
  }
  if (s < 3600) {
    return t("work.elapsed.ms", { m: Math.floor(s / 60), s: s % 60 })
  }
  return t("work.elapsed.hm", { h: Math.floor(s / 3600), m: Math.floor((s % 3600) / 60) })
}

export type Segment = { kind: "text"; work: Work } | { kind: "group"; works: Work[] }

export function segments(works: Work[]): Segment[] {
  const result: Segment[] = []
  for (const work of works) {
    if (work.kind === "TEXT") {
      result.push({ kind: "text", work })
      continue
    }
    const last = result[result.length - 1]
    if (last?.kind === "group") {
      last.works.push(work)
    } else {
      result.push({ kind: "group", works: [work] })
    }
  }
  return result
}
