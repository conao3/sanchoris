import type { LucideIcon } from "lucide-react"
import {
  Ban,
  CircleCheck,
  CircleDashed,
  CirclePause,
  CircleX,
  Clock,
  LoaderCircle,
  MessageCircle,
  Square,
  Timer,
  TriangleAlert,
} from "lucide-react"
import type {
  RunStatus,
  SkillRunStatus,
  StateEntryStatus,
  TaskStatus,
  WaitReason,
} from "../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../i18n/index.ts"
import { State, type Tone } from "./ui.tsx"

type Look = { tone: Tone; icon: LucideIcon; label: MessageKey }

function taskLook(status: TaskStatus, waitReason?: WaitReason | null): Look {
  switch (status) {
    case "PENDING":
      return { tone: "wait", icon: Clock, label: "status.waiting" }
    case "RUNNING":
      return { tone: "run", icon: CircleDashed, label: "status.running" }
    case "WAITING":
      if (waitReason === "USAGE_LIMIT") {
        return { tone: "wait", icon: Timer, label: "status.limited" }
      }
      if (waitReason === "MESSAGE") {
        return { tone: "wait", icon: MessageCircle, label: "status.waitingForMessage" }
      }
      return { tone: "wait", icon: Clock, label: "status.waiting" }
    case "NEEDS_YOU":
      return { tone: "warn", icon: CirclePause, label: "status.needsYou" }
    case "SUCCEEDED":
      return { tone: "ok", icon: CircleCheck, label: "status.completed" }
    case "FAILED":
      return { tone: "err", icon: CircleX, label: "status.failed" }
    case "ABORTED":
      return { tone: "wait", icon: Square, label: "status.stopped" }
  }
}

export function TaskState({
  status,
  waitReason,
}: {
  status: TaskStatus
  waitReason?: WaitReason | null
}) {
  const look = taskLook(status, waitReason)
  return (
    <State tone={look.tone} icon={look.icon}>
      {t(look.label)}
    </State>
  )
}

const skillRunLooks: Record<SkillRunStatus, Look> = {
  RUNNING: { tone: "run", icon: CircleDashed, label: "status.running" },
  WAITING_AGENT: { tone: "wait", icon: Clock, label: "status.withAgent" },
  WAITING: { tone: "wait", icon: Clock, label: "status.waiting" },
  SUCCEEDED: { tone: "ok", icon: CircleCheck, label: "status.succeeded" },
  FAILED: { tone: "err", icon: CircleX, label: "status.failed" },
}

export function SkillRunState({ status }: { status: SkillRunStatus }) {
  const look = skillRunLooks[status]
  return (
    <State tone={look.tone} icon={look.icon}>
      {t(look.label)}
    </State>
  )
}

const stateEntryLooks: Record<StateEntryStatus, Look> = {
  RUNNING: { tone: "run", icon: CircleDashed, label: "status.running" },
  WAITING: { tone: "wait", icon: Clock, label: "status.waiting" },
  SUCCEEDED: { tone: "ok", icon: CircleCheck, label: "status.succeeded" },
  FAILED: { tone: "err", icon: CircleX, label: "status.failed" },
  CAUGHT: { tone: "warn", icon: TriangleAlert, label: "status.caught" },
  TIMED_OUT: { tone: "err", icon: Timer, label: "status.timedOut" },
}

export function StateEntryState({ status }: { status: StateEntryStatus }) {
  const look = stateEntryLooks[status]
  return (
    <State tone={look.tone} icon={look.icon}>
      {t(look.label)}
    </State>
  )
}

const runLooks: Record<RunStatus, Look> = {
  CREATING: { tone: "run", icon: LoaderCircle, label: "status.starting" },
  RUNNING: { tone: "run", icon: CircleDashed, label: "status.running" },
  IDLE: { tone: "wait", icon: MessageCircle, label: "status.idle" },
  COMPLETED: { tone: "ok", icon: CircleCheck, label: "status.completed" },
  FAILED: { tone: "err", icon: Ban, label: "status.failed" },
}

export function RunState({ status }: { status: RunStatus }) {
  const look = runLooks[status]
  return (
    <State tone={look.tone} icon={look.icon}>
      {t(look.label)}
    </State>
  )
}
