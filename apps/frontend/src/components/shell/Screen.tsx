import { Link } from '@tanstack/react-router';
import type { ReactNode } from 'react';

export type CrumbItem = { label: string; href?: string };

export function Crumbs({ items }: { items: (string | CrumbItem)[] }) {
  const normalized: CrumbItem[] = items.map((item) =>
    typeof item === 'string' ? { label: item } : item,
  );
  return (
    <div className="mb-2 flex items-center gap-[7px] font-sans text-[12px] text-muted">
      {normalized.map((item, i) => (
        <span key={i} className="flex items-center gap-[7px]">
          {i > 0 && (
            <span className="text-[10px] text-muted-soft" aria-hidden="true">
              /
            </span>
          )}
          {item.href && i < normalized.length - 1 ? (
            <Link to={item.href} className="no-underline hover:underline">
              {item.label}
            </Link>
          ) : (
            <span className={i === normalized.length - 1 ? 'font-medium text-ink' : ''}>
              {item.label}
            </span>
          )}
        </span>
      ))}
    </div>
  );
}

export function Screen({ children }: { children: ReactNode }) {
  return (
    <main className="flex min-w-0 flex-1 flex-col overflow-hidden bg-canvas">
      {children}
    </main>
  );
}

export function ScreenHead({ children }: { children: ReactNode }) {
  return (
    <div className="flex-shrink-0 border-b border-hairline px-7 pb-[14px] pt-5">
      {children}
    </div>
  );
}

export function ScreenBody({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-7 pb-6 pt-[18px]">
      {children}
    </div>
  );
}
