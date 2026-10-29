import { Link } from '@tanstack/react-router';
import clsx from 'clsx';
import type { LucideIcon } from 'lucide-react';
import { CircleAlert, Inbox } from 'lucide-react';
import type { CSSProperties, ReactNode } from 'react';
import { t } from '../i18n';
import { Button } from './Button';

export type Tone = 'run' | 'ok' | 'warn' | 'err' | 'wait';

export const toneClass: Record<Tone, string> = {
  run: 'text-run',
  ok: 'text-ok',
  warn: 'text-warn',
  err: 'text-err',
  wait: 'text-muted',
};

export function Kbd({ children, tone = 'plain' }: { children: ReactNode; tone?: 'plain' | 'button' | 'inverse' }) {
  return (
    <span
      className={clsx(
        'inline-block rounded-kbd border px-[5px] font-mono text-xs leading-4',
        tone === 'plain' && 'border-line2 bg-bg text-muted',
        tone === 'button' && 'ml-0.5 border-line bg-bg text-muted',
        tone === 'inverse' && 'ml-0.5 border-[#52525b] bg-transparent text-line2',
      )}
    >
      {children}
    </span>
  );
}

export function Icon({ icon: IconComponent, className }: { icon: LucideIcon; className?: string }) {
  return <IconComponent className={clsx('inline-block size-[15px] shrink-0 align-[-3px]', className)} aria-hidden />;
}

export function State({ tone, icon, children, className, wrap }: { tone: Tone; icon: LucideIcon; children: ReactNode; className?: string; wrap?: boolean }) {
  return (
    <span className={clsx('inline-flex items-center gap-1.5 text-sm', wrap ? 'whitespace-normal' : 'whitespace-nowrap', toneClass[tone], className)}>
      <Icon icon={icon} />
      {children}
    </span>
  );
}

export function Title({ children }: { children: ReactNode }) {
  return <h1 className="m-0 mb-0.5 flex items-center gap-3 text-h1 font-semibold">{children}</h1>;
}

export function Sub({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={clsx('mb-[18px] text-muted', className)}>{children}</div>;
}

export function Section({
  children,
  count,
  action,
  first,
}: {
  children: ReactNode;
  count?: ReactNode;
  action?: ReactNode;
  first?: boolean;
}) {
  return (
    <h2 className={clsx('mb-1.5 flex items-center gap-2 text-sm font-semibold text-ink', first ? 'mt-0' : 'mt-[22px]')}>
      {children}
      {count !== undefined ? <span className="font-mono text-xs font-normal text-muted">{count}</span> : null}
      {action ? <span className="ml-auto">{action}</span> : null}
    </h2>
  );
}

function fixedColumns(cols: string): string {
  return cols.replace(/(^|\s)1fr/g, '$1minmax(160px,1fr)');
}

export function Th({ cols, children }: { cols: string; children: ReactNode }) {
  return (
    <div
      className="grid h-8 items-center gap-3.5 border-b border-line2 px-1 text-xs text-muted"
      style={{ gridTemplateColumns: fixedColumns(cols) }}
    >
      {children}
    </div>
  );
}

export function Row({
  cols,
  children,
  selected,
  tall,
  onClick,
  rowRef,
}: {
  cols: string;
  children: ReactNode;
  selected?: boolean;
  tall?: boolean;
  onClick?: () => void;
  rowRef?: (element: HTMLDivElement | null) => void;
}) {
  return (
    <div
      ref={rowRef}
      onClick={onClick}
      className={clsx(
        'grid items-center gap-3.5 border-b border-line px-1 last:border-b-0 hover:bg-sub',
        tall ? 'min-h-14 py-1.5' : 'min-h-10',
        selected && 'bg-sub shadow-[inset_2px_0_0_var(--color-ink)]',
        onClick && 'cursor-pointer',
      )}
      style={{ gridTemplateColumns: fixedColumns(cols) }}
    >
      {children}
    </div>
  );
}

export function Meter({ value, pace }: { value: number; pace?: number }) {
  return (
    <div className="relative h-1 rounded-sm bg-hover">
      <i className="absolute inset-y-0 left-0 rounded-sm bg-ink" style={{ width: `${Math.min(1, value) * 100}%` }} />
      {pace !== undefined ? (
        <u className="absolute -inset-y-[3px] w-[1.5px] bg-faint" style={{ left: `${pace * 100}%` }} />
      ) : null}
    </div>
  );
}

export function Segments({ filled, total }: { filled: number; total: number }) {
  return (
    <div className="flex gap-[3px]">
      {Array.from({ length: total }, (_, index) => (
        <i key={index} className={clsx('block h-2 w-[22px] rounded-sm', index < filled ? 'bg-ink' : 'bg-hover')} />
      ))}
    </div>
  );
}

export function Toggle({ on, onToggle, isDisabled }: { on: boolean; onToggle?: () => void; isDisabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={isDisabled}
      onClick={(event) => {
        event.stopPropagation();
        onToggle?.();
      }}
      className="inline-flex cursor-pointer items-center gap-[7px] text-sm text-muted disabled:cursor-not-allowed"
    >
      <i className={clsx('relative block h-3.5 w-[26px] rounded-[7px]', on ? 'bg-ink' : 'bg-line2')}>
        <span className={clsx('absolute top-0.5 size-2.5 rounded-full bg-white', on ? 'right-0.5' : 'left-0.5')} />
      </i>
      {on ? t('common.on') : t('common.off')}
    </button>
  );
}

export function Chip({ children }: { children: ReactNode }) {
  return (
    <span className="mr-1 inline-block rounded-kbd bg-hover px-1.5 font-mono text-xs text-body">{children}</span>
  );
}

const linkClass = 'underline decoration-line2 underline-offset-[3px] hover:decoration-ink';

export function Lk({ to, params, children, className }: { to: string; params?: Record<string, string>; children: ReactNode; className?: string }) {
  return (
    <Link to={to} params={params} className={clsx(linkClass, className)} onClick={(event) => event.stopPropagation()}>
      {children}
    </Link>
  );
}

export function ExternalLk({ href, children, className }: { href: string; children: ReactNode; className?: string }) {
  return (
    <a href={href} target="_blank" rel="noreferrer" className={clsx(linkClass, className)} onClick={(event) => event.stopPropagation()}>
      {children}
    </a>
  );
}

export function Id({ children }: { children: ReactNode }) {
  return <span className="font-mono text-xs text-muted">{children}</span>;
}

export function Dim({ children, className, style }: { children: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={clsx('text-muted', className)} style={style}>
      {children}
    </div>
  );
}

export function Kv({ items }: { items: [ReactNode, ReactNode][] }) {
  return (
    <div className="grid grid-cols-[112px_1fr] gap-x-2.5 gap-y-[7px]">
      {items.map(([key, value], index) => (
        <div key={index} className="contents">
          <span className="text-muted">{key}</span>
          <span>{value}</span>
        </div>
      ))}
    </div>
  );
}

export function Empty({ children, icon = Inbox }: { children: ReactNode; icon?: LucideIcon }) {
  return (
    <div className="mt-1.5 rounded-ui border border-dashed border-line2 px-[22px] py-7">
      <Icon icon={icon} className="size-5 text-muted" />
      <div className="mt-2 text-muted">{children}</div>
    </div>
  );
}

export function Skeleton({ rows = 4 }: { rows?: number }) {
  return (
    <div aria-busy="true" aria-label={t('common.loading')}>
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="grid h-10 grid-cols-[100px_1fr_70px] items-center gap-3.5 border-b border-line">
          <i className="block h-2.5 rounded-[3px] bg-hover" />
          <i className="block h-2.5 rounded-[3px] bg-hover" />
          <i className="block h-2.5 rounded-[3px] bg-hover" />
        </div>
      ))}
    </div>
  );
}

export function ErrorNote({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div className="my-1.5 mb-3.5 flex items-start gap-2.5 border-y border-line py-2.5">
      <Icon icon={CircleAlert} className="mt-0.5 text-err" />
      <div className="flex-1">
        <div className="font-medium">{t('common.loadFailed')}</div>
        <Dim>{message}</Dim>
      </div>
      {onRetry ? (
        <Button small onPress={onRetry}>
          {t('common.retry')}
        </Button>
      ) : null}
    </div>
  );
}

export function Note({ icon, children }: { icon: LucideIcon; children: ReactNode }) {
  return (
    <div className="my-1.5 mb-3.5 flex items-start gap-2.5 border-y border-line py-2.5">
      <Icon icon={icon} className="mt-0.5" />
      <div>{children}</div>
    </div>
  );
}

export function Code({ children }: { children: ReactNode }) {
  return (
    <div className="whitespace-pre rounded-ui border border-line bg-sub px-3.5 py-2.5 font-mono text-sm leading-[1.75] text-body">
      {children}
    </div>
  );
}
