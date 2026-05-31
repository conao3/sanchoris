import { useState } from 'react';
import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, type PillKind } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Bar chart ────────────────────────────────────────────────────────────────

const BAR_DATA: Array<{ h: number; kind: 'normal' | 'fail' | 'curr' }> = [
  { h: 30, kind: 'normal' }, { h: 42, kind: 'normal' }, { h: 14, kind: 'fail' },
  { h: 38, kind: 'normal' }, { h: 46, kind: 'normal' }, { h: 40, kind: 'normal' },
  { h: 50, kind: 'normal' }, { h: 18, kind: 'fail' },  { h: 42, kind: 'normal' },
  { h: 36, kind: 'normal' }, { h: 48, kind: 'normal' }, { h: 40, kind: 'normal' },
  { h: 38, kind: 'normal' }, { h: 50, kind: 'normal' }, { h: 16, kind: 'fail' },
  { h: 42, kind: 'normal' }, { h: 36, kind: 'normal' }, { h: 58, kind: 'normal' },
  { h: 38, kind: 'normal' }, { h: 42, kind: 'normal' }, { h: 46, kind: 'normal' },
  { h: 20, kind: 'fail' },  { h: 38, kind: 'normal' }, { h: 42, kind: 'normal' },
  { h: 48, kind: 'normal' }, { h: 18, kind: 'fail' },  { h: 42, kind: 'normal' },
  { h: 50, kind: 'normal' }, { h: 16, kind: 'fail' },  { h: 54, kind: 'curr' },
];

function BarChart() {
  return (
    <div className="rounded-[8px] border border-hairline bg-surface-soft px-[14px] pb-[10px] pt-3">
      <div className="mb-2 flex justify-between font-sans text-[10px] font-semibold uppercase tracking-[1.2px] text-muted">
        <span>duration · last 30 runs</span>
        <span>0 — 9 min</span>
      </div>
      <div className="flex h-[64px] items-end gap-[3px]">
        {BAR_DATA.map((bar, i) => (
          <div
            key={i}
            className={`min-w-[5px] flex-1 rounded-t-[2px] ${
              bar.kind === 'fail' ? 'bg-error' : bar.kind === 'curr' ? 'bg-primary' : 'bg-surface-card'
            }`}
            style={{ height: `${bar.h}%` }}
          />
        ))}
      </div>
      <div className="flex justify-between pt-[5px] font-mono text-[10px] text-muted-soft">
        <span>R-9114</span>
        <span>R-9121</span>
        <span>R-9128</span>
        <span>R-9135</span>
        <span>R-9143 (now)</span>
      </div>
    </div>
  );
}

// ─── Step dots ────────────────────────────────────────────────────────────────

type StepState = 'ok' | 'fail' | 'warn' | 'skip' | 'empty';

const stepColor: Record<StepState, string> = {
  ok: 'bg-success',
  fail: 'bg-error',
  warn: 'bg-warning',
  skip: 'bg-hairline',
  empty: 'bg-surface-card',
};

function StepGrid({ steps }: { steps: StepState[] }) {
  return (
    <div className="inline-flex gap-[2px]">
      {steps.map((s, i) => (
        <span key={i} className={`h-[14px] w-[10px] rounded-[1.5px] ${stepColor[s]}`} />
      ))}
    </div>
  );
}

// ─── Chip (page-local) ────────────────────────────────────────────────────────

function Chip({ active, children }: { active?: boolean; children: ReactNode }) {
  return (
    <span
      className={`inline-flex h-[22px] cursor-pointer items-center gap-[6px] rounded-[6px] border border-hairline px-2 font-sans text-[11px] ${
        active ? 'bg-surface-cream-strong font-medium text-ink' : 'bg-canvas text-body'
      }`}
    >
      {children}
    </span>
  );
}

// ─── Pill with status dot (design uses a small circle before the label) ───────

function StatusPill({ kind, label }: { kind: PillKind; label: string }) {
  return (
    <Pill kind={kind}>
      <span className="h-[6px] w-[6px] flex-shrink-0 rounded-full bg-white/85" />
      {label}
    </Pill>
  );
}

// ─── Expanded step row ────────────────────────────────────────────────────────

const EXPAND_STEPS = [
  { name: '01 Source',  dur: '0.4s',              fail: false },
  { name: '02 Norm.',   dur: '0.9s',              fail: false },
  { name: '03 Prio.',   dur: '0.2s',              fail: false },
  { name: '04 WF',      dur: '0.1s',              fail: false },
  { name: '05 Worker',  dur: '0.1s',              fail: false },
  { name: '06 WS',      dur: '3.1s',              fail: false },
  { name: '07 Execute', dur: '1m 52s · vitest 3 fail', fail: true },
  { name: '08—10',      dur: 'skipped',           fail: false },
];

function ExpandedRow() {
  return (
    <tr>
      <td colSpan={6} className="border-b border-hairline bg-surface-soft px-3 py-[10px]">
        <div className="mb-[6px] font-sans text-[10px] font-semibold uppercase tracking-[1.1px] text-muted">
          Step-by-step · failed at 07 Execute
        </div>
        <div className="flex gap-[5px]">
          {EXPAND_STEPS.map((s) => (
            <div
              key={s.name}
              className={`flex min-w-0 flex-col gap-[2px] rounded-[5px] border px-2 py-[5px] ${
                s.fail
                  ? 'border-[rgba(198,69,69,0.4)] bg-[rgba(198,69,69,0.06)]'
                  : 'border-hairline-soft bg-canvas'
              }`}
            >
              <span
                className={`font-mono text-[9px] font-semibold uppercase tracking-[1px] ${
                  s.fail ? 'text-error' : 'text-muted-soft'
                }`}
              >
                {s.name}
              </span>
              <span className="font-mono text-[10px] text-muted">{s.dur}</span>
            </div>
          ))}
        </div>
      </td>
    </tr>
  );
}

// ─── Run history table ────────────────────────────────────────────────────────

type RunRow = {
  id: string;
  task: string;
  version: string;
  duration: string;
  steps: StepState[];
  pillKind: PillKind;
  pillLabel: string;
  expandable?: boolean;
};

const RUN_ROWS: RunRow[] = [
  { id: 'R-9143', task: 'CON-1247', version: 'v0.3.2', duration: '4m 18s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','warn','empty'], pillKind: 'running',  pillLabel: 'at gate' },
  { id: 'R-9142', task: 'CON-1248', version: 'v0.3.2', duration: '3m 51s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','ok','ok'],       pillKind: 'success',  pillLabel: 'pass' },
  { id: 'R-9140', task: 'CON-1242', version: 'v0.3.2', duration: '2m 04s',
    steps: ['ok','ok','ok','ok','ok','ok','fail','skip','skip','skip'], pillKind: 'failed',   pillLabel: 'fail', expandable: true },
  { id: 'R-9138', task: 'CON-1245', version: 'v0.3.2', duration: '2h 11m',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','warn','empty'], pillKind: 'gate',     pillLabel: 'at gate' },
  { id: 'R-9137', task: 'CON-1240', version: 'v0.3.2', duration: '3m 22s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','ok','ok'],       pillKind: 'success',  pillLabel: 'pass' },
  { id: 'R-9135', task: 'CON-1238', version: 'v0.3.2', duration: '3m 41s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','ok','ok'],       pillKind: 'success',  pillLabel: 'pass' },
  { id: 'R-9131', task: 'CON-1235', version: 'v0.3.2', duration: '5m 12s',
    steps: ['ok','ok','ok','ok','warn','ok','ok','ok','ok','ok'],     pillKind: 'warn',     pillLabel: 'retried' },
  { id: 'R-9128', task: 'CON-1231', version: 'v0.3.2', duration: '3m 02s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','ok','ok'],       pillKind: 'success',  pillLabel: 'pass' },
  { id: 'R-9124', task: 'CON-1229', version: 'v0.3.2', duration: '4m 02s',
    steps: ['ok','ok','ok','ok','ok','ok','ok','ok','ok','ok'],       pillKind: 'success',  pillLabel: 'pass' },
];

function HistoryTable() {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="flex-1 overflow-auto rounded-[8px] border border-hairline bg-canvas">
      <table className="w-full border-separate border-spacing-0 text-[11.5px]">
        <thead>
          <tr>
            <th className="w-[72px] border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">Run</th>
            <th className="w-[84px] border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">Task</th>
            <th className="w-[60px] border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">v</th>
            <th className="w-[74px] border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">Dur</th>
            <th className="border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">Steps</th>
            <th className="w-[74px] border-b border-t border-hairline bg-surface-soft px-3 py-2 text-left font-sans text-[9.5px] font-semibold uppercase tracking-[1.2px] text-muted">Outcome</th>
          </tr>
        </thead>
        <tbody>
          {RUN_ROWS.map((row) => (
            <>
              <tr
                key={row.id}
                className={row.expandable ? 'cursor-pointer hover:bg-surface-soft/60' : ''}
                onClick={row.expandable ? () => setExpanded((v) => !v) : undefined}
              >
                <td className="border-b border-hairline-soft px-3 py-2 font-mono align-middle">{row.id}</td>
                <td className="border-b border-hairline-soft px-3 py-2 font-mono align-middle">{row.task}</td>
                <td className="border-b border-hairline-soft px-3 py-2 font-mono align-middle">{row.version}</td>
                <td className="border-b border-hairline-soft px-3 py-2 font-mono align-middle">{row.duration}</td>
                <td className="border-b border-hairline-soft px-3 py-2 align-middle">
                  <StepGrid steps={row.steps} />
                </td>
                <td className="border-b border-hairline-soft px-3 py-2 align-middle">
                  <StatusPill kind={row.pillKind} label={row.pillLabel} />
                </td>
              </tr>
              {row.expandable && expanded && <ExpandedRow key={`${row.id}-exp`} />}
            </>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ─── YAML diff ────────────────────────────────────────────────────────────────

type DiffLine = { type: 'hunk' | 'ctx' | 'add' | 'del'; text: string };

const DIFF_LINES: DiffLine[] = [
  { type: 'hunk', text: '@@ -42,4 +42,9 @@ gate:' },
  { type: 'ctx',  text: '  kind: merge-approval' },
  { type: 'ctx',  text: '  approvers: 1' },
  { type: 'add',  text: '+ auto_approve_when:' },
  { type: 'add',  text: '+   - checks: green' },
  { type: 'add',  text: '+   - migration_touched: false' },
  { type: 'add',  text: '+   - diff_loc: { lt: 400 }' },
  { type: 'ctx',  text: '  policy: policies/high-risk-paths.yaml' },
  { type: 'hunk', text: '@@ -71,3 +76,3 @@ sinks:' },
  { type: 'del',  text: '- - kind: update_linear_basic' },
  { type: 'add',  text: '+ - kind: update_linear  # renamed' },
  { type: 'ctx',  text: '  - kind: comment_pr' },
];

const diffLineColor: Record<DiffLine['type'], string> = {
  hunk: 'text-muted-soft',
  ctx:  'text-on-dark',
  add:  'text-[#7fcc8f]',
  del:  'text-[#e58176]',
};

function YamlDiff() {
  return (
    <div className="rounded-[8px] bg-surface-dark px-[14px] py-3 font-mono text-[11px] leading-[1.65]">
      <div className="mb-1 text-[10.5px] text-muted-soft">workflows/delivery/default.yaml</div>
      {DIFF_LINES.map((line, i) => (
        <div key={i} className={diffLineColor[line.type]}>
          {line.text}
        </div>
      ))}
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

const workflowsRailItems = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'workflows',
}));

export function PublishWorkflowPage() {
  return (
    <AppShell
      rail={<Rail items={workflowsRailItems} bottomItems={ACCOUNT_ITEMS} />}
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', 'Workflows', 'delivery/default', 'Publish']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Publish v0.3.3 ·{' '}
                <span className="font-mono text-[18px] text-muted">delivery/default</span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Will roll out to{' '}
                <strong className="font-semibold text-body-strong">4 active projects</strong>{' '}
                · in-flight runs continue on v0.3.2 · rollback in 1 click
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">View as YAML ↗</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          {/* Full-bleed split: cancel ScreenBody padding with negative margins */}
          <div className="-mx-7 -mb-6 -mt-[18px] flex min-h-0 flex-1 overflow-hidden">
            {/* Left: Publish panel */}
            <div className="flex flex-1 flex-col gap-3 overflow-auto border-r border-hairline px-[22px] py-[18px]">
              <div>
                <h3 className="m-0 mb-[2px] font-serif text-[22px] font-medium leading-tight tracking-[-0.4px] text-ink">
                  Publish v0.3.3
                </h3>
                <p className="text-[12px] text-muted">
                  From{' '}
                  <strong className="font-semibold text-ink">draft v0.3.2 · r-2026.05.24</strong>{' '}
                  ·{' '}
                  <strong className="font-semibold text-ink">2 commits</strong>{' '}
                  by kanagawa-ops · clean dry-run on fixture
                </p>
              </div>

              <div>
                <span className="mb-1 block font-sans text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
                  Changes since v0.3.2
                </span>
                <YamlDiff />
              </div>

              <div>
                <span className="mb-1 block font-sans text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
                  Version notes
                </span>
                <div className="min-h-[64px] rounded-[8px] border border-hairline bg-surface-soft px-3 py-[10px] text-[12.5px] leading-[1.5] text-body">
                  <span className="mb-1 block font-mono text-[11px] italic text-muted-soft">
                    // pre-filled from your changelog
                  </span>
                  Add auto-approve when checks green and no migrations.
                  <br />
                  Tighten merge gate — require diff &lt; 400 LOC.
                  <br />
                  Rename{' '}
                  <span className="rounded-[3px] bg-surface-card px-[5px] py-[1px] font-mono text-[11px]">
                    update_linear_basic
                  </span>{' '}
                  →{' '}
                  <span className="rounded-[3px] bg-surface-card px-[5px] py-[1px] font-mono text-[11px]">
                    update_linear
                  </span>
                  .
                </div>
              </div>

              <div className="mt-auto flex items-center gap-2 border-t border-hairline-soft pt-[14px]">
                <span className="text-[11.5px] text-muted">
                  11 in-flight runs will finish on{' '}
                  <strong className="font-semibold text-ink">v0.3.2</strong>.
                </span>
                <Btn variant="secondary" size="sm">Cancel</Btn>
                <Btn variant="primary" size="sm">Publish</Btn>
              </div>
            </div>

            {/* Right: Run history panel */}
            <div className="flex flex-1 flex-col gap-3 overflow-auto px-[22px] py-[18px]">
              <h3 className="m-0 font-serif text-[22px] font-medium leading-tight tracking-[-0.4px] text-ink">
                Run history
              </h3>

              <div className="flex flex-wrap items-center gap-[6px]">
                <Chip active>
                  version: <strong className="font-semibold text-ink">v0.3.2</strong>{' '}
                  <span className="text-muted">×</span>
                </Chip>
                <Chip>last 30 runs</Chip>
                <Chip>all projects</Chip>
                <span className="ml-auto font-mono text-[10.5px] text-muted-soft">
                  window: 24h · 30 runs
                </span>
              </div>

              <BarChart />

              <HistoryTable />
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
