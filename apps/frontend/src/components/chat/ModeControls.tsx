import { Hammer, ListChecks, Lock, LockOpen, PenLine } from "lucide-react"
import type { ChatInteractionMode, ChatRuntimeMode } from "../../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../../i18n/index.ts"
import { Button } from "../Button.tsx"
import { Menu } from "../Menu.tsx"

export type Modes = { interactionMode: ChatInteractionMode; runtimeMode: ChatRuntimeMode }

export const DEFAULT_MODES: Modes = { interactionMode: "DEFAULT", runtimeMode: "FULL_ACCESS" }

const RUNTIME: Record<
  ChatRuntimeMode,
  { label: MessageKey; detail: MessageKey; icon: typeof Lock }
> = {
  APPROVAL_REQUIRED: { label: "mode.supervised", detail: "mode.supervisedDetail", icon: Lock },
  AUTO_ACCEPT_EDITS: { label: "mode.acceptEdits", detail: "mode.acceptEditsDetail", icon: PenLine },
  FULL_ACCESS: { label: "mode.fullAccess", detail: "mode.fullAccessDetail", icon: LockOpen },
}

const RUNTIME_ORDER: ChatRuntimeMode[] = ["APPROVAL_REQUIRED", "AUTO_ACCEPT_EDITS", "FULL_ACCESS"]

export function ModeControls({
  modes,
  onChange,
}: {
  modes: Modes
  onChange: (modes: Modes) => void
}) {
  const plan = modes.interactionMode === "PLAN"
  const runtime = RUNTIME[modes.runtimeMode]
  return (
    <div className="flex items-center gap-1.5">
      <span title={plan ? t("mode.planHint") : t("mode.buildHint")}>
        <Button
          small
          icon={plan ? ListChecks : Hammer}
          onPress={() => onChange({ ...modes, interactionMode: plan ? "DEFAULT" : "PLAN" })}
        >
          {plan ? t("mode.plan") : t("mode.build")}
        </Button>
      </span>
      <Menu
        compact
        icon={runtime.icon}
        label={t(runtime.label)}
        title={t("mode.runtime")}
        value={modes.runtimeMode}
        options={RUNTIME_ORDER.map((value) => ({
          value,
          label: t(RUNTIME[value].label),
          detail: t(RUNTIME[value].detail),
        }))}
        onChange={(runtimeMode) => onChange({ ...modes, runtimeMode })}
      />
    </div>
  )
}
