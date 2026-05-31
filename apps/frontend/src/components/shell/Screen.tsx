import type { ReactNode } from 'react';

export function Crumbs({ items }: { items: string[] }) {
  return (
    <div className="mb-2 flex items-center gap-[7px] font-sans text-[12px] text-muted">
      {items.map((item, i) => (
        <span key={i} className="flex items-center gap-[7px]">
          {i > 0 && (
            <span className="text-[10px] text-muted-soft" aria-hidden="true">
              /
            </span>
          )}
          <span className={i === items.length - 1 ? 'font-medium text-ink' : ''}>{item}</span>
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
