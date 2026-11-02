import { Check, Copy } from "lucide-react"
import { useEffect, useState } from "react"
import { t } from "../../i18n/index.ts"
import { IconButton } from "../Button.tsx"

const COPIED_MS = 1500

export function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    const timer = copied ? window.setTimeout(() => setCopied(false), COPIED_MS) : undefined
    return () => window.clearTimeout(timer)
  }, [copied])
  return (
    <IconButton
      icon={copied ? Check : Copy}
      label={copied ? t("chat.copied") : t("chat.copy")}
      onPress={() => void navigator.clipboard.writeText(text).then(() => setCopied(true))}
    />
  )
}
