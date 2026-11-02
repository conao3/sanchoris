import { CornerDownRight, ListOrdered, X } from "lucide-react"
import { t } from "../../i18n/index.ts"
import { Button, IconButton } from "../Button.tsx"

type Queued = { id: string; text: string }

export function QueuedBanner({
  messages,
  working,
  onSteer,
  onRemove,
}: {
  messages: Queued[]
  working: boolean
  onSteer: (id: string) => void
  onRemove: (id: string) => void
}) {
  return (
    <div className="mt-3.5 rounded-ui border border-line">
      <div className="flex items-center gap-2 border-b border-line px-3 py-1.5 text-sm">
        <ListOrdered className="size-3.5 shrink-0 text-muted" aria-hidden />
        <span className="font-medium text-ink">{t("queue.title")}</span>
        <span className="text-xs text-muted">{messages.length}</span>
      </div>
      <ul>
        {messages.map((message) => (
          <li
            key={message.id}
            className="flex items-center gap-2 border-b border-line px-3 py-1.5 last:border-b-0"
          >
            <span className="min-w-0 flex-1 truncate text-sm text-ink" title={message.text}>
              {message.text}
            </span>
            <Button
              small
              icon={CornerDownRight}
              tip={working ? t("queue.steerHint") : t("queue.sendNowHint")}
              reasonAlign="right"
              onPress={() => onSteer(message.id)}
            >
              {working ? t("queue.steer") : t("queue.sendNow")}
            </Button>
            <IconButton
              icon={X}
              label={t("queue.remove")}
              align="right"
              onPress={() => onRemove(message.id)}
            />
          </li>
        ))}
      </ul>
    </div>
  )
}
