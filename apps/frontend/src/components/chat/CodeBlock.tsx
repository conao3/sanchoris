import { useEffect, useState } from "react"
import { CopyButton } from "./CopyButton.tsx"

const THEME = "github-light"

function highlight(code: string, language: string): Promise<string> {
  return import("shiki").then(({ bundledLanguages, codeToHtml }) =>
    codeToHtml(code, { lang: language in bundledLanguages ? language : "text", theme: THEME }),
  )
}

export function CodeBlock({ code, language }: { code: string; language: string }) {
  const [html, setHtml] = useState<{ key: string; html: string }>()
  const key = `${language}\n${code}`
  useEffect(() => {
    let live = true
    void highlight(code, language).then((result) => {
      if (live) {
        setHtml({ key, html: result })
      }
    })
    return () => {
      live = false
    }
  }, [code, language, key])
  return (
    <div className="group/code my-2 overflow-hidden rounded-ui border border-line bg-sub">
      <div className="flex items-center justify-between gap-2 py-1 pr-1.5 pl-3 select-none">
        <span className="font-mono text-xs text-muted">{language}</span>
        <span className="opacity-0 transition-opacity group-hover/code:opacity-100 group-focus-within/code:opacity-100 pointer-coarse:opacity-100">
          <CopyButton text={code} />
        </span>
      </div>
      {html?.key === key ? (
        <div
          className="overflow-x-auto px-3 pb-2.5 font-mono text-xs [&_pre]:!bg-transparent"
          dangerouslySetInnerHTML={{ __html: html.html }}
        />
      ) : (
        <pre className="overflow-x-auto px-3 pb-2.5 font-mono text-xs text-ink">{code}</pre>
      )}
    </div>
  )
}
