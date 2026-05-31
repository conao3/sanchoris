import type { ReactNode } from 'react';
import { Topbar } from './Topbar';

export function AppShell({
  rail,
  children,
}: {
  rail: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-canvas font-sans text-ink">
      <Topbar />
      <div className="flex min-h-0 flex-1">
        {rail}
        {children}
      </div>
    </div>
  );
}
