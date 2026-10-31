import { Link, useNavigate, useRouterState } from "@tanstack/react-router"
import clsx from "clsx"
import type { LucideIcon } from "lucide-react"
import {
  Check,
  ChevronsUpDown,
  House,
  ListChecks,
  LogOut,
  Menu,
  MessageSquare,
  Plus,
  Puzzle,
  Settings,
  ShieldCheck,
  Zap,
} from "lucide-react"
import { createContext, type ReactNode, useContext, useState } from "react"
import { Dialog, Modal, ModalOverlay, Button as RACButton } from "react-aria-components"
import { type Role, SignOutDocument } from "../graphql/generated/graphql.ts"
import { type MessageKey, t } from "../i18n/index.ts"
import { signOut } from "../lib/cognito.ts"
import { useKeys } from "../lib/keys.ts"
import { useAction } from "../lib/mutation.ts"
import { useTenant } from "../lib/tenant.tsx"
import { Icon, Kbd } from "./ui.tsx"

type MenuItem = {
  to: string
  label: MessageKey
  icon: LucideIcon
  match: (path: string) => boolean
}

const topMenu: MenuItem[] = [
  { to: "/", label: "nav.home", icon: House, match: (path) => path === "/" },
  {
    to: "/triggers",
    label: "nav.triggers",
    icon: Zap,
    match: (path) => path.startsWith("/triggers"),
  },
  { to: "/skills", label: "nav.skills", icon: Puzzle, match: (path) => path.startsWith("/skills") },
  {
    to: "/chat",
    label: "nav.chat",
    icon: MessageSquare,
    match: (path) => path.startsWith("/chat"),
  },
  {
    to: "/tasks",
    label: "nav.tasks",
    icon: ListChecks,
    match: (path) => path.startsWith("/tasks"),
  },
]

const operatorItem: MenuItem = {
  to: "/operator/tenants",
  label: "nav.tenants",
  icon: ShieldCheck,
  match: (path) => path.startsWith("/operator"),
}

const settingsItem: MenuItem = {
  to: "/settings",
  label: "nav.settings",
  icon: Settings,
  match: (path) => path.startsWith("/settings"),
}

const roleLabels: Record<Role, MessageKey> = {
  OWNER: "role.owner",
  ADMIN: "role.admin",
  MEMBER: "role.member",
}

function MenuLink({ item, path }: { item: MenuItem; path: string }) {
  const active = item.match(path)
  const className = clsx(
    "flex items-center gap-[9px] rounded-ui px-2 py-[5px] max-lg:min-h-10",
    active ? "bg-hover font-medium text-ink" : "text-body hover:bg-hover",
  )
  const content = (
    <>
      <Icon icon={item.icon} />
      {t(item.label)}
    </>
  )
  if (path === item.to) {
    return (
      <span aria-current="page" className={className}>
        {content}
      </span>
    )
  }
  return (
    <Link to={item.to} className={className}>
      {content}
    </Link>
  )
}

function TenantSwitcher() {
  const { viewer, membership, switchTenant } = useTenant()
  const [open, setOpen] = useState(false)
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between rounded-ui px-2 py-1.5 font-semibold hover:bg-hover max-lg:min-h-10"
        aria-expanded={open}
      >
        {membership.tenant.name}
        <Icon icon={ChevronsUpDown} className="ml-auto text-faint" />
      </button>
      {open ? (
        <div className="absolute left-0 right-0 top-9 z-10 rounded-[8px] border border-line2 bg-bg py-1.5">
          <div className="px-3.5 pb-1 pt-1.5 text-xs font-semibold uppercase tracking-[.06em] text-faint">
            {t("shell.switchTenant")}
          </div>
          {viewer.memberships.map((item) =>
            item.id === membership.id ? (
              <div
                key={item.id}
                aria-current="true"
                className="flex w-full items-center gap-2 px-3.5 py-[7px]"
              >
                {item.tenant.name}
                <Icon icon={Check} className="ml-auto" />
              </div>
            ) : (
              <button
                key={item.id}
                type="button"
                onClick={() => {
                  setOpen(false)
                  switchTenant(item.tenant.slug)
                }}
                className="flex w-full items-center gap-2 px-3.5 py-[7px] text-left hover:bg-hover"
              >
                {item.tenant.name}
              </button>
            ),
          )}
          <Link
            to="/tenants/new"
            className="flex items-center gap-2 border-t border-line px-3.5 py-[7px] text-muted hover:bg-hover"
          >
            <Icon icon={Plus} />
            {t("nav.createTenant")}
          </Link>
        </div>
      ) : null}
    </div>
  )
}

function UserMenu() {
  const { viewer, membership } = useTenant()
  const [open, setOpen] = useState(false)
  const [recordSignOut] = useAction(SignOutDocument)
  return (
    <div className="relative">
      {open ? (
        <div className="absolute bottom-9 left-0 right-0 z-10 rounded-[8px] border border-line2 bg-bg py-1.5">
          <div className="truncate px-3.5 pb-1 pt-1.5 text-xs text-muted">{viewer.user.email}</div>
          <button
            type="button"
            onClick={() => void signOut(() => recordSignOut({}))}
            className="flex w-full items-center gap-2 border-t border-line px-3.5 py-[7px] text-left hover:bg-hover"
          >
            <Icon icon={LogOut} />
            {t("shell.signOut")}
          </button>
        </div>
      ) : null}
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center rounded-ui px-2 py-1.5 text-left text-sm text-muted hover:bg-hover max-lg:min-h-10"
        aria-expanded={open}
      >
        {viewer.user.name} · {t(roleLabels[membership.role])}
        <Icon icon={ChevronsUpDown} className="ml-auto text-faint" />
      </button>
    </div>
  )
}

function Sidebar({ className }: { className?: string }) {
  const { viewer } = useTenant()
  const path = useRouterState({ select: (state) => state.location.pathname })
  return (
    <nav
      className={clsx("flex flex-col gap-0.5 border-r border-line bg-sub px-2.5 py-3", className)}
    >
      <TenantSwitcher />
      <div className="h-2" />
      {topMenu.map((item) => (
        <MenuLink key={item.to} item={item} path={path} />
      ))}
      <div className="flex-1" />
      {viewer.isOperator ? <MenuLink item={operatorItem} path={path} /> : null}
      <MenuLink item={settingsItem} path={path} />
      <UserMenu />
    </nav>
  )
}

const NavDrawerContext = createContext<() => void>(() => {})

function NavDrawer({
  isOpen,
  onOpenChange,
}: {
  isOpen: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className="fixed inset-0 z-30 bg-ink/20 lg:hidden"
    >
      <Modal className="h-full w-[232px] outline-none">
        <Dialog aria-label={t("shell.navigation")} className="h-full outline-none">
          <Sidebar className="h-full overflow-auto" />
        </Dialog>
      </Modal>
    </ModalOverlay>
  )
}

export function AppFrame({ children }: { children: ReactNode }) {
  const navigate = useNavigate()
  const locationKey = useRouterState({ select: (state) => state.location.state.__TSR_key })
  const [drawerKey, setDrawerKey] = useState<string>()
  const drawerOpen = drawerKey !== undefined && drawerKey === locationKey
  useKeys({
    "g h": () => void navigate({ to: "/" }),
    "g t": () => void navigate({ to: "/tasks" }),
    "g r": () => void navigate({ to: "/triggers" }),
    "g s": () => void navigate({ to: "/settings" }),
  })
  return (
    <NavDrawerContext.Provider value={() => setDrawerKey(locationKey)}>
      <div className="grid h-screen grid-cols-1 overflow-hidden lg:grid-cols-[232px_1fr]">
        <Sidebar className="max-lg:hidden" />
        {children}
      </div>
      <NavDrawer
        isOpen={drawerOpen}
        onOpenChange={(open) => setDrawerKey(open ? locationKey : undefined)}
      />
    </NavDrawerContext.Provider>
  )
}

export type Crumb = { label: string; to?: string; params?: Record<string, string> }

export function Page({
  crumbs,
  badge,
  actions,
  keys,
  children,
}: {
  crumbs: Crumb[]
  badge?: ReactNode
  actions?: ReactNode
  keys: [string, MessageKey][]
  children: ReactNode
}) {
  const openDrawer = useContext(NavDrawerContext)
  return (
    <div className="flex min-h-0 min-w-0 flex-col">
      <header className="flex h-11 flex-none items-center justify-between gap-3 border-b border-line px-4 md:px-6">
        <RACButton
          aria-label={t("shell.openNavigation")}
          onPress={openDrawer}
          className="-ml-1.5 inline-flex size-8 flex-none items-center justify-center rounded-ui text-body outline-none hover:bg-hover focus-visible:ring-2 focus-visible:ring-line2 lg:hidden"
        >
          <Icon icon={Menu} />
        </RACButton>
        <div className="flex min-w-0 flex-1 items-center gap-[7px] text-muted">
          {crumbs.map((crumb, index) => (
            <span key={index} className="contents">
              {index > 0 ? <span className="flex-none">/</span> : null}
              {index === crumbs.length - 1 ? (
                <b className="min-w-0 truncate font-medium text-ink">{crumb.label}</b>
              ) : crumb.to ? (
                <Link
                  to={crumb.to}
                  params={crumb.params}
                  className="min-w-0 truncate hover:text-ink max-md:max-w-24"
                >
                  {crumb.label}
                </Link>
              ) : (
                <span className="min-w-0 truncate max-md:max-w-24">{crumb.label}</span>
              )}
            </span>
          ))}
          {badge ? <span className="flex-none">{badge}</span> : null}
        </div>
        <div className="flex flex-none items-center gap-2">{actions}</div>
      </header>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      <footer className="flex h-[30px] flex-none items-center gap-[18px] border-t border-line bg-sub px-6 text-xs text-muted max-md:hidden">
        {keys.map(([key, label]) => (
          <span key={key} className="inline-flex items-center gap-1.5">
            <Kbd>{key}</Kbd>
            {t(label)}
          </span>
        ))}
      </footer>
    </div>
  )
}

export function Pg({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={clsx("min-h-0 flex-1 overflow-auto px-4 py-5 md:px-6", className)}>
      {children}
    </div>
  )
}

export function Cols({ main, aside }: { main: ReactNode; aside: ReactNode }) {
  return (
    <div className="grid min-h-0 flex-1 grid-cols-1 max-lg:content-start max-lg:overflow-auto lg:grid-cols-[1fr_340px]">
      <div className="min-w-0 px-4 py-5 md:px-6 lg:min-h-0 lg:overflow-auto">{main}</div>
      <aside className="min-w-0 border-line px-4 py-5 max-lg:border-t md:px-6 lg:min-h-0 lg:overflow-auto lg:border-l lg:px-[22px]">
        {aside}
      </aside>
    </div>
  )
}
