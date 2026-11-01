import clsx from "clsx"
import { ChevronRight, FileDiff } from "lucide-react"
import { useState } from "react"
import type { ChatQuery } from "../../graphql/generated/graphql.ts"
import { t } from "../../i18n/index.ts"
import { CodeBlock } from "./CodeBlock.tsx"

type Diff = NonNullable<
  NonNullable<NonNullable<ChatQuery["tenant"]>["conversation"]>["messages"][number]["diff"]
>

function Count({ additions, deletions }: { additions: number; deletions: number }) {
  return (
    <span className="shrink-0 font-mono text-xs">
      <span className="text-ok">+{additions}</span> <span className="text-err">−{deletions}</span>
    </span>
  )
}

export function ChangedFiles({ diff }: { diff: Diff }) {
  const [open, setOpen] = useState(false)
  const repos = new Set(diff.files.map((file) => file.repo))
  const additions = diff.files.reduce((sum, file) => sum + (file.additions ?? 0), 0)
  const deletions = diff.files.reduce((sum, file) => sum + (file.deletions ?? 0), 0)
  return (
    <div className="mt-2.5 overflow-hidden rounded-ui border border-line">
      <div className="flex items-center gap-2 border-b border-line bg-sub px-3 py-1.5 text-sm">
        <FileDiff className="size-3.5 shrink-0 text-muted" aria-hidden />
        <span className="font-medium text-ink">
          {t(diff.files.length === 1 ? "diff.changedOne" : "diff.changed", {
            n: diff.files.length,
          })}
        </span>
        <Count additions={additions} deletions={deletions} />
      </div>
      <ul className="py-1">
        {diff.files.map((file) => (
          <li
            key={`${file.repo}/${file.path}`}
            className="flex items-center justify-between gap-3 px-3 py-0.5 text-sm"
          >
            <span className="min-w-0 truncate font-mono text-xs text-ink">
              {repos.size > 1 ? `${file.repo}/${file.path}` : file.path}
            </span>
            {file.additions === null || file.deletions === null ? (
              <span className="shrink-0 text-xs text-faint">{t("diff.binary")}</span>
            ) : (
              <Count additions={file.additions} deletions={file.deletions} />
            )}
          </li>
        ))}
      </ul>
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-1.5 border-t border-line px-3 py-1.5 text-left text-sm text-muted outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-line2"
      >
        <ChevronRight
          className={clsx("size-3.5 shrink-0 transition-transform", open && "rotate-90")}
          aria-hidden
        />
        {open ? t("diff.hide") : t("diff.view")}
      </button>
      {open ? (
        <div className="border-t border-line px-2 pb-2">
          <CodeBlock code={diff.patch} language="diff" />
          {diff.truncated ? <p className="px-1 text-xs text-muted">{t("diff.truncated")}</p> : null}
        </div>
      ) : null}
    </div>
  )
}
