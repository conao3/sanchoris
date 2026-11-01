import clsx from "clsx"
import { Check, ChevronRight, Circle, CircleDot, ListTodo, Play } from "lucide-react"
import { useState } from "react"
import type { ChatPlanFieldsFragment } from "../../graphql/generated/graphql.ts"
import { t } from "../../i18n/index.ts"
import { Button } from "../Button.tsx"
import { CopyButton } from "./CopyButton.tsx"
import { Markdown } from "./Markdown.tsx"

const HEADING = /^#{1,6}\s+(.+)$/m

export function PlanSteps({ plan }: { plan: ChatPlanFieldsFragment }) {
  return (
    <div className="space-y-1 text-sm">
      {plan.explanation ? <p className="text-muted">{plan.explanation}</p> : null}
      <ol className="space-y-0.5">
        {plan.steps.map((step, index) => {
          const Icon =
            step.status === "COMPLETED" ? Check : step.status === "RUNNING" ? CircleDot : Circle
          return (
            <li key={index} className="flex items-start gap-2">
              <Icon
                className={clsx(
                  "mt-0.5 size-3.5 shrink-0",
                  step.status === "COMPLETED" ? "text-ok" : "text-muted",
                )}
                aria-hidden
              />
              <span
                className={clsx(
                  step.status === "COMPLETED" && "text-muted line-through",
                  step.status === "RUNNING" && "font-medium text-ink",
                )}
              >
                {step.text}
              </span>
            </li>
          )
        })}
      </ol>
    </div>
  )
}

export function TasksBanner({ plan }: { plan: ChatPlanFieldsFragment }) {
  const [open, setOpen] = useState(false)
  const done = plan.steps.filter((step) => step.status === "COMPLETED").length
  const current =
    plan.steps.find((step) => step.status === "RUNNING") ??
    plan.steps.find((step) => step.status === "PENDING")
  return (
    <div className="mt-3.5 rounded-ui border border-line">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-line2"
      >
        <ListTodo className="size-3.5 shrink-0 text-muted" aria-hidden />
        <span className="font-medium text-ink">{t("plan.tasks")}</span>
        <span className="min-w-0 flex-1 truncate text-muted">{current?.text ?? ""}</span>
        <span
          className={clsx(
            "shrink-0 font-mono text-xs",
            done === plan.steps.length ? "text-ok" : "text-muted",
          )}
        >
          {done}/{plan.steps.length}
        </span>
        <ChevronRight
          className={clsx("size-3.5 shrink-0 transition-transform", open && "rotate-90")}
          aria-hidden
        />
      </button>
      {open ? (
        <div className="border-t border-line px-3 py-2">
          <PlanSteps plan={plan} />
        </div>
      ) : null}
    </div>
  )
}

export function planTitle(markdown: string): string {
  return HEADING.exec(markdown)?.[1]?.trim() ?? t("plan.proposed")
}

export function ProposedPlanCard({
  markdown,
  onImplement,
  implementReason,
}: {
  markdown: string
  onImplement?: () => void
  implementReason?: string
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="mt-2.5 overflow-hidden rounded-ui border border-line">
      <div className="flex items-center gap-2 border-b border-line bg-sub px-3 py-1.5 text-sm">
        <span className="rounded-kbd border border-line2 px-1.5 text-xs font-semibold text-muted">
          {t("plan.badge")}
        </span>
        <span className="min-w-0 flex-1 truncate font-medium text-ink">{planTitle(markdown)}</span>
        <CopyButton text={markdown} />
      </div>
      <div className={clsx("relative px-3 py-2", !open && "max-h-48 overflow-hidden")}>
        <Markdown text={markdown} className="text-sm text-ink" />
        {!open ? (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-12 bg-linear-to-t from-bg" />
        ) : null}
      </div>
      <div className="flex items-center justify-between gap-2 border-t border-line px-2 py-1.5">
        <button
          type="button"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex items-center gap-1.5 rounded-kbd px-1 text-sm text-muted outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-line2"
        >
          <ChevronRight
            className={clsx("size-3.5 shrink-0 transition-transform", open && "rotate-90")}
            aria-hidden
          />
          {open ? t("plan.collapse") : t("plan.expand")}
        </button>
        {onImplement ? (
          <Button
            variant="primary"
            small
            icon={Play}
            reason={implementReason}
            onPress={onImplement}
          >
            {t("plan.implement")}
          </Button>
        ) : null}
      </div>
    </div>
  )
}
