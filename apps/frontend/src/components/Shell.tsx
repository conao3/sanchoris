import { Link, useNavigate, useRouterState } from '@tanstack/react-router';
import clsx from 'clsx';
import type { LucideIcon } from 'lucide-react';
import { Check, ChevronsUpDown, House, ListChecks, MessageSquare, Plus, Puzzle, Settings, ShieldCheck, Zap } from 'lucide-react';
import { type ReactNode, useState } from 'react';
import { type MessageKey, t } from '../i18n';
import { useKeys } from '../lib/keys';
import { useTenant } from '../lib/tenant';
import { Icon, Kbd } from './ui';

type MenuItem = { to: string; label: MessageKey; icon: LucideIcon; match: (path: string) => boolean };

const topMenu: MenuItem[] = [
  { to: '/', label: 'nav.home', icon: House, match: (path) => path === '/' },
  { to: '/triggers', label: 'nav.triggers', icon: Zap, match: (path) => path.startsWith('/triggers') },
  { to: '/skills', label: 'nav.skills', icon: Puzzle, match: (path) => path.startsWith('/skills') },
  { to: '/chat', label: 'nav.chat', icon: MessageSquare, match: (path) => path.startsWith('/chat') },
  { to: '/tasks', label: 'nav.tasks', icon: ListChecks, match: (path) => path.startsWith('/tasks') },
];

const operatorItem: MenuItem = {
  to: '/operator/tenants',
  label: 'nav.tenants',
  icon: ShieldCheck,
  match: (path) => path.startsWith('/operator'),
};

const settingsItem: MenuItem = {
  to: '/settings',
  label: 'nav.settings',
  icon: Settings,
  match: (path) => path.startsWith('/settings'),
};

const roleLabels: Record<string, MessageKey> = {
  OWNER: 'role.owner',
  ADMIN: 'role.admin',
  MEMBER: 'role.member',
};

function MenuLink({ item, path }: { item: MenuItem; path: string }) {
  const active = item.match(path);
  return (
    <Link
      to={item.to}
      className={clsx(
        'flex items-center gap-[9px] rounded-ui px-2 py-[5px]',
        active ? 'bg-hover font-medium text-ink' : 'text-body hover:bg-hover',
      )}
    >
      <Icon icon={item.icon} />
      {t(item.label)}
    </Link>
  );
}

function TenantSwitcher() {
  const { viewer, membership, switchTenant } = useTenant();
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex w-full items-center justify-between rounded-ui px-2 py-1.5 font-semibold hover:bg-hover"
        aria-expanded={open}
      >
        {membership.tenant.name}
        <Icon icon={ChevronsUpDown} className="ml-auto text-faint" />
      </button>
      {open ? (
        <div className="absolute left-0 right-0 top-9 z-10 rounded-[8px] border border-line2 bg-bg py-1.5">
          <div className="px-3.5 pb-1 pt-1.5 text-2xs font-semibold uppercase tracking-[.06em] text-faint">
            {t('shell.switchTenant')}
          </div>
          {viewer.memberships.map((item) => (
            <button
              key={item.id}
              type="button"
              onClick={() => {
                setOpen(false);
                switchTenant(item.tenant.slug);
              }}
              className="flex w-full items-center gap-2 px-3.5 py-[7px] text-left hover:bg-hover"
            >
              {item.tenant.name}
              {item.id === membership.id ? <Icon icon={Check} className="ml-auto" /> : null}
            </button>
          ))}
          <Link to="/tenants/new" className="flex items-center gap-2 border-t border-line px-3.5 py-[7px] text-muted hover:bg-hover">
            <Icon icon={Plus} />
            {t('nav.createTenant')}
          </Link>
        </div>
      ) : null}
    </div>
  );
}

function Sidebar() {
  const { viewer, membership } = useTenant();
  const path = useRouterState({ select: (state) => state.location.pathname });
  return (
    <nav className="flex flex-col gap-0.5 border-r border-line bg-sub px-2.5 py-3">
      <TenantSwitcher />
      <div className="h-2" />
      {topMenu.map((item) => (
        <MenuLink key={item.to} item={item} path={path} />
      ))}
      <div className="flex-1" />
      {viewer.isOperator ? <MenuLink item={operatorItem} path={path} /> : null}
      <MenuLink item={settingsItem} path={path} />
      <div className="px-2 py-1.5 text-sm text-muted">
        {viewer.user.name} · {t(roleLabels[membership.role])}
      </div>
    </nav>
  );
}

export function AppFrame({ children }: { children: ReactNode }) {
  const navigate = useNavigate();
  useKeys({
    'g h': () => void navigate({ to: '/' }),
    'g t': () => void navigate({ to: '/tasks' }),
    'g r': () => void navigate({ to: '/triggers' }),
    'g s': () => void navigate({ to: '/settings' }),
  });
  return (
    <div className="grid h-screen grid-cols-[208px_1fr] overflow-hidden">
      <Sidebar />
      {children}
    </div>
  );
}

export type Crumb = { label: string; to?: string; params?: Record<string, string> };

export function Page({
  crumbs,
  badge,
  actions,
  keys,
  children,
}: {
  crumbs: Crumb[];
  badge?: ReactNode;
  actions?: ReactNode;
  keys: [string, MessageKey][];
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-0 min-w-0 flex-col">
      <header className="flex h-11 flex-none items-center justify-between border-b border-line px-6">
        <div className="flex items-center gap-[7px] text-muted">
          {crumbs.map((crumb, index) => (
            <span key={index} className="contents">
              {index > 0 ? <span>/</span> : null}
              {index === crumbs.length - 1 ? (
                <b className="font-medium text-ink">{crumb.label}</b>
              ) : crumb.to ? (
                <Link to={crumb.to} params={crumb.params} className="hover:text-ink">
                  {crumb.label}
                </Link>
              ) : (
                <span>{crumb.label}</span>
              )}
            </span>
          ))}
          {badge}
        </div>
        <div className="flex items-center gap-2">{actions}</div>
      </header>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      <footer className="flex h-[30px] flex-none items-center gap-[18px] border-t border-line bg-sub px-6 text-sm text-muted">
        {keys.map(([key, label]) => (
          <span key={key} className="inline-flex items-center gap-1.5">
            <Kbd>{key}</Kbd>
            {t(label)}
          </span>
        ))}
      </footer>
    </div>
  );
}

export function Pg({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={clsx('min-h-0 flex-1 overflow-auto px-6 py-5', className)}>{children}</div>;
}

export function Cols({ main, aside }: { main: ReactNode; aside: ReactNode }) {
  return (
    <div className="grid min-h-0 flex-1 grid-cols-[1fr_300px]">
      <div className="min-h-0 min-w-0 overflow-auto px-6 py-5">{main}</div>
      <aside className="min-h-0 overflow-auto border-l border-line px-[22px] py-5">{aside}</aside>
    </div>
  );
}
