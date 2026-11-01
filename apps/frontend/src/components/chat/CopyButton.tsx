import { Check, Copy } from "lucide-react"
import { useEffect, useState } from "react"
import { t } from "../../i18n/index.ts"

const COPIED_MS = 1500

export function CopyButton({ text }: { text: string }) {
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
