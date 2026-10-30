import clsx from "clsx"
import { useEffect, useRef, useState } from "react"
import type { LucideIcon } from "lucide-react"
import {
  ArrowRight,
  Bot,
  CircleCheck,
  CircleDashed,
  CirclePause,
  CircleX,
  Clock,
  GitBranch,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  Split,
  Terminal,
  UserCheck,
} from "lucide-react"
import type { SkillQuery } from "../graphql/generated/graphql.ts"
import { t } from "../i18n/index.ts"
import { Icon, State } from "./ui.tsx"

type SkillState = NonNullable<NonNullable<SkillQuery["tenant"]>["skill"]>["states"][number]

const MAX_SCALE = 0.7
const MIN_SCALE = 0.62
const NODE_GAP = 20
const NODE_WIDTH = 120
const BASE_HEIGHT = 60
const SUMMARY_LINE = 18
const CHAR_WIDTH = 6.6
const COUNT_LINE = 18
const PORT_Y = 38
const MARGIN_X = 20
const MARGIN_Y = 24

const muted = "var(--color-muted)"
const danger = "var(--color-err)"

function nodeKind(state: SkillState): { label: string; icon: LucideIcon } {
  if (state.type !== "TASK" || !state.resource) {
    const icons: Record<SkillState["type"], LucideIcon> = {
      PASS: ArrowRight,
      CHOICE: Split,
      WAIT: Clock,
      SUCCEED: CircleCheck,
      FAIL: CircleX,
      TASK: Terminal,
    }
    return { label: state.type, icon: icons[state.type] }
  }
  const [service, action = ""] = state.resource.replace(/^sanchoris:::/, "").split(":")
  const icon = (() => {
    if (service === "git") return GitBranch
    if (service === "agent") return Bot
    if (service === "approval") return UserCheck
    if (service === "chat") return MessageSquare
    if (service === "workspace") return Terminal
    if (action.startsWith("createPullRequests")) return GitPullRequest
    if (action.startsWith("checks")) return CircleCheck
    if (action.startsWith("mergePullRequest")) return GitMerge
    return MessageSquare
  })()
  return { label: (service ?? "").toUpperCase(), icon }
}

function activityLines(state: SkillState) {
  const { activity } = state
  const lines = []
  if (activity.running > 0) {
    lines.push(
      <State key="running" tone="run" icon={CircleDashed}>
        {t("skill.nodeRunning", { n: activity.running })}
      </State>,
    )
  }
  if (activity.withAgent > 0) {
    lines.push(
      <State key="agent" tone="wait" icon={Clock}>
        {t("skill.nodeWithAgent", { n: activity.withAgent })}
      </State>,
    )
  }
  if (activity.needsYou > 0) {
    lines.push(
      <State key="needs" tone="warn" icon={CirclePause}>
        {t("skill.nodeNeedsYou", { n: activity.needsYou })}
      </State>,
    )
  }
  if (activity.succeededToday > 0) {
    lines.push(
      <State key="today" tone="ok" icon={CircleCheck}>
        {t("skill.nodeToday", { n: activity.succeededToday })}
      </State>,
    )
  }
  return lines
}

type Placed = {
  state: SkillState
  x: number
  y: number
  width: number
  height: number
  lines: ReturnType<typeof activityLines>
}

function edgePath(from: Placed, to: Placed): { d: string; labelX: number; labelY: number } {
  const fromCenter = from.x + from.width / 2
  const toCenter = to.x + to.width / 2
  if (Math.abs(from.y - to.y) < 10) {
    if (to.x > from.x) {
      return {
        d: `M${from.x + from.width} ${from.y + PORT_Y} H${to.x}`,
        labelX: to.x,
        labelY: to.y - 6,
      }
    }
    return {
      d: `M${from.x} ${from.y + PORT_Y} H${to.x + to.width}`,
      labelX: to.x + to.width + 4,
      labelY: to.y + PORT_Y - 6,
    }
  }
  if (to.y > from.y) {
    const bottom = from.y + from.height
    const exitRight = toCenter > fromCenter + 1
    const startX = exitRight ? from.x + from.width - 10 : fromCenter
    if (Math.abs(startX - toCenter) < 1) {
      return { d: `M${startX} ${bottom} V${to.y}`, labelX: startX + 6, labelY: (bottom + to.y) / 2 }
    }
    const mid = exitRight ? bottom + 20 : to.y - 24
    return {
      d: `M${startX} ${bottom} V${mid} H${toCenter} V${to.y}`,
      labelX: exitRight ? toCenter + 6 : startX + 6,
      labelY: exitRight ? to.y - 6 : mid - 6,
    }
  }
  const targetBottom = to.y + to.height
  if (Math.abs(fromCenter - toCenter) < 1) {
    return {
      d: `M${fromCenter} ${from.y} V${targetBottom}`,
      labelX: fromCenter + 6,
      labelY: (from.y + targetBottom) / 2,
    }
  }
  const mid = (from.y + targetBottom) / 2
  return {
    d: `M${fromCenter} ${from.y} V${mid} H${toCenter} V${targetBottom}`,
    labelX: fromCenter + 6,
    labelY: mid - 6,
  }
}

export function FlowDiagram({
  states,
  startAt,
  selected,
  onSelect,
}: {
  states: SkillState[]
  startAt?: string | null
  selected?: string
  onSelect: (name: string) => void
}) {
  const minX = Math.min(...states.map((state) => state.x))
  const minY = Math.min(...states.map((state) => state.y))
  const spanX = Math.max(...states.map((state) => state.x)) - minX
  const container = useRef<HTMLDivElement>(null)
  const [available, setAvailable] = useState(0)
  useEffect(() => {
    const element = container.current
    if (!element) {
      return
    }
    const observer = new ResizeObserver(([entry]) => setAvailable(entry?.contentRect.width ?? 0))
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  const scale =
    spanX === 0 || available === 0
      ? MAX_SCALE
      : Math.max(MIN_SCALE, Math.min(MAX_SCALE, (available - 2 * MARGIN_X - NODE_WIDTH) / spanX))
  const xs = [...new Set(states.map((state) => state.x))].sort((a, b) => a - b)
  const step = Math.min(...xs.slice(1).map((x, index) => x - (xs[index] ?? x)), Infinity)
  const nodeWidth = Number.isFinite(step)
    ? Math.min(NODE_WIDTH, Math.round(step * scale) - NODE_GAP)
    : NODE_WIDTH
  const placed = new Map<string, Placed>(
    states.map((state) => {
      const lines = activityLines(state)
      return [
        state.name,
        {
          state,
          lines,
          x: MARGIN_X + Math.round((state.x - minX) * scale),
          y: MARGIN_Y + Math.round((state.y - minY) * scale),
          width: nodeWidth,
          height:
            BASE_HEIGHT +
            SUMMARY_LINE *
              Math.max(1, Math.ceil((state.summary.length * CHAR_WIDTH) / (nodeWidth - 22))) +
            (lines.length > 0 ? 14 + lines.length * COUNT_LINE : 0),
        },
      ]
    }),
  )
  const nodes = [...placed.values()]
  const width = Math.max(...nodes.map((node) => node.x + node.width)) + MARGIN_X
  const height = Math.max(...nodes.map((node) => node.y + node.height)) + MARGIN_Y
  const fails = new Set(states.filter((state) => state.type === "FAIL").map((state) => state.name))
  const edges = nodes.flatMap((node) =>
    node.state.transitions.flatMap((transition) => {
      const target = placed.get(transition.to)
      if (!target) {
        return []
      }
      return [
        {
          key: `${node.state.name}-${transition.to}`,
          path: edgePath(node, target),
          label: transition.label,
          danger: fails.has(transition.to),
        },
      ]
    }),
  )

  return (
    <div
      ref={container}
      data-scroll-x
      className="relative overflow-x-auto rounded-ui border border-line bg-sub bg-[radial-gradient(var(--color-line)_1px,transparent_1px)] bg-size-[16px_16px]"
    >
      <div className="relative" style={{ width, height }}>
        <svg className="absolute inset-0" width={width} height={height} aria-hidden>
          <defs>
            <marker
              id="flow-arrow"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="8"
              markerHeight="8"
              orient="auto"
            >
              <path d="M0 0L10 5L0 10z" fill={muted} />
            </marker>
            <marker
              id="flow-arrow-danger"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="8"
              markerHeight="8"
              orient="auto"
            >
              <path d="M0 0L10 5L0 10z" fill={danger} />
            </marker>
          </defs>
          {edges.map((edge) => (
            <path
              key={edge.key}
              d={edge.path.d}
              stroke={muted}
              strokeWidth="1.5"
              fill="none"
              markerEnd={edge.danger ? "url(#flow-arrow-danger)" : "url(#flow-arrow)"}
            />
          ))}
          {edges.map((edge) =>
            edge.label ? (
              <text
                key={`${edge.key}-label`}
                x={edge.path.labelX}
                y={edge.path.labelY}
                fontSize="12"
                fill={edge.danger ? danger : muted}
              >
                {edge.label}
              </text>
            ) : null,
          )}
        </svg>
        {nodes.map((node) => {
          const kind = nodeKind(node.state)
          const first = node.state.name === startAt
          const last = node.state.transitions.length === 0
          return (
            <button
              key={node.state.name}
              type="button"
              onClick={() => onSelect(node.state.name)}
              className={clsx(
                "absolute rounded-ui border bg-bg px-[11px] py-[9px] text-left",
                node.state.name === selected
                  ? "border-ink shadow-[0_0_0_1px_var(--color-ink)]"
                  : "border-line2",
              )}
              style={{ left: node.x, top: node.y, width: node.width, height: node.height }}
            >
              {first ? null : (
                <span className="absolute -left-1 top-[34px] size-[7px] rounded-full border-[1.5px] border-faint bg-bg" />
              )}
              {last ? null : (
                <span className="absolute -right-1 top-[34px] size-[7px] rounded-full border-[1.5px] border-faint bg-bg" />
              )}
              <div className="flex items-center gap-1.5 font-mono text-xs font-medium text-muted">
                <Icon icon={kind.icon} className="size-[13px]" />
                {kind.label}
              </div>
              <div className="my-0.5 font-medium">{node.state.name}</div>
              <div className="text-sm text-muted wrap-anywhere">{node.state.summary}</div>
              {node.lines.length > 0 ? (
                <div className="mt-2 flex flex-col gap-0.5 border-t border-line pt-1.5">
                  {node.lines}
                </div>
              ) : null}
            </button>
          )
        })}
      </div>
    </div>
  )
}
