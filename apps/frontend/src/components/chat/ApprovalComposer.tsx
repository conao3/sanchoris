import { useState } from "react"
import type {
  ChatApprovalDecision,
  ChatApprovalFieldsFragment,
  ChatApprovalKind,
} from "../../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../../i18n/index.ts"
import { Button } from "../Button.tsx"

const TITLES: Record<ChatApprovalKind, MessageKey> = {
  COMMAND: "approval.command",
  FILE_CHANGE: "approval.fileChange",
  TOOL: "approval.tool",
}

export function ApprovalComposer({
  approval,
  onDecide,
}: {
  approval: ChatApprovalFieldsFragment
  onDecide: (decision: ChatApprovalDecision) => Promise<boolean>
}) {
  const [sending, setSending] = useState(false)
  const busy = sending ? t("approval.sending") : undefined

  function decide(decision: ChatApprovalDecision) {
    setSending(true)
    void onDecide(decision).then((sent) => {
      if (!sent) {
        setSending(false)
      }
    })
  }

  return (
    <div className="mt-3.5 rounded-ui border border-line2">
      <div className="border-b border-line px-3 py-2.5">
        <div className="mb-1 flex items-center gap-2 text-xs text-muted">
          <span className="font-semibold tracking-wide uppercase">{t(TITLES[approval.kind])}</span>
          <span className="font-mono">{approval.toolName}</span>
        </div>
        <pre className="max-h-48 overflow-auto rounded-ui bg-term-bg p-2.5 font-mono text-xs break-words whitespace-pre-wrap text-term-fg">
          {approval.detail || approval.toolName}
        </pre>
        {approval.reason ? <p className="mt-1.5 text-sm text-muted">{approval.reason}</p> : null}
      </div>
      <div className="flex flex-wrap items-center justify-end gap-2 px-2.5 py-2">
        <Button small variant="danger" reason={busy} onPress={() => decide("CANCEL")}>
          {t("approval.cancel")}
        </Button>
        <Button small reason={busy} onPress={() => decide("DECLINE")}>
          {t("approval.decline")}
        </Button>
        <Button small reason={busy} onPress={() => decide("ACCEPT_FOR_SESSION")}>
          {t("approval.acceptSession")}
        </Button>
        <Button variant="primary" small reason={busy} onPress={() => decide("ACCEPT")}>
          {t("approval.accept")}
        </Button>
      </div>
    </div>
  )
}
