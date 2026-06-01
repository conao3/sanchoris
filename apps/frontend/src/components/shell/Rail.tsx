import { Link, useRouterState } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { Kbd } from './primitives';

export type RailItemDef = {
  key: string;
  icon: ReactNode;
  label: string;
  badge?: string;
  badgeTone?: 'default' | 'coral';
  href?: string;
  active?: boolean;
};

function RailSection({ children }: { children: ReactNode }) {
  return (
    <div className="px-3 pb-[6px] pt-[14px] text-[10.5px] font-semibold uppercase tracking-[1.5px] text-muted-soft">
      {children}
    </div>
  );
}

function RailItem({
  icon,
  label,
  badge,
  badgeTone = 'default',
  active,
  href,
}: {
  icon: ReactNode;
  label: string;
  badge?: string;
  badgeTone?: 'default' | 'coral';
  active?: boolean;
  href?: string;
}) {
  const badgeClass =
    badgeTone === 'coral'
      ? 'bg-primary text-white border-primary'
      : 'border border-hairline bg-canvas text-muted';

  const inner = (
    <>
      <span
        className={`inline-flex h-4 w-4 flex-shrink-0 items-center justify-center ${
          active ? 'text-primary' : 'text-muted'
        }`}
      >
        {icon}
      </span>
      <span className="flex-1">{label}</span>
      {badge && (
        <span
          className={`rounded-full px-[6px] py-[1px] font-mono text-[10.5px] font-medium leading-[1.3] ${badgeClass}`}
        >
          {badge}
        </span>
      )}
    </>
  );

  const baseClass = `flex cursor-pointer items-center gap-[11px] rounded-[6px] px-[10px] py-2 text-[13px] no-underline ${
    active ? 'bg-surface-cream-strong font-semibold text-ink' : 'text-body'
  }`;

  if (href) {
    return (
      <Link to={href} className={baseClass}>
        {inner}
      </Link>
    );
  }

  return (
    <span role="button" tabIndex={0} className={baseClass}>
      {inner}
    </span>
  );
}

function RailHelp() {
  return (
    <div className="mt-2 flex flex-col gap-1 border-t border-hairline px-[10px] py-2 text-[11px] text-muted">
      <div className="flex items-center justify-between gap-[6px]">
        <span>Command</span>
        <Kbd>⌘K</Kbd>
      </div>
      <div className="flex items-center justify-between gap-[6px]">
        <span>Navigate</span>
        <span className="flex gap-1">
          <Kbd>J</Kbd>
          <Kbd>K</Kbd>
        </span>
      </div>
    </div>
  );
}

export function Rail({
  items,
  bottomItems,
}: {
  items: RailItemDef[];
  bottomItems?: RailItemDef[];
}) {
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  return (
    <aside className="flex w-56 flex-shrink-0 flex-col gap-[2px] border-r border-hairline bg-surface-soft px-3 pb-[14px] pt-[18px]">
      {items.map((item) => (
        <RailItem
          key={item.key}
          icon={item.icon}
          label={item.label}
          badge={item.badge}
          badgeTone={item.badgeTone}
          href={item.href}
          active={item.href ? pathname === item.href : item.active}
        />
      ))}
      <div className="flex-1" />
      {bottomItems?.map((item) => (
        <RailItem
          key={item.key}
          icon={item.icon}
          label={item.label}
          badge={item.badge}
          badgeTone={item.badgeTone}
          href={item.href}
          active={item.href ? pathname === item.href : item.active}
        />
      ))}
      <RailHelp />
    </aside>
  );
}
