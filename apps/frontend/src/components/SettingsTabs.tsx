import { Link } from "@tanstack/react-router"
import type { LucideIcon } from "lucide-react"
import { CreditCard, FolderGit2, GitBranch, KeyRound, ListChecks, Users } from "lucide-react"
import { type ReactNode, useLayoutEffect, useRef } from "react"
import { type MessageKey, t } from "../i18n/index.ts"
import { Page, Pg } from "./Shell.tsx"
import { Icon } from "./ui.tsx"

export type SettingsTab =
  | "integrations"
  | "config"
  | "credentials"
  | "members"
  | "audit"
  | "billing"

const tabs = [
  { id: "integrations", to: "/settings", label: "settings.integrations", icon: GitBranch },
  { id: "config", to: "/settings/config", label: "settings.config", icon: FolderGit2 },
  { id: "credentials", to: "/settings/credentials", label: "settings.credentials", icon: KeyRound },
  { id: "members", to: "/settings/members", label: "settings.members", icon: Users },
  { id: "audit", to: "/settings/audit", label: "settings.audit", icon: ListChecks },
  { id: "billing", to: "/settings/billing", label: "settings.billing", icon: CreditCard },
] as const satisfies readonly { id: SettingsTab; to: string; label: MessageKey; icon: LucideIcon }[]

export function SettingsPage({
  tab,
  actions,
  keys = [["g s", "nav.settings"]],
  children,
}: {
  tab: SettingsTab
  actions?: ReactNode
  keys?: [string, MessageKey][]
  children: ReactNode
}) {
  const current = tabs.find((item) => item.id === tab) ?? tabs[0]
  const strip = useRef<HTMLDivElement>(null)
  const active = useRef<HTMLSpanElement>(null)

  useLayoutEffect(() => {
    if (strip.current && active.current) {
      strip.current.scrollLeft =
        active.current.offsetLeft + active.current.offsetWidth - strip.current.clientWidth
    }
  }, [tab])

  return (
    <Page
      crumbs={[
        { label: t("nav.settings"), to: tab === "integrations" ? undefined : "/settings" },
        { label: t(current.label) },
      ]}
      actions={actions}
      keys={keys}
    >
      <Pg>
        <div ref={strip} data-scroll-x className="relative mb-4 overflow-x-auto">
          <div className="flex h-[34px] w-max min-w-full items-end gap-[18px] border-b border-line">
            {tabs.map((item) =>
              item.id === tab ? (
                <span
                  key={item.id}
                  ref={active}
                  aria-current="page"
                  className="-mb-px flex items-center gap-1.5 whitespace-nowrap border-b-2 border-ink pb-2 font-medium text-ink"
                >
                  <Icon icon={item.icon} />
                  {t(item.label)}
                </span>
              ) : (
                <Link
                  key={item.id}
                  to={item.to}
                  className="flex items-center gap-1.5 whitespace-nowrap pb-2 text-muted hover:text-ink"
                >
                  <Icon icon={item.icon} />
                  {t(item.label)}
                </Link>
              ),
            )}
          </div>
        </div>
        {children}
      </Pg>
    </Page>
  )
}
