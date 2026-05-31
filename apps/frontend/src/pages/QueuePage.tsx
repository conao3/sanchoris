import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Kbd, Pill } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Rail items ─────────────────────────────────────────────────────────────────

const queueRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'queue',
}));

// ─── Page-local primitives ───────────────────────────────────────────────────────

function Chip({ active, count, children }: { active?: boolean; count?: number; children: ReactNode }) {
  return (
    <span
      className={`inline-flex h-[28px] cursor-pointer items-center gap-[5px] rounded-[6px] border border-hairline px-[10px] py-[3px] text-[12px] font-medium ${
        active ? 'bg-surface-cream-strong text-ink' : 'bg-canvas text-body'
      }`}
    >
      {children}
      {count != null && (
        <span className={`text-[11px] ${active ? 'text-muted' : 'text-muted-soft'}`}>{count}</span>
      )}
    </span>
  );
}

type SourceType = 'linear' | 'github' | 'cron' | 'direct' | 'webhook';

const SOURCE_ICON_CLASS: Record<SourceType, string> = {
  linear: 'bg-[rgba(93,184,166,0.2)] text-[#2c6e62]',
  github: 'bg-surface-dark text-on-dark',
  cron: 'bg-[rgba(232,165,90,0.25)] text-[#7a4d10]',
  direct: 'bg-[rgba(204,120,92,0.18)] text-primary-active',
  webhook: 'bg-surface-card text-ink',
};

const SOURCE_ICON_LABEL: Record<SourceType, string> = {
  linear: 'L',
  github: 'G',
  cron: 'C',
  direct: 'D',
  webhook: 'W',
};

function SourceBadge({ type, label }: { type: SourceType; label: string }) {
  return (
    <span className="inline-flex items-center gap-[6px] text-[12px] text-body">
      <span
        className={`inline-flex h-[18px] w-[18px] items-center justify-center rounded-[4px] font-mono text-[10px] font-bold ${SOURCE_ICON_CLASS[type]}`}
      >
        {SOURCE_ICON_LABEL[type]}
      </span>
      {label}
    </span>
  );
}

type Priority = 'P0' | 'P1' | 'P2';

const PRI_CLASS: Record<Priority, string> = {
  P0: 'text-error',
  P1: 'text-warning',
  P2: 'text-muted',
};

function PriCell({ pri }: { pri: Priority }) {
  return (
    <span className={`font-mono text-[12px] font-semibold ${PRI_CLASS[pri]}`}>{pri}</span>
  );
}

function DragHandle() {
  return <span className="font-mono text-[12px] text-muted-soft">☰</span>;
}

function WorkflowChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-[22px] items-center rounded-[6px] border border-hairline bg-canvas px-[8px] py-[3px] text-[11px] text-body">
      {children}
    </span>
  );
}

// ─── Table data ─────────────────────────────────────────────────────────────────

type QueueRow = {
  qid: string;
  title: ReactNode;
  source: { type: SourceType; label: string };
  pri: Priority;
  age: string;
  workflow: string;
  qpos: string;
};

const QUEUE_ROWS_LINEAR: QueueRow[] = [
  {
    qid: 'CON-1247',
    title: 'Add OAuth login for vendor portal',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P0',
    age: '12 m',
    workflow: 'delivery/default',
    qpos: '#1',
  },
  {
    qid: 'CON-1248',
    title: 'Per-tenant rate limits on /api/checkout',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P1',
    age: '38 m',
    workflow: 'delivery/default',
    qpos: '#2',
  },
  {
    qid: 'CON-1249',
    title: 'Webhook signature mismatch on /api/stripe',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P0',
    age: '52 m',
    workflow: 'hotfix',
    qpos: '#3',
  },
  {
    qid: 'CON-1250',
    title: 'Bulk CSV import (parent of 3 dupes — ENG-1182, ENG-1184)',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P1',
    age: '2 h',
    workflow: 'delivery/default',
    qpos: '#4',
  },
  {
    qid: 'CON-1251',
    title: 'Localise checkout error toasts for ja-JP',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P2',
    age: '3 h',
    workflow: 'delivery/default',
    qpos: '#7',
  },
  {
    qid: 'CON-1252',
    title: 'Update Storybook 8 deprecation list',
    source: { type: 'linear', label: 'Linear' },
    pri: 'P2',
    age: '5 h',
    workflow: 'maint',
    qpos: '#9',
  },
];

const QUEUE_ROWS_GITHUB: QueueRow[] = [
  {
    qid: 'gh-#481',
    title: (
      <>
        CI failure: Safari e2e —{' '}
        <span className="font-mono text-muted">auth.spec.ts</span>
      </>
    ),
    source: { type: 'github', label: 'GitHub' },
    pri: 'P1',
    age: '25 m',
    workflow: 'incident',
    qpos: '#5',
  },
  {
    qid: 'gh-issue-803',
    title: 'Bug: refresh-iframe cookie dropped on Safari ITP',
    source: { type: 'github', label: 'GitHub' },
    pri: 'P1',
    age: '1 h',
    workflow: 'delivery/default',
    qpos: '#6',
  },
  {
    qid: 'gh-issue-805',
    title: 'Docs: clarify auth scope changes',
    source: { type: 'github', label: 'GitHub' },
    pri: 'P2',
    age: '4 h',
    workflow: 'docs',
    qpos: '#10',
  },
  {
    qid: 'gh-dep',
    title: (
      <>
        Dependabot: <span className="font-mono">undici</span> 6.18 → 6.19 (CVE)
      </>
    ),
    source: { type: 'github', label: 'GitHub' },
    pri: 'P1',
    age: '6 h',
    workflow: 'security',
    qpos: '#8',
  },
];

const QUEUE_ROWS_CRON: QueueRow[] = [
  {
    qid: 'cron-drift',
    title: 'Nightly schema-drift sweep across acme-platform',
    source: { type: 'cron', label: 'cron' },
    pri: 'P2',
    age: '1 h',
    workflow: 'maint',
    qpos: '#11',
  },
  {
    qid: 'cron-audit',
    title: 'Weekly dep audit + license scan',
    source: { type: 'cron', label: 'cron' },
    pri: 'P2',
    age: '9 h',
    workflow: 'security',
    qpos: '#13',
  },
];

const QUEUE_ROWS_DIRECT: QueueRow[] = [
  {
    qid: 'dir-yk-04',
    title: 'Investigate elevated 502 from EU edge (manually filed)',
    source: { type: 'direct', label: 'direct' },
    pri: 'P0',
    age: '14 m',
    workflow: 'incident',
    qpos: '#12',
  },
  {
    qid: 'wh-pd-271',
    title: (
      <>
        PagerDuty: <span className="font-mono">webhook.lag.p95 &gt; 30s</span> for stripe-bridge
      </>
    ),
    source: { type: 'webhook', label: 'webhook' },
    pri: 'P1',
    age: '22 m',
    workflow: 'incident',
    qpos: '#14',
  },
];

// ─── Table components ────────────────────────────────────────────────────────────

function QueueTableRow({ row }: { row: QueueRow }) {
  return (
    <tr className="border-b border-hairline-soft">
      <td className="px-[16px] py-[13px] align-middle" style={{ width: '36px' }}>
        <DragHandle />
      </td>
      <td className="py-[13px] pr-[16px] align-middle">
        <span className="mr-[8px] font-mono text-[11.5px] text-muted">{row.qid}</span>
        <span className="text-[13px] text-ink">{row.title}</span>
      </td>
      <td className="py-[13px] pr-[16px] align-middle" style={{ width: '120px' }}>
        <SourceBadge type={row.source.type} label={row.source.label} />
      </td>
      <td className="py-[13px] pr-[16px] align-middle" style={{ width: '60px' }}>
        <PriCell pri={row.pri} />
      </td>
      <td
        className="py-[13px] pr-[16px] align-middle font-mono text-[12px] text-muted"
        style={{ width: '80px' }}
      >
        {row.age}
      </td>
      <td className="py-[13px] pr-[16px] align-middle" style={{ width: '160px' }}>
        <WorkflowChip>{row.workflow}</WorkflowChip>
      </td>
      <td
        className="py-[13px] align-middle font-mono text-[12px] font-semibold text-muted"
        style={{ width: '80px' }}
      >
        {row.qpos}
      </td>
    </tr>
  );
}

function RowGroupHeader({ children }: { children: ReactNode }) {
  return (
    <tr>
      <td
        colSpan={7}
        className="border-b border-t border-hairline bg-surface-soft px-[16px] py-[8px] text-[11px] font-semibold uppercase tracking-[1.4px] text-muted"
      >
        {children}
      </td>
    </tr>
  );
}

// ─── Page ────────────────────────────────────────────────────────────────────────

export function QueuePage() {
  return (
    <AppShell rail={<Rail items={queueRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', 'Queue']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Queue{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="default">14 incoming</Pill>
                  <Pill kind="running">3 picking up</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Tasks awaiting workflow assignment · ranked by score = priority × staleness ·{' '}
                <strong className="font-semibold text-body-strong">workflow delivery/default v0.3.2</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Bulk assign</Btn>
              <Btn variant="primary">+ New task</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex flex-col gap-4 overflow-hidden">
            <div className="flex flex-wrap items-center gap-[6px]">
              <Chip active count={14}>All sources</Chip>
              <Chip count={6}>Linear</Chip>
              <Chip count={4}>GitHub</Chip>
              <Chip count={2}>cron</Chip>
              <Chip count={1}>direct</Chip>
              <Chip count={1}>webhook</Chip>
              <span className="ml-2 inline-block h-[18px] w-px bg-hairline" aria-hidden="true" />
              <Chip>Priority ≥ P2</Chip>
              <Chip>Age ≤ 24h</Chip>
              <span className="ml-auto flex items-center gap-[5px] text-[12px] text-muted">
                Drag rows to reorder ·{' '}
                <Kbd>J</Kbd>
                <Kbd>K</Kbd>
              </span>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              <div className="overflow-hidden rounded-[10px] border border-hairline">
                <table className="w-full border-collapse text-left">
                  <thead>
                    <tr>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft px-[16px] py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '36px' }}
                      />
                      <th className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
                        Task
                      </th>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '120px' }}
                      >
                        Source
                      </th>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '60px' }}
                      >
                        Pri
                      </th>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '80px' }}
                      >
                        Age
                      </th>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '160px' }}
                      >
                        Workflow
                      </th>
                      <th
                        className="border-b border-t border-hairline bg-surface-soft py-[10px] pr-[16px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={{ width: '80px' }}
                      >
                        Queue #
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    <RowGroupHeader>Linear · team CON</RowGroupHeader>
                    {QUEUE_ROWS_LINEAR.map((row) => (
                      <QueueTableRow key={row.qid} row={row} />
                    ))}
                    <RowGroupHeader>GitHub · events</RowGroupHeader>
                    {QUEUE_ROWS_GITHUB.map((row) => (
                      <QueueTableRow key={row.qid} row={row} />
                    ))}
                    <RowGroupHeader>cron · scheduled</RowGroupHeader>
                    {QUEUE_ROWS_CRON.map((row) => (
                      <QueueTableRow key={row.qid} row={row} />
                    ))}
                    <RowGroupHeader>direct · webhook</RowGroupHeader>
                    {QUEUE_ROWS_DIRECT.map((row) => (
                      <QueueTableRow key={row.qid} row={row} />
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
