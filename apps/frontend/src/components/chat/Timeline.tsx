import clsx from "clsx"
import { Check, ChevronRight, Copy, LoaderCircle, type LucideIcon } from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { t } from "../../i18n/index.ts"
import { clock } from "../../lib/format.ts"
import { Markdown } from "./Markdown.tsx"
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

const COPIED_MS = 1500
const TICK_MS = 1000

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    const timer = copied ? window.setTimeout(() => setCopied(false), COPIED_MS) : undefined
    return () => window.clearTimeout(timer)
  }, [copied])
  const Icon = copied ? Check : Copy
  return (
    <button
      type="button"
      aria-label={copied ? t("chat.copied") : t("chat.copy")}
      title={copied ? t("chat.copied") : t("chat.copy")}
      onClick={() => void navigator.clipboard.writeText(text).then(() => setCopied(true))}
      className="rounded-kbd p-0.5 text-faint outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-line2"
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

export function MemberMessage({ name, text, at }: { name: string; text: string; at: string }) {
  return (
    <div className="group flex flex-col items-end">
      <div className="max-w-[80%] rounded-bubble bg-hover px-3.5 py-2 break-words whitespace-pre-wrap text-ink">
        {text}
      </div>
      <Meta align="right">
        <span>
          {name} · {clock(at)}
        </span>
        <CopyButton text={text} />
      </Meta>
    </div>
  )
}

export function AgentMessage({
  text,
  at,
  work,
  startedAt,
  children,
}: {
  text: string
  at: string
  work: Work[]
  startedAt?: string
  children?: ReactNode
}) {
  return (
    <div className="group">
      {work.length > 0 ? (
        <TurnFold
          label={
            startedAt
              ? t("work.workedFor", {
                  d: elapsed((new Date(at).getTime() - new Date(startedAt).getTime()) / 1000),
                })
              : t("work.worked")
          }
          work={work}
        />
      ) : null}
      {text.trim() ? <Markdown text={text} className="text-ink" /> : null}
      {children}
      <Meta align="left">
        <span>sanchoris · {clock(at)}</span>
        {text.trim() ? <CopyButton text={text} /> : null}
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
}: {
  startedAt: string
  work: Work[]
  waiting: boolean
}) {
  const now = useNow(true)
  const latest = work.findLast((entry) => entry.kind !== "TEXT")
  const seconds = (now - new Date(startedAt).getTime()) / 1000
  return (
    <div>
      {work.length > 0 ? <WorkSegments work={work} /> : null}
      <Disclosure
        icon={LoaderCircle}
        live
        label={
          waiting
            ? t("work.waitingAnswer")
            : latest
              ? `${t("work.workingFor", { d: elapsed(seconds) })} · ${workLabel(latest)}`
              : t("work.workingFor", { d: elapsed(seconds) })
        }
      />
    </div>
  )
}
