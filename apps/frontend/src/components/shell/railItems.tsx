import type { RailItemDef } from './Rail';

// ─── Rail icons ─────────────────────────────────────────────────────────────────
// Shared sidebar nav definitions for every workspace screen (Inbox, PRs, Workers, …).
// The icon set and ordering mirror the design source of truth
// (idea/projects/design/sanchoris-01/Sanchoris App Overview.html, topbar/rail chrome).

function SpaceIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="1" y="1" width="5" height="5" rx="1" />
      <rect x="8" y="1" width="5" height="5" rx="1" />
      <rect x="1" y="8" width="5" height="5" rx="1" />
      <rect x="8" y="8" width="5" height="5" rx="1" />
    </svg>
  );
}

function ProjectsIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M1 3.5C1 2.7 1.7 2 2.5 2H5l1.5 1.5h5C12.3 3.5 13 4.2 13 5v6c0 0.8-0.7 1.5-1.5 1.5h-9C1.7 12.5 1 11.8 1 11V3.5z" />
    </svg>
  );
}

function QueueIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
      <path d="M2 4h10M2 7h10M2 10h10" />
    </svg>
  );
}

function PRsIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="3.5" cy="3" r="1.5" />
      <circle cx="3.5" cy="11" r="1.5" />
      <circle cx="10.5" cy="11" r="1.5" />
      <path d="M3.5 4.5v5M10.5 9.5V5.5C10.5 4.4 9.6 3.5 8.5 3.5H6.5" />
      <path d="M7.5 1.7L6 3.5L7.5 5.3" />
    </svg>
  );
}

function WorkflowsIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="3" cy="3.5" r="1.8" />
      <circle cx="11" cy="3.5" r="1.8" />
      <circle cx="7" cy="10.5" r="1.8" />
      <path d="M3.5 5l3 4M10.5 5l-3 4" />
    </svg>
  );
}

function WorkersIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <rect x="2" y="3" width="10" height="6" rx="1" />
      <path d="M4 11h6M5 9v2M9 9v2" />
    </svg>
  );
}

function MemoryIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <path d="M2 2h7l3 3v7H2V2zM9 2v3h3M4.5 7.5h5M4.5 9.5h5" />
    </svg>
  );
}

function ChannelsIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="7" cy="7" r="5.5" />
      <path d="M1.5 7h11M7 1.5c2 1.8 2 9.2 0 11M7 1.5c-2 1.8-2 9.2 0 11" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 14 14" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="1.5">
      <circle cx="7" cy="7" r="2" />
      <path d="M7 1v2M7 11v2M1 7h2M11 7h2M2.7 2.7L4.1 4.1M9.9 9.9l1.4 1.4M2.7 11.3l1.4-1.4M9.9 4.1l1.4-1.4" />
    </svg>
  );
}

// ─── Rail item definitions ──────────────────────────────────────────────────────
// `active` is intentionally omitted here; each screen derives its own active item
// (e.g. `WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'prs' }))`).

export const WORKSPACE_ITEMS: RailItemDef[] = [
  { key: 'space', icon: <SpaceIcon />, label: 'Space', href: '/space' },
  { key: 'projects', icon: <ProjectsIcon />, label: 'Projects', badge: '7', href: '/projects/acme-platform' },
  { key: 'queue', icon: <QueueIcon />, label: 'Queue', badge: '28', badgeTone: 'coral', href: '/queue' },
  { key: 'prs', icon: <PRsIcon />, label: 'PRs', badge: '12', href: '/pull-requests' },
  { key: 'workflows', icon: <WorkflowsIcon />, label: 'Workflows', href: '/workflows' },
  { key: 'workers', icon: <WorkersIcon />, label: 'Workers', href: '/workers' },
  { key: 'memory', icon: <MemoryIcon />, label: 'Memory', href: '/memory' },
  { key: 'channels', icon: <ChannelsIcon />, label: 'Channels', href: '/channels' },
];

export const ACCOUNT_ITEMS: RailItemDef[] = [
  { key: 'settings', icon: <SettingsIcon />, label: 'Settings' },
];
