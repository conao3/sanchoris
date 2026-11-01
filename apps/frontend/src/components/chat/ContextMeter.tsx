import clsx from "clsx"
import { locale, t } from "../../i18n/index.ts"
import { percent } from "../../lib/format.ts"

const RADIUS = 6
const CIRCUMFERENCE = 2 * Math.PI * RADIUS
const tokenFormat = new Intl.NumberFormat(locale)

export type AutoCompact = { idleMinutes: number; contextPercent: number }

export function ContextMeter({
  context,
  autoCompact,
}: {
  context: { usedTokens: number; maxTokens: number }
  autoCompact: AutoCompact | null
}) {
  const ratio = Math.min(1, context.usedTokens / context.maxTokens)
  const due = autoCompact !== null && ratio * 100 >= autoCompact.contextPercent
  const hint = [
    t("context.hint", {
      used: tokenFormat.format(context.usedTokens),
      max: tokenFormat.format(context.maxTokens),
    }),
    autoCompact
      ? t("context.autoCompact", {
          minutes: autoCompact.idleMinutes,
          percent: autoCompact.contextPercent,
        })
      : t("context.autoCompactOff"),
  ].join("\n")
  return (
    <span
      role="img"
      aria-label={t("context.label", { percent: percent(ratio) })}
      title={hint}
      className={clsx(
        "flex items-center gap-1 text-xs tabular-nums",
        due ? "text-warn" : "text-muted",
      )}
    >
      <svg viewBox="0 0 16 16" className="size-3.5 -rotate-90" aria-hidden>
        <circle cx="8" cy="8" r={RADIUS} fill="none" strokeWidth="2" className="stroke-line2" />
        <circle
          cx="8"
          cy="8"
          r={RADIUS}
          fill="none"
          strokeWidth="2"
          strokeDasharray={CIRCUMFERENCE}
          strokeDashoffset={CIRCUMFERENCE * (1 - ratio)}
          className="stroke-current"
        />
      </svg>
      {percent(ratio)}
    </span>
  )
}
