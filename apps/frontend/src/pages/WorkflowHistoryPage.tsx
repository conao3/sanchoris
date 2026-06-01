import { useState } from 'react';
import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Page-local primitives ─────────────────────────────────────────────────────

function Chip({
  active,
  style,
  children,
}: {
  active?: boolean;
  style?: React.CSSProperties;
  children: ReactNode;
}) {
  return (
    <span
      style={style}
      className={`inline-flex h-7 cursor-pointer items-center gap-[6px] rounded-[6px] border border-hairline px-[10px] py-[5px] text-[12px] font-[inherit] ${
        active
          ? 'border-hairline bg-surface-cream-strong font-medium text-ink'
          : 'bg-canvas text-body'
      }`}
    >
      {children}
    </span>
  );
}

function ChipCount({ children }: { children: ReactNode }) {
  return <span className="text-[11px] text-muted-soft">{children}</span>;
}

// ─── Bar chart ─────────────────────────────────────────────────────────────────

type BarState = 'default' | 'fail' | 'warn' | 'curr';

type BarDef = { height: number; state: BarState };

const BAR_DATA: BarDef[] = [
  { height: 30, state: 'default' },
  { height: 42, state: 'default' },
  { height: 38, state: 'default' },
  { height: 36, state: 'default' },
  { height: 14, state: 'fail' },
  { height: 40, state: 'default' },
  { height: 45, state: 'default' },
  { height: 38, state: 'default' },
  { height: 65, state: 'warn' },
  { height: 42, state: 'default' },
  { height: 36, state: 'default' },
  { height: 48, state: 'default' },
  { height: 40, state: 'default' },
  { height: 38, state: 'default' },
  { height: 50, state: 'default' },
  { height: 42, state: 'default' },
  { height: 36, state: 'default' },
  { height: 58, state: 'warn' },
  { height: 38, state: 'default' },
  { height: 42, state: 'default' },
  { height: 46, state: 'default' },
  { height: 40, state: 'default' },
  { height: 38, state: 'default' },
  { height: 18, state: 'fail' },
  { height: 42, state: 'default' },
  { height: 48, state: 'default' },
  { height: 36, state: 'default' },
  { height: 42, state: 'default' },
  { height: 50, state: 'default' },
  { height: 56, state: 'curr' },
];

const barStateClass: Record<BarState, string> = {
  default: 'bg-surface-card',
  fail: 'bg-error',
  warn: 'bg-warning',
  curr: 'bg-primary',
};

function BarChart() {
  return (
    <div className="rounded-[10px] border border-hairline bg-surface-soft px-[18px] pb-[18px] pt-[14px] mb-4">
      <div className="mb-[10px] flex items-center justify-between">
        <span className="text-[12px] font-semibold uppercase tracking-[1.2px] text-muted">
          Duration by run · 30 runs
        </span>
        <span className="text-[11px] text-muted-soft">
          scale 0–9 min · red = failed, amber = retried, coral = current
        </span>
      </div>
      <div className="flex h-[110px] items-end gap-1">
        {BAR_DATA.map((bar, i) => (
          <div
            key={i}
            className={`relative min-w-[8px] flex-1 rounded-t-[2px] ${barStateClass[bar.state]}`}
            style={{ height: `${bar.height}%` }}
          />
        ))}
      </div>
      <div className="flex justify-between pt-[6px] font-mono text-[10.5px] text-muted-soft">
        <span>R-9114</span>
        <span>R-9121</span>
        <span>R-9128</span>
        <span>R-9135</span>
        <span>R-9143 (now)</span>
      </div>
    </div>
  );
}

// ─── Step grid ─────────────────────────────────────────────────────────────────

type StepState = 'ok' | 'fail' | 'warn' | 'skip' | 'now' | 'default';

const stepStateClass: Record<StepState, string> = {
  default: 'bg-surface-card',
  ok: 'bg-success',
  fail: 'bg-error',
  warn: 'bg-warning',
  skip: 'bg-hairline',
  now: 'border-[1.5px] border-accent-amber bg-accent-amber opacity-60',
};

function StepGrid({ steps }: { steps: StepState[] }) {
  return (
    <div className="inline-flex gap-[3px]">
      {steps.map((st, i) => (
        <span
          key={i}
          className={`h-[18px] w-[14px] rounded-[2px] ${stepStateClass[st]}`}
        />
      ))}
    </div>
  );
}

// ─── Run table data ────────────────────────────────────────────────────────────

type RunRow = {
  run: string;
  task: string;
  started: string;
  steps: StepState[];
  worker: string;
  duration: string;
  status: { kind: 'running' | 'success' | 'warn' | 'failed' | 'gate'; label: string };
  expandable?: boolean;
};

const RUN_ROWS: RunRow[] = [
  {
    run: 'R-9143',
    task: 'CON-1247',
    started: '13:38 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'now', 'default', 'default'],
    worker: 'codex',
    duration: '4m 18s',
    status: { kind: 'running', label: 'running' },
  },
  {
    run: 'R-9142',
    task: 'CON-1248',
    started: '13:24 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok'],
    worker: 'claude-code',
    duration: '3m 51s',
    status: { kind: 'success', label: 'pass' },
  },
  {
    run: 'R-9141',
    task: 'CON-1243',
    started: '12:48 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'warn', 'ok', 'ok', 'ok', 'ok', 'ok'],
    worker: 'codex',
    duration: '5m 12s',
    status: { kind: 'warn', label: 'retried' },
  },
  {
    run: 'R-9140',
    task: 'CON-1242',
    started: '12:11 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'fail', 'skip', 'skip', 'skip'],
    worker: 'claude-code',
    duration: '2m 04s',
    status: { kind: 'failed', label: 'failed' },
    expandable: true,
  },
  {
    run: 'R-9139',
    task: 'CON-1241',
    started: '11:55 JST',
    steps: ['ok', 'fail', 'skip', 'skip', 'skip', 'skip', 'skip', 'skip', 'skip', 'skip'],
    worker: 'claude-code',
    duration: '52s',
    status: { kind: 'failed', label: 'paused' },
  },
  {
    run: 'R-9138',
    task: 'CON-1245',
    started: '11:32 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'now', 'default'],
    worker: 'openhands',
    duration: '2h 11m',
    status: { kind: 'gate', label: 'at gate' },
  },
  {
    run: 'R-9137',
    task: 'CON-1240',
    started: '10:18 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok'],
    worker: 'codex',
    duration: '3m 22s',
    status: { kind: 'success', label: 'pass' },
  },
  {
    run: 'R-9136',
    task: 'CON-1255',
    started: '09:48 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'warn', 'warn', 'skip', 'skip', 'skip'],
    worker: 'codex',
    duration: '1h 04m',
    status: { kind: 'warn', label: 'blocked' },
  },
  {
    run: 'R-9135',
    task: 'CON-1238',
    started: '09:11 JST',
    steps: ['ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok', 'ok'],
    worker: 'claude-code',
    duration: '3m 41s',
    status: { kind: 'success', label: 'pass' },
  },
];

// ─── Expandable step detail ────────────────────────────────────────────────────

type ExpStep = {
  num: string;
  name: string;
  sub: string;
  dur: string;
  failed?: boolean;
};

const EXP_STEPS: ExpStep[] = [
  { num: '01', name: 'Source', sub: 'Linear · CON-1242', dur: '0.4s' },
  { num: '02', name: 'Normalize', sub: 'title, repo, branch', dur: '0.9s' },
  { num: '03', name: 'Prioritize', sub: 'score 0.72', dur: '0.2s' },
  { num: '04', name: 'Select WF', sub: 'delivery/default', dur: '0.1s' },
  { num: '05', name: 'Select worker', sub: 'claude-code w-cc-1', dur: '0.1s' },
  { num: '06', name: 'Workspace', sub: 'ws-acme-plt-2e02', dur: '3.1s' },
  {
    num: '07',
    name: 'Execute · failed',
    sub: 'vitest exit 1 · 3 fail / 0 pass',
    dur: '1m 52s',
    failed: true,
  },
  { num: '08', name: 'Validate', sub: 'skipped', dur: '—' },
  { num: '09', name: 'Gate', sub: 'skipped', dur: '—' },
  { num: '10', name: 'Update', sub: 'skipped', dur: '—' },
];

function ExpandedDetail() {
  return (
    <tr>
      <td colSpan={7} className="border-b border-hairline bg-surface-soft p-0">
        <div className="grid grid-cols-5 gap-[10px] p-[14px_16px]">
          {EXP_STEPS.map((step) => (
            <div
              key={step.num}
              className="rounded-[6px] border border-hairline-soft bg-canvas p-[8px_10px] text-[11.5px] leading-[1.4]"
            >
              <span
                className={`mb-[2px] block text-[10px] font-semibold uppercase tracking-[1px] ${
                  step.failed ? 'text-error' : 'text-muted'
                }`}
              >
                {step.num} {step.name}
              </span>
              <span className="text-muted-soft">{step.sub}</span>
              <div className="font-mono text-[10.5px] text-muted-soft">
                {step.failed ? (
                  <>
                    {step.dur.split(' · ')[0]} ·{' '}
                    <span className="text-error">retry budget 0/2</span>
                  </>
                ) : (
                  step.dur
                )}
              </div>
            </div>
          ))}
        </div>
      </td>
    </tr>
  );
}

// ─── Status pill dot ──────────────────────────────────────────────────────────

function PillWithDot({
  kind,
  children,
}: {
  kind: 'running' | 'success' | 'warn' | 'failed' | 'gate';
  children: ReactNode;
}) {
  const pillKindMap = {
    running: 'running' as const,
    success: 'success' as const,
    warn: 'warn' as const,
    failed: 'failed' as const,
    gate: 'gate' as const,
  };
  return (
    <Pill kind={pillKindMap[kind]}>
      <span className="inline-block h-[6px] w-[6px] rounded-full bg-[rgba(255,255,255,0.85)]" />
      {children}
    </Pill>
  );
}

// ─── Run table ─────────────────────────────────────────────────────────────────

function RunTable() {
  const [expanded, setExpanded] = useState(true);

  return (
    <div className="flex-1 overflow-hidden rounded-[10px] border border-hairline bg-canvas">
      <table className="w-full border-separate border-spacing-0 text-[13px]">
        <thead>
          <tr>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 90 }}
            >
              Run
            </th>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 120 }}
            >
              Task
            </th>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 90 }}
            >
              Started
            </th>
            <th className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
              Steps · 10
            </th>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 80 }}
            >
              Worker
            </th>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 80 }}
            >
              Duration
            </th>
            <th
              className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
              style={{ width: 100 }}
            >
              Status
            </th>
          </tr>
        </thead>
        <tbody>
          {RUN_ROWS.map((row) => (
            <>
              <tr
                key={row.run}
                className="cursor-pointer"
                onClick={row.expandable ? () => setExpanded((v) => !v) : () => navigate('/runs/' + row.run.toLowerCase())}
              >
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle font-mono text-[13px]">
                  {row.run}
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle font-mono text-[13px]">
                  {row.task}
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle font-mono text-[13px] text-muted">
                  {row.started}
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle">
                  <StepGrid steps={row.steps} />
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle font-mono text-[13px]">
                  {row.worker}
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle font-mono text-[13px] text-muted">
                  {row.duration}
                </td>
                <td className="border-b border-hairline-soft px-4 py-[13px] align-middle">
                  <PillWithDot kind={row.status.kind}>{row.status.label}</PillWithDot>
                </td>
              </tr>
              {row.expandable && expanded && <ExpandedDetail key={`${row.run}-exp`} />}
            </>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

export function WorkflowHistoryPage() {
  return (
    <AppShell
      rail={
        <Rail
          items={WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'workflows' }))}
          bottomItems={ACCOUNT_ITEMS}
        />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Workflows', { label: 'delivery/default', href: '/workflows/delivery-default' }, 'History']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                delivery/default · run history{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="default">
                    <span className="font-mono">last 30 runs</span>
                  </Pill>
                  <Pill kind="success">27 pass</Pill>
                  <Pill kind="failed">2 fail</Pill>
                  <Pill kind="warn">1 retry</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Aggregated across all projects · median duration{' '}
                <strong className="font-semibold text-body-strong">3m 42s</strong> · updated 13:42
                JST today
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Export CSV</Btn>
              <Btn variant="primary">Re-run failures</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          {/* Filter row */}
          <div className="mb-[14px] flex items-center gap-2">
            <Chip active>
              v0.3.2 <ChipCount>30</ChipCount>
            </Chip>
            <Chip>
              v0.3.1 <ChipCount>142</ChipCount>
            </Chip>
            <Chip>
              v0.3.0 <ChipCount>218</ChipCount>
            </Chip>
            <Chip style={{ marginLeft: 8 }}>All projects</Chip>
            <Chip>All statuses</Chip>
            <span className="ml-auto font-sans text-[11px] text-muted-soft">
              Density:{' '}
              <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">
                comfortable
              </span>
            </span>
          </div>

          {/* Duration chart */}
          <BarChart />

          {/* Run history table */}
          <RunTable />
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
