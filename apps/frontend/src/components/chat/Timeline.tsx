import clsx from "clsx"
import {
  ChevronRight,
  CircleStop,
  CornerDownLeft,
  FoldVertical,
  GitFork,
  LoaderCircle,
  Undo2,
  type LucideIcon,
} from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import type { ChatTurnOutcome } from "../../graphql/generated/graphql.ts"
import { t } from "../../i18n/index.ts"
import { clock } from "../../lib/format.ts"
import { CopyButton } from "./CopyButton.tsx"
import { Markdown } from "./Markdown.tsx"
import { PlanSteps } from "./Plan.tsx"
import {
  type Work,
  answerText,
  elapsed,
  groupIcon,
  groupSummary,
  segments,
  toolCommand,
  toolInput,
  workIcon,
  workLabel,
} from "./work.ts"

const TICK_MS = 1000

function MetaAction({
  icon: Icon,
  label,
  reason,
  onPress,
}: {
  icon: LucideIcon
  label: string
  reason?: string
  onPress: () => void
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={reason ?? label}
      aria-disabled={!!reason}
      onClick={() => {
        if (!reason) {
          onPress()
        }
      }}
      className={clsx(
        "rounded-kbd p-0.5 outline-none focus-visible:ring-2 focus-visible:ring-line2",
        reason ? "cursor-not-allowed text-line2" : "text-faint hover:text-ink",
      )}
    >
      <Icon className="size-3.5" aria-hidden />
    </button>
  )
}

function Meta({ children, align }: { children: ReactNode; align: "left" | "right" }) {
  return (
    <div
      className={clsx(
        "mt-1 flex items-center gap-2 text-xs text-faint opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100 pointer-coarse:opacity-100",
        align === "right" && "justify-end",
      )}
    >
      {children}
    </div>
  )
}

export function MemberMessage({
  name,
  text,
  at,
  steered,
  images = [],
  onRewind,
  rewindReason,
}: {
  name: string
  text: string
  at: string
  steered?: boolean
  images?: { id: string; name: string; url: string }[]
  onRewind?: () => void
  rewindReason?: string
}) {
  return (
    <div className="group flex flex-col items-end">
      {steered ? (
        <span
          className="mb-0.5 flex items-center gap-1 text-xs text-muted"
          title={t("chat.steeredHint")}
        >
          <CornerDownLeft className="size-3" aria-hidden />
          {t("chat.steered")}
        </span>
      ) : null}
      {images.length > 0 ? (
        <div className="mb-1 flex max-w-[80%] flex-wrap justify-end gap-1.5">
          {images.map((image) => (
            <a
              key={image.id}
              href={image.url}
              target="_blank"
              rel="noreferrer"
              title={t("chat.openImage", { name: image.name })}
              className="block overflow-hidden rounded-ui border border-line"
            >
              <img
                src={image.url}
                alt={image.name}
                className="max-h-52 w-full max-w-[210px] object-cover"
              />
            </a>
          ))}
        </div>
      ) : null}
      <div className="max-w-[80%] rounded-bubble bg-hover px-3.5 py-2 break-words whitespace-pre-wrap text-ink">
        {text}
      </div>
      <Meta align="right">
        <span>
          {name} · {clock(at)}
        </span>
        <CopyButton text={text} />
        {onRewind ? (
          <MetaAction
            icon={Undo2}
            label={t("rewind.action")}
            reason={rewindReason}
            onPress={onRewind}
          />
        ) : null}
      </Meta>
    </div>
  )
}

export function CompactNotice({ at }: { at: string }) {
  return (
    <div className="flex items-center gap-3 text-xs text-muted" title={t("chat.autoCompactedHint")}>
      <span className="h-px flex-1 bg-line" aria-hidden />
      <span className="flex items-center gap-1.5">
        <FoldVertical className="size-3.5" aria-hidden />
        {t("chat.autoCompacted")} · {clock(at)}
      </span>
      <span className="h-px flex-1 bg-line" aria-hidden />
    </div>
  )
}

export function AgentMessage({
  text,
  at,
  work,
  startedAt,
  outcome,
  onFork,
  children,
}: {
  text: string
  at: string
  work: Work[]
  startedAt?: string
  outcome?: ChatTurnOutcome | null
  onFork?: () => void
  children?: ReactNode
}) {
  return (
    <div className="group">
      {work.length > 0 ? (
        <TurnFold
          label={
            startedAt
              ? t(outcome === "INTERRUPTED" ? "work.stoppedAfter" : "work.workedFor", {
                  d: elapsed((new Date(at).getTime() - new Date(startedAt).getTime()) / 1000),
                })
              : t(outcome === "INTERRUPTED" ? "work.stopped" : "work.worked")
          }
          work={work}
        />
      ) : null}
      {text.trim() ? <Markdown text={text} className="text-ink" /> : null}
      {outcome === "INTERRUPTED" || outcome === "FAILED" ? (
        <div
          className={clsx(
            "mt-1.5 flex items-center gap-1.5 text-sm",
            outcome === "FAILED" ? "text-err" : "text-muted",
          )}
        >
          <CircleStop className="size-3.5 shrink-0" aria-hidden />
          {outcome === "FAILED" ? t("chat.failed") : t("chat.interrupted")}
        </div>
      ) : null}
      {children}
      <Meta align="left">
        <span>sanchoris · {clock(at)}</span>
        {text.trim() ? <CopyButton text={text} /> : null}
        {onFork ? <MetaAction icon={GitFork} label={t("fork.action")} onPress={onFork} /> : null}
      </Meta>
    </div>
  )
}

function Disclosure({
  icon: Icon,
  label,
  time,
  open,
  onToggle,
  tone,
  live,
}: {
  icon: LucideIcon
  label: string
  time?: string
  open?: boolean
  onToggle?: () => void
  tone?: "err"
  live?: boolean
}) {
  const content = (
    <>
      <Icon className={clsx("size-3.5 shrink-0", live && "animate-pulse")} aria-hidden />
      <span className={clsx("min-w-0 truncate", live && "animate-pulse")}>{label}</span>
      {onToggle ? (
        <ChevronRight
          className={clsx("size-3.5 shrink-0 transition-transform", open && "rotate-90")}
          aria-hidden
        />
      ) : null}
      {time ? (
        <span className="ml-auto shrink-0 pl-2 text-xs text-faint">{clock(time)}</span>
      ) : null}
    </>
  )
  const classes = clsx(
    "flex h-7 w-full items-center gap-2 rounded-kbd px-1.5 text-left text-sm",
    tone === "err" ? "text-err" : "text-muted",
  )
  if (!onToggle) {
    return <div className={classes}>{content}</div>
  }
  return (
    <button
      type="button"
      aria-expanded={open}
      onClick={onToggle}
      className={clsx(
        classes,
        "outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-line2",
      )}
    >
      {content}
    </button>
  )
}

function Output({ children, tone }: { children: ReactNode; tone?: "err" }) {
  return (
    <pre
      className={clsx(
        "max-h-80 overflow-auto rounded-ui bg-term-bg p-2.5 font-mono text-xs break-words whitespace-pre-wrap",
        tone === "err" ? "text-term-error" : "text-term-fg",
      )}
    >
      {children}
    </pre>
  )
}

function WorkDetail({ work }: { work: Work }) {
  if (work.kind === "THINKING") {
    return <Markdown text={work.text ?? ""} className="text-sm text-muted" />
  }
  if (work.kind === "PLAN" && work.plan) {
    return <PlanSteps plan={work.plan} />
  }
  if (work.kind === "APPROVAL" && work.approval) {
    return (
      <div className="space-y-1.5 text-sm">
        <Output>{work.approval.detail || work.approval.toolName}</Output>
        {work.approval.reason ? <p className="text-muted">{work.approval.reason}</p> : null}
      </div>
    )
  }
  if (work.kind === "QUESTION" && work.question) {
    const question = work.question
    return (
      <dl className="space-y-1.5 text-sm">
        {question.items.map((item) => (
          <div key={item.id}>
            <dt className="text-muted">{item.question}</dt>
            <dd className="font-medium text-ink">{answerText(question, item.id)}</dd>
          </div>
        ))}
      </dl>
    )
  }
  const tool = work.tool
  if (!tool) {
    return null
  }
  const command = toolCommand(tool)
  const input = toolInput(tool)
  return (
    <div className="space-y-1.5">
      <Output>
        {command
          ? `$ ${command}`
          : Object.keys(input).length > 0
            ? JSON.stringify(input, null, 2)
            : tool.name}
      </Output>
      <Output tone={tool.isError ? "err" : undefined}>{tool.output ?? t("work.noOutput")}</Output>
    </div>
  )
}

function WorkRow({ work }: { work: Work }) {
  const [open, setOpen] = useState(false)
  return (
    <div>
      <Disclosure
        icon={workIcon(work)}
        label={workLabel(work)}
        time={work.at}
        open={open}
        onToggle={() => setOpen(!open)}
        tone={work.tool?.isError ? "err" : undefined}
      />
      {open ? (
        <div className="mt-1 mb-2 ml-6">
          <WorkDetail work={work} />
        </div>
      ) : null}
    </div>
  )
}

function WorkGroup({ works }: { works: Work[] }) {
  const [open, setOpen] = useState(false)
  const only = works.length === 1 ? works[0] : undefined
  if (only) {
    return <WorkRow work={only} />
  }
  return (
    <div>
      <Disclosure
        icon={groupIcon(works)}
        label={groupSummary(works)}
        time={works[works.length - 1]?.at}
        open={open}
        onToggle={() => setOpen(!open)}
        tone={works.some((work) => work.tool?.isError) ? "err" : undefined}
      />
      {open ? (
        <div className="ml-3 border-l border-line pl-2">
          {works.map((work, index) => (
            <WorkRow key={`${work.at}-${index}`} work={work} />
          ))}
        </div>
      ) : null}
    </div>
  )
}

function WorkSegments({ work }: { work: Work[] }) {
  return (
    <div className="space-y-1">
      {segments(work).map((segment, index) =>
        segment.kind === "text" ? (
          <Markdown key={index} text={segment.work.text ?? ""} className="my-1.5 text-ink" />
        ) : (
          <WorkGroup key={index} works={segment.works} />
        ),
      )}
    </div>
  )
}

function TurnFold({ label, work }: { label: string; work: Work[] }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="mb-1.5">
      <Disclosure
        icon={groupIcon(work)}
        label={label}
        open={open}
        onToggle={() => setOpen(!open)}
      />
      {open ? (
        <div className="mt-1 ml-3 border-l border-line pl-2">
          <WorkSegments work={work} />
        </div>
      ) : null}
    </div>
  )
}

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = active ? window.setInterval(() => setNow(Date.now()), TICK_MS) : undefined
    return () => window.clearInterval(timer)
  }, [active])
  return now
}

export function Working({
  startedAt,
  work,
  waiting,
  partial,
}: {
  startedAt: string
  work: Work[]
  waiting?: "question" | "approval" | "session"
  partial?: { kind: string; text: string } | null
}) {
  const now = useNow(true)
  const latest = work.findLast((entry) => entry.kind !== "TEXT")
  const seconds = (now - new Date(startedAt).getTime()) / 1000
  return (
    <div>
      {work.length > 0 ? <WorkSegments work={work} /> : null}
      {partial?.kind === "THINKING" ? (
        <Markdown text={partial.text} className="streaming my-1 text-sm text-muted" />
      ) : null}
      {partial?.kind === "TEXT" ? (
        <Markdown text={partial.text} className="streaming my-1.5 text-ink" />
      ) : null}
      <Disclosure
        icon={LoaderCircle}
        live
        label={
          waiting === "question"
            ? t("work.waitingAnswer")
            : waiting === "approval"
              ? t("work.waitingApproval")
              : waiting === "session"
                ? t("work.startingSession")
                : latest
                  ? `${t("work.workingFor", { d: elapsed(seconds) })} · ${workLabel(latest)}`
                  : t("work.workingFor", { d: elapsed(seconds) })
        }
      />
    </div>
  )
}
