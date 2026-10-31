import type { NeedsYouReason, SkillRunStatus, TaskStatus } from "../graphql/generated/graphql.ts"
import { t } from "../i18n/index.ts"
import { usd, weekdayTime } from "./format.ts"

type SkillRunSummary = { status: SkillRunStatus; currentState?: string | null; handoffs: number }
type RequiredSkill = { skill?: { name: string }; latestRun?: SkillRunSummary | null }

const finishedStatuses: TaskStatus[] = ["SUCCEEDED", "FAILED", "ABORTED"]

export function isActive(status: TaskStatus): boolean {
  return !finishedStatuses.includes(status)
}

export function skillStep(run: SkillRunSummary): string {
  switch (run.status) {
    case "FAILED":
      return run.handoffs > 0 ? t("task.failedAfterRounds", { n: run.handoffs }) : t("task.failed")
    case "SUCCEEDED":
      return t("task.succeeded")
    case "WAITING_AGENT":
      return t("task.fixRound", { n: run.handoffs })
    default:
      return t("task.atState", { state: run.currentState ?? "" })
  }
}

export function skillProgress(task: {
  status: TaskStatus
  requiredSkills: RequiredSkill[]
}): string {
  const required = task.requiredSkills[0]
  if (!required) {
    return t("task.noRequiredSkill")
  }
  if (task.status === "PENDING") {
    return t("task.notStarted")
  }
  if (!required.latestRun) {
    return t("task.agentWorking")
  }
  return `${required.skill?.name ?? ""}: ${skillStep(required.latestRun)}`
}

export function needsYouText(task: {
  needsYouReason?: NeedsYouReason | null
  costUsd: number
  maxNudges?: number
  requiredSkills: RequiredSkill[]
}): string {
  const run = task.requiredSkills[0]?.latestRun
  const skill = task.requiredSkills[0]?.skill?.name ?? ""
  const spent = t("task.spent", { cost: usd(task.costUsd) })
  switch (task.needsYouReason) {
    case "SKILL_FAILED":
      return `${t("task.gaveUp", { skill, n: run?.handoffs ?? 0 })} · ${spent}`
    case "NUDGES_EXHAUSTED":
      return `${t("task.nudgesExhausted", { skill })} · ${spent}`
    case "AGENT_ASKED":
      return `${t("task.agentAsked")} · ${spent}`
    case "APPROVAL_REQUESTED":
      return `${t("task.approvalRequested")} · ${spent}`
    case "USAGE_LIMIT":
      return `${t("task.usageLimitManual")} · ${spent}`
    case "LIMIT_EXCEEDED":
      return `${t("task.limitExceeded")} · ${spent}`
    case "WORKSPACE_STOPPED":
      return `${t("task.workspaceStopped")} · ${spent}`
    default:
      return spent
  }
}

export function limitedText(task: {
  session: { agent: string }
  resumeAt?: string | null
  autoResume: boolean
}): string {
  const parts = [
    t("task.limitReached", {
      agent: t(task.session.agent === "CODEX" ? "provider.codex" : "provider.claude"),
    }),
  ]
  if (task.resumeAt) {
    parts.push(t("task.resumes", { at: weekdayTime(task.resumeAt) }))
  }
  parts.push(task.autoResume ? t("task.autoResumeOn") : t("task.autoResumeOff"))
  return parts.join(" · ")
}

export function originName(task: {
  trigger: { name: string } | null
  conversation: { sessionName: string } | null
}): string {
  if (task.trigger) {
    return task.trigger.name
  }
  return t("tasks.chatSession", { session: task.conversation?.sessionName ?? "" })
}
