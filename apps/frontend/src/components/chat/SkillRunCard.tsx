import clsx from "clsx"
import {
  ChevronRight,
  CircleCheck,
  CircleX,
  Clock,
  ExternalLink,
  LoaderCircle,
  type LucideIcon,
} from "lucide-react"
import { Link } from "@tanstack/react-router"
import { useEffect, useRef, useState } from "react"
import type { ChatSkillRunFieldsFragment } from "../../graphql/generated/graphql.ts"
import { t } from "../../i18n/index.ts"
import { stamp } from "../../lib/format.ts"
import { useNow } from "./Timeline.tsx"
import { elapsed } from "./work.ts"

type SkillRun = ChatSkillRunFieldsFragment
type SkillState = SkillRun["skill"]["states"][number]
type Tone = "run" | "ok" | "err"

const TONE: Record<Tone, { card: string; head: string; text: string }> = {
  run: { card: "border-l-run", head: "bg-run/7", text: "text-run" },
  ok: { card: "border-l-ok", head: "bg-ok/7", text: "text-ok" },
  err: { card: "border-l-err", head: "bg-err/7", text: "text-err" },
}

function tone(run: SkillRun): Tone {
  switch (run.status) {
    case "SUCCEEDED":
      return "ok"
    case "FAILED":
      return "err"
    default:
      return "run"
  }
}

function statusIcon(run: SkillRun): LucideIcon {
  switch (run.status) {
    case "SUCCEEDED":
      return CircleCheck
    case "FAILED":
      return CircleX
    case "WAITING_AGENT":
      return Clock
    default:
      return LoaderCircle
  }
}

/** The TASK states from StartAt along each state's next, or first choice, transition. */
export function mainPath(skill: SkillRun["skill"]): SkillState[] {
  const byName = new Map(skill.states.map((state) => [state.name, state]))
  const seen = new Set<string>()
  const path: SkillState[] = []
  let name = skill.startAt
  while (name && !seen.has(name)) {
    seen.add(name)
    const state = byName.get(name)
    if (!state) {
      break
    }
    path.push(state)
    const next =
      state.transitions.find((transition) => transition.kind === "NEXT") ??
      state.transitions.find((transition) => transition.kind === "CHOICE")
    name = next?.to ?? null
  }
  return path.filter((state) => state.type === "TASK")
}

/** How far the run is along the path: the index of the state it is at, or the path's length when it succeeded. */
export function pathPosition(run: SkillRun, path: SkillState[]): number {
  if (run.status === "SUCCEEDED") {
    return path.length
  }
  const names = path.map((state) => state.name)
  const at = names.indexOf(run.currentState ?? "")
  if (at >= 0) {
    return at
  }
  const last = run.stateEntries.findLast((entry) => names.includes(entry.stateName))
  return last ? names.indexOf(last.stateName) : 0
}

function seconds(from: string, to: string | number): number {
  return ((typeof to === "number" ? to : new Date(to).getTime()) - new Date(from).getTime()) / 1000
}

function fields(json: string | null): [string, string][] {
  if (json === null) {
    return []
  }
  const value: unknown = JSON.parse(json)
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return [["", JSON.stringify(value)]]
  }
  return Object.entries(value).map(([key, item]) => [
    key,
    typeof item === "string" ? item : JSON.stringify(item),
  ])
}

function shortRepo(repository: string): string {
  return repository.split("/").pop() ?? repository
}

function Summary({ run }: { run: SkillRun }) {
  const [output] = run.outputs
  if (run.status === "SUCCEEDED") {
    return output ? (
      <a
        href={output.url}
        target="_blank"
        rel="noreferrer"
        title={`${output.repository}#${output.number}`}
        className="flex min-w-0 font-medium text-ink underline decoration-line2 underline-offset-[3px] hover:decoration-ink"
        onClick={(event) => event.stopPropagation()}
      >
        <span className="shrink-0 pr-1 no-underline" aria-hidden>
          →
        </span>
        <span className="truncate">{shortRepo(output.repository)}</span>
        <span className="shrink-0">#{output.number}</span>
      </a>
    ) : null
  }
  if (run.status === "FAILED") {
    return (
      <span className="truncate">{t("skillRun.failedAt", { state: run.currentState ?? "" })}</span>
    )
  }
  if (run.status === "WAITING_AGENT") {
    return <span className="truncate">{t("task.fixRound", { n: run.handoffs })}</span>
  }
  return <span className="truncate">{run.currentState}</span>
}

function Steps({ run, path }: { run: SkillRun; path: SkillState[] }) {
  const position = pathPosition(run, path)
  return (
    <span className="flex gap-0.5 px-3 pb-1.5" aria-hidden>
      {path.map((state, index) => (
        <i
          key={state.name}
          title={state.name}
          className={clsx(
            "block h-1 flex-1 rounded-sm",
            index < position
              ? "bg-ok"
              : index === position
                ? run.status === "FAILED"
                  ? "bg-err"
                  : "bg-run"
                : "bg-line2",
          )}
        />
      ))}
    </span>
  )
}

function Node({ run, state }: { run: SkillRun; state: string }) {
  const entry = run.stateEntries.findLast((candidate) => candidate.stateName === state)
  const status = entry?.status
  const detail = !entry
    ? null
    : status === "SUCCEEDED"
      ? entry.exitedAt
        ? t("skillRun.stateDone", {
            d: elapsed(seconds(entry.enteredAt, entry.exitedAt)),
            n: entry.attempts,
          })
        : null
      : status === "FAILED" || status === "TIMED_OUT"
        ? t("skillRun.stateFailed", { n: entry.attempts })
        : status === "CAUGHT"
          ? t("skillRun.stateCaught")
          : t("skillRun.stateRunning", { n: entry.attempts })
  return (
    <span
      className={clsx(
        "shrink-0 rounded-ui border bg-bg px-2.5 py-1 font-mono text-xs text-ink",
        !entry && "border-line text-faint",
        status === "SUCCEEDED" && "border-ok/50",
        (status === "RUNNING" || status === "WAITING") && "border-run ring-3 ring-run/15",
        (status === "FAILED" || status === "TIMED_OUT") && "border-err ring-3 ring-err/15",
        status === "CAUGHT" && "border-line2",
      )}
    >
      {state}
      {detail ? (
        <small
          className={clsx(
            "block font-sans text-[11px]",
            status === "SUCCEEDED"
              ? "text-ok"
              : status === "FAILED" || status === "TIMED_OUT"
                ? "text-err"
                : "text-run",
          )}
        >
          {detail}
        </small>
      ) : null}
    </span>
  )
}

function Flow({ run, states }: { run: SkillRun; states: string[] }) {
  const container = useRef<HTMLDivElement>(null)
  const current = run.currentState
  useEffect(() => {
    const element = container.current
    const node = element?.querySelector<HTMLElement>(`[data-state="${CSS.escape(current ?? "")}"]`)
    if (element && node) {
      element.scrollLeft = node.offsetLeft - (element.clientWidth - node.offsetWidth) / 2
    }
  }, [current])
  return (
    <div
      ref={container}
      className="relative -m-1 mb-1.5 flex items-center overflow-x-auto p-1 [scrollbar-width:thin]"
    >
      {states.map((state, index) => (
        <span key={state} data-state={state} className="flex shrink-0 items-center">
          {index > 0 ? <span className="h-px w-4 bg-line2" aria-hidden /> : null}
          <Node run={run} state={state} />
        </span>
      ))}
    </div>
  )
}

function Fields({ label, json, error }: { label: string; json: string | null; error?: boolean }) {
  const rows = fields(json)
  return (
    <div
      className={clsx(
        "min-w-0 rounded-ui border bg-bg px-2.5 py-2",
        error ? "border-err/40" : "border-line",
      )}
    >
      <div className="text-[11px] tracking-wide text-muted uppercase">{label}</div>
      {rows.length === 0 ? (
        <div className="mt-1 text-xs text-faint">{t("skillRun.noOutputYet")}</div>
      ) : (
        <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-2.5 font-mono text-xs">
          {rows.map(([key, value]) => (
            <div key={key} className="contents">
              <dt className="text-muted">{key}</dt>
              <dd className={clsx("truncate", error ? "text-err" : "text-body")} title={value}>
                {value}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  )
}

/** A skill run of a chat turn: a heading that expands into the run's states, input and output. */
export function SkillRunCard({ run }: { run: SkillRun }) {
  const [open, setOpen] = useState(run.status === "FAILED")
  const live = run.status !== "SUCCEEDED" && run.status !== "FAILED"
  const now = useNow(live)
  const colors = TONE[tone(run)]
  const StatusIcon = statusIcon(run)
  const path = mainPath(run.skill)
  const offPath =
    live && run.currentState && !path.some((state) => state.name === run.currentState)
      ? run.currentState
      : null
  const took = elapsed(seconds(run.startedAt, run.endedAt ?? now))
  const timeout = run.skill.timeoutSeconds
  const error = run.error
    ? JSON.stringify(run.cause ? { [run.error]: run.cause } : { error: run.error })
    : null
  return (
    <div
      className={clsx("overflow-hidden rounded-ui border border-l-[3px] border-line", colors.card)}
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className={clsx(
          "flex min-h-9 w-full items-center gap-2 px-3 py-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-line2 focus-visible:ring-inset",
          colors.head,
        )}
      >
        <StatusIcon
          className={clsx(
            "size-3.5 shrink-0",
            colors.text,
            run.status === "RUNNING" && "motion-safe:animate-spin",
          )}
          aria-hidden
        />
        <span className="shrink-0 font-mono text-sm font-medium text-ink">{run.skill.name}</span>
        <span
          className={clsx(
            "shrink-0 rounded-kbd border border-current px-1.5 text-[11px] leading-4 font-medium",
            colors.text,
          )}
        >
          {t("skillRun.engine")}
        </span>
        <span className="flex min-w-0 flex-1 text-sm text-body">
          <Summary run={run} />
        </span>
        <span className={clsx("flex shrink-0 items-center gap-2 text-xs", colors.text)}>
          {took}
          <ChevronRight
            className={clsx("size-3.5 text-muted transition-transform", open && "rotate-90")}
            aria-hidden
          />
        </span>
      </button>
      {path.length > 1 ? (
        <div className={colors.head}>
          <Steps run={run} path={path} />
        </div>
      ) : null}
      {open ? (
        <div className="border-t border-line bg-sub p-3">
          {path.length > 0 || offPath ? (
            <Flow
              run={run}
              states={[...path.map((state) => state.name), ...(offPath ? [offPath] : [])]}
            />
          ) : null}
          <div className="grid gap-2">
            <Fields label={t("skillRun.input")} json={run.input} />
            {error ? (
              <Fields label={t("skillRun.error")} json={error} error />
            ) : (
              <Fields label={t("skillRun.output")} json={run.output} />
            )}
          </div>
          <div className="mt-2.5 flex flex-wrap gap-x-3.5 gap-y-1 text-xs text-muted">
            {run.run ? (
              <Link
                to="/tasks/$task/runs/$n"
                params={{ task: run.task.id, n: String(run.run.number) }}
                className="inline-flex items-center gap-1 hover:text-ink"
              >
                {t("skillRun.openRun")}
                <ExternalLink className="size-3" aria-hidden />
              </Link>
            ) : null}
            <Link to="/skills/$skill" params={{ skill: run.skill.name }} className="hover:text-ink">
              {t("skillRun.definition")}
            </Link>
            <span>{t("skillRun.startedAt", { at: stamp(run.startedAt) })}</span>
            {live && timeout ? (
              <span>
                {t("skillRun.timesOutIn", {
                  d: elapsed(timeout - seconds(run.startedAt, now)),
                })}
              </span>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  )
}
