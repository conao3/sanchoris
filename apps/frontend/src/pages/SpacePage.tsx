import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Kbd, Pill, type PillKind } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Window-tab dot color map ──────────────────────────────────────────────────

type WDot = 'coral' | 'teal' | 'navy' | 'amber';

const W_DOT_CLASS: Record<WDot, string> = {
  coral: 'bg-primary',
  teal: 'bg-accent-teal',
  navy: 'bg-surface-dark',
  amber: 'bg-accent-amber',
};

// ─── WindowTab ─────────────────────────────────────────────────────────────────

function WindowTab({
  label,
  dot,
  active,
  kbd,
  onClick,
}: {
  label: string;
  dot: WDot;
  active?: boolean;
  kbd: string;
  onClick?: () => void;
}) {
  return (
    <div
      onClick={onClick}
      className={`flex h-[36px] cursor-pointer select-none items-center gap-2 rounded-t-[7px] border border-b-0 px-[14px] pb-0 pt-[7px] text-[12.5px] -mb-px ${
        active
          ? 'border-hairline bg-canvas font-medium text-ink'
          : 'border-transparent text-muted'
      }`}
    >
      <span className={`h-[7px] w-[7px] flex-shrink-0 rounded-full ${W_DOT_CLASS[dot]}`} />
      {label}
      <span className="ml-1 font-mono text-[11px] text-muted">{kbd}</span>
    </div>
  );
}

// ─── MiniWin ───────────────────────────────────────────────────────────────────

function MiniWin({
  dot,
  title,
  subtitle,
  tall,
  children,
  onClick,
}: {
  dot: WDot;
  title: string;
  subtitle?: string;
  tall?: boolean;
  children: ReactNode;
  onClick?: () => void;
}) {
  return (
    <div
      onClick={onClick}
      className={`flex min-h-0 flex-col overflow-hidden rounded-[10px] border border-hairline bg-canvas shadow-[0_2px_8px_rgba(20,20,19,0.04)] ${
        tall ? 'row-span-2' : ''
      }${onClick ? ' cursor-pointer' : ''}`}
    >
      {/* mwh */}
      <div className="flex h-8 flex-shrink-0 items-center gap-2 border-b border-hairline-soft bg-surface-soft px-3 text-[12px] font-medium text-body-strong">
        <span className={`h-[7px] w-[7px] flex-shrink-0 rounded-full ${W_DOT_CLASS[dot]}`} />
        <span className="truncate">{title}</span>
        {subtitle && (
          <span className="ml-1 truncate font-mono text-[11px] font-normal text-muted-soft">
            {subtitle}
          </span>
        )}
        <span className="ml-auto flex flex-shrink-0 gap-1 font-mono text-[11px] text-muted-soft">
          <span>−</span>
          <span>□</span>
          <span>×</span>
        </span>
      </div>
      {/* mwb */}
      <div className="flex min-h-0 flex-1 flex-col overflow-auto">{children}</div>
    </div>
  );
}

// ─── StatusPill (with inner dot) ──────────────────────────────────────────────

function StatusPill({ kind, label }: { kind: PillKind; label: string }) {
  const dotClass =
    kind === 'queued' || kind === 'done' ? 'bg-muted-soft' : 'bg-white/[0.85]';
  return (
    <Pill kind={kind}>
      <span className={`h-[6px] w-[6px] flex-shrink-0 rounded-full ${dotClass}`} />
      {label}
    </Pill>
  );
}

// ─── Source badge ──────────────────────────────────────────────────────────────

type SrcKind = 'git' | 'lin' | 'cron' | 'dir' | 'web';

const SRC_META: Record<SrcKind, { bg: string; color: string; letter: string; label: string }> = {
  git: { bg: 'bg-surface-dark', color: 'text-on-dark', letter: 'G', label: 'GitHub' },
  lin: {
    bg: 'bg-[rgba(93,184,166,0.2)]',
    color: 'text-[#2c6e62]',
    letter: 'L',
    label: 'Linear',
  },
  cron: {
    bg: 'bg-[rgba(232,165,90,0.25)]',
    color: 'text-[#7a4d10]',
    letter: 'C',
    label: 'cron',
  },
  dir: {
    bg: 'bg-[rgba(204,120,92,0.18)]',
    color: 'text-primary-active',
    letter: 'D',
    label: 'direct',
  },
  web: { bg: 'bg-surface-card', color: 'text-ink', letter: 'W', label: 'webhook' },
};

function SrcBadge({ kind }: { kind: SrcKind }) {
  const m = SRC_META[kind];
  return (
    <span className="inline-flex items-center gap-[5px] text-[10px] text-body">
      <span
        className={`inline-flex h-4 w-4 items-center justify-center rounded-[4px] font-mono text-[9px] font-bold ${m.bg} ${m.color}`}
      >
        {m.letter}
      </span>
      {m.label}
    </span>
  );
}

// ─── Check dots (PR checks column) ────────────────────────────────────────────

type CheckDot = 'ok' | 'warn' | 'err' | 'pend';

const CHECK_COLOR: Record<CheckDot, string> = {
  ok: '#386b46',
  warn: '#d4a017',
  err: '#c64545',
  pend: '#8e8b82',
};

const CHECK_GLYPH: Record<CheckDot, string> = {
  ok: '●',
  warn: '●',
  err: '●',
  pend: '·',
};

function CheckDots({ dots }: { dots: CheckDot[] }) {
  return (
    <span className="font-mono text-[11px]">
      {dots.map((dot, i) => (
        // eslint-disable-next-line react/no-array-index-key
        <span key={i} style={{ color: CHECK_COLOR[dot] }}>
          {CHECK_GLYPH[dot]}
        </span>
      ))}
    </span>
  );
}

// ─── Shared mini-table th ─────────────────────────────────────────────────────

function MTh({ width, children }: { width?: string; children?: ReactNode }) {
  return (
    <th
      style={width ? { width } : undefined}
      className="border-y border-hairline bg-surface-soft px-3 py-[8px] text-left text-[10px] font-semibold uppercase tracking-[1.4px] text-muted"
    >
      {children}
    </th>
  );
}

// ─── Runs data ─────────────────────────────────────────────────────────────────

type RunRow = {
  id: string;
  ticket: string;
  title: string;
  worker: string;
  status: PillKind;
  statusLabel: string;
  age: string;
};

const RUNS: RunRow[] = [
  {
    id: 'R-9143',
    ticket: 'CON-1247',
    title: 'Add OAuth login for vendor portal',
    worker: 'codex',
    status: 'running',
    statusLabel: 'execute',
    age: '4m',
  },
  {
    id: 'R-9142',
    ticket: 'CON-1248',
    title: 'Per-tenant rate limits on /api/checkout',
    worker: 'cc',
    status: 'validating',
    statusLabel: 'validate',
    age: '14m',
  },
  {
    id: 'R-9141',
    ticket: 'CON-1249',
    title: 'Webhook signature mismatch /api/stripe',
    worker: 'codex',
    status: 'running',
    statusLabel: 'execute',
    age: '11m',
  },
  {
    id: 'R-9138',
    ticket: 'CON-1245',
    title: 'Retry/backoff on webhook fanout',
    worker: 'oh',
    status: 'gate',
    statusLabel: 'gate',
    age: '2h',
  },
  {
    id: 'R-9145',
    ticket: 'CON-1250',
    title: 'Bulk CSV import (rollup)',
    worker: 'oh',
    status: 'running',
    statusLabel: 'plan',
    age: '2m',
  },
  {
    id: 'R-9094',
    ticket: 'CON-1244',
    title: 'Stabilise Safari e2e flake',
    worker: 'codex',
    status: 'done',
    statusLabel: 'done',
    age: '1d',
  },
  {
    id: 'R-9091',
    ticket: 'CON-1243',
    title: 'Fix login redirect loop after SSO',
    worker: 'codex',
    status: 'done',
    statusLabel: 'done',
    age: '1d',
  },
  {
    id: 'R-9077',
    ticket: 'CON-1241',
    title: 'Migrate orders_v2 schema',
    worker: 'cc',
    status: 'failed',
    statusLabel: 'paused',
    age: '2d',
  },
  {
    id: 'R-9120',
    ticket: 'cron-audit',
    title: 'Weekly dep audit',
    worker: 'sym',
    status: 'queued',
    statusLabel: 'queued',
    age: '9h',
  },
];

// ─── Queue data ────────────────────────────────────────────────────────────────

type QueueRow = {
  src: SrcKind;
  id: string;
  title: string;
  priority: string;
  priorityColor: string;
  age: string;
};

const QUEUE: QueueRow[] = [
  {
    src: 'git',
    id: '#481',
    title: 'CI failure: Safari e2e auth.spec.ts',
    priority: 'P1',
    priorityColor: 'text-warning',
    age: '25m',
  },
  {
    src: 'lin',
    id: 'CON-1251',
    title: 'Localise checkout error toasts',
    priority: 'P2',
    priorityColor: 'text-muted',
    age: '3h',
  },
  {
    src: 'cron',
    id: 'drift',
    title: 'Nightly schema-drift sweep',
    priority: 'P2',
    priorityColor: 'text-muted',
    age: '1h',
  },
  {
    src: 'dir',
    id: 'dir-yk-04',
    title: 'Investigate elevated 502 EU edge',
    priority: 'P0',
    priorityColor: 'text-error',
    age: '14m',
  },
  {
    src: 'web',
    id: 'pd-271',
    title: 'webhook.lag.p95 > 30s stripe-bridge',
    priority: 'P1',
    priorityColor: 'text-warning',
    age: '22m',
  },
  {
    src: 'lin',
    id: 'CON-1252',
    title: 'Storybook 8 deprecation list',
    priority: 'P2',
    priorityColor: 'text-muted',
    age: '5h',
  },
];

// ─── PRs data ──────────────────────────────────────────────────────────────────

type PRRow = {
  pr: string;
  prefix: string;
  title: string;
  checks: CheckDot[];
  status: PillKind;
  statusLabel: string;
};

const PRS: PRRow[] = [
  {
    pr: 'acme-platform·#482',
    prefix: 'feat(auth):',
    title: 'OAuth login vendor portal',
    checks: ['ok', 'ok', 'ok', 'warn'],
    status: 'gate',
    statusLabel: 'gate',
  },
  {
    pr: 'acme-platform·#481',
    prefix: 'fix(auth):',
    title: '±90s JWT skew',
    checks: ['ok', 'ok', 'ok', 'ok'],
    status: 'success',
    statusLabel: 'ready',
  },
  {
    pr: 'stripe-bridge·#87',
    prefix: 'feat:',
    title: 'retry/backoff on webhook fanout',
    checks: ['ok', 'ok', 'ok', 'err'],
    status: 'failed',
    statusLabel: 'fail',
  },
  {
    pr: 'growth-api·#214',
    prefix: 'chore:',
    title: 'TS 5.5',
    checks: ['ok', 'ok', 'ok', 'ok'],
    status: 'success',
    statusLabel: 'ready',
  },
  {
    pr: 'acme-platform·#480',
    prefix: 'perf:',
    title: 'lazy-load checkout assets',
    checks: ['ok', 'ok', 'warn', 'warn'],
    status: 'default',
    statusLabel: 'review',
  },
  {
    pr: 'platform-docs·#38',
    prefix: 'docs:',
    title: 'clarify auth scope changes',
    checks: ['ok', 'ok', 'pend', 'pend'],
    status: 'draft',
    statusLabel: 'draft',
  },
];

// ─── Winbar ────────────────────────────────────────────────────────────────────

function Winbar() {
  return (
    <div className="flex h-[42px] flex-shrink-0 items-stretch border-b border-hairline bg-surface-soft px-4 pb-0 pt-[6px]">
      <WindowTab label="acme-platform · runs" dot="coral" active kbd="⌘1" onClick={() => navigate('/runs/r-9143')} />
      <WindowTab label="queue · global" dot="teal" kbd="⌘2" onClick={() => navigate('/queue')} />
      <WindowTab label="PRs · all repos" dot="navy" kbd="⌘3" onClick={() => navigate('/pull-requests')} />
      <WindowTab label="incidents · prod" dot="amber" kbd="⌘4" onClick={() => navigate('/gates')} />
      <button
        type="button"
        className="ml-[6px] flex h-[30px] cursor-pointer items-center gap-[6px] self-center rounded-[6px] border border-dashed border-hairline bg-canvas px-3 font-sans text-[12px] text-muted"
      >
        + New window
      </button>
      <span className="ml-auto flex items-center gap-2 self-center pr-1 font-mono text-[11px] text-muted-soft">
        layout <Kbd>quad-A</Kbd> · <Kbd>⌘⇧L</Kbd> switch
      </span>
    </div>
  );
}

// ─── Runs mini-window ─────────────────────────────────────────────────────────

function RunsMiniWin() {
  return (
    <MiniWin
      dot="coral"
      title="acme-platform · Runs"
      subtitle="github.com/acme/acme-platform · main · wf v0.3.2"
      tall
      onClick={() => navigate('/runs/r-9143')}
    >
      {/* mini-toolbar */}
      <div className="flex flex-shrink-0 items-center gap-[6px] border-b border-hairline-soft px-3 py-[7px]">
        {(
          [
            { label: 'All', count: '9', active: true },
            { label: 'Running', count: '4' },
            { label: 'Gates', count: '1' },
            { label: 'Failed', count: '1' },
          ] as { label: string; count: string; active?: boolean }[]
        ).map((chip) => (
          <span
            key={chip.label}
            className={`inline-flex h-[22px] cursor-pointer items-center gap-1 rounded-[6px] border border-hairline px-2 font-sans text-[11px] font-medium ${
              chip.active ? 'bg-surface-cream-strong text-ink' : 'bg-canvas text-body'
            }`}
          >
            {chip.label}{' '}
            <span className="font-mono text-[10px] text-muted-soft">{chip.count}</span>
          </span>
        ))}
        <span className="ml-auto font-mono text-[10px] text-muted-soft">updated 13:42 JST</span>
      </div>
      {/* table */}
      <table className="w-full table-fixed border-separate border-spacing-0">
        <thead>
          <tr>
            <MTh width="78px">Run</MTh>
            <MTh>Title</MTh>
            <MTh width="78px">Worker</MTh>
            <MTh width="100px">Status</MTh>
            <MTh width="62px">Age</MTh>
          </tr>
        </thead>
        <tbody>
          {RUNS.map((row, i) => {
            const border = i < RUNS.length - 1 ? 'border-b border-hairline-soft' : '';
            return (
              <tr key={row.id}>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="font-mono text-[11px] text-muted">{row.id}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="mr-1 font-mono text-[10px] text-muted">{row.ticket}</span>
                  <span className="text-[12px] text-body">{row.title}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="font-mono text-[11px] text-muted">{row.worker}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <StatusPill kind={row.status} label={row.statusLabel} />
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="font-mono text-[11px] text-muted">{row.age}</span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </MiniWin>
  );
}

// ─── Queue mini-window ────────────────────────────────────────────────────────

function QueueMiniWin() {
  return (
    <MiniWin dot="teal" title="queue · global" subtitle="14 incoming · ranked" onClick={() => navigate('/queue')}>
      <table className="w-full table-fixed border-separate border-spacing-0">
        <thead>
          <tr>
            <MTh width="68px">Source</MTh>
            <MTh>Task</MTh>
            <MTh width="34px">P</MTh>
            <MTh width="44px">Age</MTh>
          </tr>
        </thead>
        <tbody>
          {QUEUE.map((row, i) => {
            const border = i < QUEUE.length - 1 ? 'border-b border-hairline-soft' : '';
            return (
              <tr key={row.id}>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <SrcBadge kind={row.src} />
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="mr-1 font-mono text-[10px] text-muted">{row.id}</span>
                  <span className="text-[12px] text-body">{row.title}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className={`font-mono text-[11px] font-semibold ${row.priorityColor}`}>
                    {row.priority}
                  </span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="font-mono text-[11px] text-muted">{row.age}</span>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </MiniWin>
  );
}

// ─── PRs mini-window ──────────────────────────────────────────────────────────

function PRsMiniWin() {
  return (
    <MiniWin dot="amber" title="PRs · all repos" subtitle="11 open · 1 failing" onClick={() => navigate('/pull-requests')}>
      <table className="w-full table-fixed border-separate border-spacing-0">
        <thead>
          <tr>
            <MTh width="80px">PR</MTh>
            <MTh>Title</MTh>
            <MTh width="60px">Checks</MTh>
            <MTh width="78px">Status</MTh>
          </tr>
        </thead>
        <tbody>
          {PRS.map((row, i) => {
            const border = i < PRS.length - 1 ? 'border-b border-hairline-soft' : '';
            return (
              <tr key={row.pr}>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <span className="font-mono text-[10px] text-muted">{row.pr}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <strong className="font-medium text-ink">{row.prefix}</strong>{' '}
                  <span className="text-[12px] text-body">{row.title}</span>
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <CheckDots dots={row.checks} />
                </td>
                <td className={`px-3 py-[9px] align-middle ${border}`}>
                  <StatusPill kind={row.status} label={row.statusLabel} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </MiniWin>
  );
}

// ─── Canvas area ───────────────────────────────────────────────────────────────

function CanvasArea() {
  return (
    <div
      className="min-h-0 flex-1 p-5"
      style={{
        display: 'grid',
        gridTemplateColumns: '1.55fr 1fr',
        gridTemplateRows: '1fr 1fr',
        gap: '20px',
      }}
    >
      <RunsMiniWin />
      <QueueMiniWin />
      <PRsMiniWin />
    </div>
  );
}

// ─── SpacePage ─────────────────────────────────────────────────────────────────

export function SpacePage() {
  return (
    <AppShell
      rail={
        <Rail
          items={WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'space' }))}
          bottomItems={ACCOUNT_ITEMS}
        />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Spaces', 'platform-team']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Space · platform-team{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="default">
                    <span className="font-mono">4 windows tiled</span>
                  </Pill>
                  <Pill kind="running">5 active runs</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Multi-window canvas ·{' '}
                <strong className="font-semibold text-body-strong">kanagawa-ops</strong> · layout{' '}
                <strong className="font-semibold text-body-strong">quad-A</strong> · saved 13:31 JST
                today · <Kbd>⌘1</Kbd>…<Kbd>⌘4</Kbd> jump between windows
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Save layout</Btn>
              <Btn variant="primary">Share space</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          {/* Override ScreenBody padding to allow winbar + canvas to fill edge-to-edge */}
          <div className="-mx-7 -mb-6 -mt-[18px] flex min-h-0 flex-1 flex-col">
            <Winbar />
            <CanvasArea />
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
