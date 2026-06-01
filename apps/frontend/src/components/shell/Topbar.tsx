import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';
import { Kbd } from './primitives';
import { clearTokens, getLogoutUrl } from '../../lib/cognito';

export type AvatarVariant = 'default' | 'me' | 'sm' | 'xs' | 'coral' | 'teal' | 'navy' | 'amber';

const avatarVariantClass: Record<AvatarVariant, string> = {
  default: 'bg-surface-card text-ink',
  me: 'bg-surface-dark text-on-dark',
  sm: 'bg-surface-card text-ink w-[22px] h-[22px] text-[9.5px]',
  xs: 'bg-surface-card text-ink w-[18px] h-[18px] text-[9px]',
  coral: 'bg-primary text-white',
  teal: 'bg-accent-teal text-white',
  navy: 'bg-surface-dark text-on-dark',
  amber: 'bg-accent-amber text-[#4a3306]',
};

export function Avatar({
  variant = 'default',
  title,
  children,
}: {
  variant?: AvatarVariant;
  title?: string;
  children: ReactNode;
}) {
  const sizeClass = variant === 'sm' ? '' : variant === 'xs' ? '' : 'w-7 h-7 text-[11px]';
  return (
    <div
      title={title}
      className={`inline-flex flex-shrink-0 items-center justify-center rounded-full font-semibold ${sizeClass} ${avatarVariantClass[variant]}`}
    >
      {children}
    </div>
  );
}

function Wordmark() {
  return (
    <span className="flex items-center gap-[9px] font-serif text-[22px] font-medium leading-none tracking-[-0.4px] text-ink">
      <span className="relative inline-block h-4 w-4">
        <span
          className="absolute inset-0 rounded-[2px] border-[1.4px] border-ink"
          aria-hidden="true"
        />
        <span
          className="absolute left-0 top-0 h-[7px] w-[7px] bg-primary"
          aria-hidden="true"
        />
      </span>
      sanchoris<em className="not-italic text-primary">.</em>
    </span>
  );
}

function WorkspaceSwitcher({
  tag,
  tone,
  name,
  strong,
}: {
  tag: string;
  tone: 'org' | 'proj';
  name: string;
  strong?: boolean;
}) {
  const tagClass =
    tone === 'org' ? 'bg-surface-dark text-on-dark' : 'bg-primary text-white';
  return (
    <button
      type="button"
      className="inline-flex cursor-pointer items-center gap-[7px] rounded-[6px] border-none bg-transparent px-[9px] py-[6px] font-[inherit] text-[13px] text-body"
    >
      <span
        className={`inline-flex h-[18px] w-[18px] items-center justify-center rounded-[4px] text-[10px] font-bold ${tagClass}`}
      >
        {tag}
      </span>
      <span className={`text-ink ${strong ? 'font-semibold' : 'font-medium'}`}>{name}</span>
      <span className="text-[10px] text-muted-soft">▾</span>
    </button>
  );
}

function SearchBar() {
  return (
    <div className="flex h-[34px] w-[440px] max-w-full items-center gap-[10px] rounded-[7px] border border-hairline bg-surface-soft px-3 text-[13px] text-muted">
      <span className="text-[14px] text-muted">⌕</span>
      <span className="flex-1">Search tasks · runs · PRs · memory…</span>
      <Kbd>⌘K</Kbd>
    </div>
  );
}

function ModelBadge() {
  return (
    <span className="inline-flex h-7 items-center gap-[7px] rounded-[6px] border border-hairline bg-surface-soft px-[10px] py-[4px] font-mono text-[12px] text-body-strong">
      <span className="h-[6px] w-[6px] rounded-full bg-accent-teal" aria-hidden="true" />
      claude-sonnet-4-6
    </span>
  );
}

function TopbarIcon({
  title,
  badge,
  children,
}: {
  title: string;
  badge?: number;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      className="relative inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-[6px] border-none bg-transparent font-[inherit] text-[14px] text-muted"
    >
      {children}
      {badge != null && (
        <span className="absolute right-1 top-1 inline-flex min-w-[13px] items-center justify-center rounded-full bg-primary px-[3px] font-sans text-[9px] font-bold leading-none text-white">
          {badge}
        </span>
      )}
    </button>
  );
}

export function Topbar() {
  return (
    <header className="flex h-14 flex-shrink-0 items-center gap-4 border-b border-hairline bg-canvas px-5">
      <div className="flex items-center gap-3">
        <Link to="/space" className="no-underline">
          <Wordmark />
        </Link>
        <span className="h-[22px] w-px bg-hairline" aria-hidden="true" />
        <WorkspaceSwitcher tag="A" tone="org" name="acme-org" />
        <span className="font-light text-muted-soft">/</span>
        <WorkspaceSwitcher tag="P" tone="proj" name="acme-platform" strong />
      </div>
      <div className="flex flex-1 justify-center">
        <SearchBar />
      </div>
      <div className="flex items-center gap-3">
        <ModelBadge />
        <TopbarIcon title="Notifications" badge={3}>⌁</TopbarIcon>
        <TopbarIcon title="Help">?</TopbarIcon>
        <button
          type="button"
          title="Sign out"
          onClick={() => {
            clearTokens();
            window.location.href = getLogoutUrl();
          }}
          className="inline-flex cursor-pointer items-center border-none bg-transparent p-0"
        >
          <Avatar variant="me" title="Yuto Kanagawa (you)">YK</Avatar>
        </button>
      </div>
    </header>
  );
}
