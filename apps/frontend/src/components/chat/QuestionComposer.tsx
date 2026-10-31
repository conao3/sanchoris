import clsx from "clsx"
import { Check, Square } from "lucide-react"
import { type KeyboardEvent, useEffect, useRef, useState } from "react"
import type { ChatQuestionFieldsFragment } from "../../graphql/generated/graphql.ts"
import { t } from "../../i18n/index.ts"
import { useKeys } from "../../lib/keys.ts"
import { Button } from "../Button.tsx"
import { Kbd } from "../ui.tsx"
import { Markdown } from "./Markdown.tsx"

type Answers = { questionId: string; answers: string[] }[]

const ADVANCE_DELAY_MS = 200

export function QuestionComposer({
  question,
  onSubmit,
  onStop,
}: {
  question: ChatQuestionFieldsFragment
  onSubmit: (answers: Answers) => Promise<boolean>
  onStop: () => void
}) {
  const [index, setIndex] = useState(0)
  const [picked, setPicked] = useState<Record<string, string[]>>({})
  const [typed, setTyped] = useState<Record<string, string>>({})
  const [submitting, setSubmitting] = useState(false)
  const advanceTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => () => clearTimeout(advanceTimer.current), [])
  const items = question.items
  const item = items[index]
  const last = index === items.length - 1

  function resolved(id: string, typedNow = typed, pickedNow = picked): string[] {
    const own = typedNow[id]?.trim()
    return own ? [own] : (pickedNow[id] ?? [])
  }

  function advance(typedNow = typed, pickedNow = picked) {
    if (!item || resolved(item.id, typedNow, pickedNow).length === 0 || submitting) {
      return
    }
    if (!last) {
      setIndex(index + 1)
      return
    }
    setSubmitting(true)
    void onSubmit(
      items.map((entry) => ({
        questionId: entry.id,
        answers: resolved(entry.id, typedNow, pickedNow),
      })),
    ).then((sent) => {
      if (!sent) {
        setSubmitting(false)
      }
    })
  }

  function select(label: string) {
    if (!item) {
      return
    }
    const current = picked[item.id] ?? []
    const next = item.multiSelect
      ? current.includes(label)
        ? current.filter((value) => value !== label)
        : [...current, label]
      : [label]
    const pickedNow = { ...picked, [item.id]: next }
    setPicked(pickedNow)
    if (!item.multiSelect && !typed[item.id]?.trim()) {
      clearTimeout(advanceTimer.current)
      advanceTimer.current = setTimeout(() => advance(typed, pickedNow), ADVANCE_DELAY_MS)
    }
  }

  useKeys(
    Object.fromEntries(
      (item?.options ?? [])
        .slice(0, 9)
        .map((option, position) => [String(position + 1), () => select(option.label)]),
    ),
  )

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (!item) {
      return
    }
    const own = typed[item.id] ?? ""
    const position = Number(event.key)
    if (own === "" && Number.isInteger(position) && position >= 1 && position <= 9) {
      const option = item.options[position - 1]
      if (option) {
        event.preventDefault()
        select(option.label)
      }
      return
    }
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault()
      advance()
    }
  }

  if (!item) {
    return null
  }
  const selected = picked[item.id] ?? []
  return (
    <div className="mt-3.5 rounded-ui border border-line2">
      <div className="border-b border-line px-3 py-2.5">
        <div className="mb-1 flex items-center gap-2 text-xs text-muted">
          <span className="font-semibold tracking-wide uppercase">
            {item.header || t("question.header")}
          </span>
          {items.length > 1 ? (
            <span>{t("question.counter", { n: index + 1, total: items.length })}</span>
          ) : null}
        </div>
        <Markdown text={item.question} className="font-medium text-ink" />
        {item.multiSelect ? (
          <div className="mt-0.5 text-xs text-muted">{t("question.multi")}</div>
        ) : null}
        <div className="mt-2 space-y-1">
          {item.options.map((option, position) => {
            const on = selected.includes(option.label)
            return (
              <button
                key={option.label}
                type="button"
                aria-pressed={on}
                onClick={() => select(option.label)}
                className={clsx(
                  "flex w-full items-start gap-2.5 rounded-ui border px-2.5 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-line2",
                  on ? "border-ink bg-hover" : "border-line hover:bg-hover",
                )}
              >
                {position < 9 ? <Kbd>{position + 1}</Kbd> : null}
                <span className="min-w-0 flex-1">
                  <span className="block font-medium text-ink">{option.label}</span>
                  {option.description && option.description !== option.label ? (
                    <span className="block text-sm text-muted">{option.description}</span>
                  ) : null}
                </span>
                {on ? <Check className="mt-0.5 size-4 shrink-0 text-ink" aria-hidden /> : null}
              </button>
            )
          })}
        </div>
      </div>
      <div className="flex items-start justify-between gap-3 px-2.5 py-2">
        <textarea
          rows={1}
          value={typed[item.id] ?? ""}
          onChange={(event) => setTyped({ ...typed, [item.id]: event.target.value })}
          onKeyDown={onKeyDown}
          placeholder={
            item.options.length > 0 ? t("question.otherPlaceholder") : t("question.typePlaceholder")
          }
          aria-label={t("question.otherPlaceholder")}
          className="min-h-5 flex-1 resize-none bg-transparent outline-none placeholder:text-faint"
        />
        <Button small icon={Square} onPress={onStop}>
          {t("chat.stop")}
        </Button>
        <Button
          variant="primary"
          small
          reason={
            submitting
              ? t("question.submitting")
              : resolved(item.id).length === 0
                ? t("reason.questionUnanswered")
                : undefined
          }
          onPress={() => advance()}
        >
          {submitting
            ? t("question.submitting")
            : !last
              ? t("question.next")
              : items.length > 1
                ? t("question.submitAll")
                : t("question.submit")}
        </Button>
      </div>
    </div>
  )
}
