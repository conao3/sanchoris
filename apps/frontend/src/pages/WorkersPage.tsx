import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, type PillKind } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Rail items ─────────────────────────────────────────────────────────────

const workerRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'workers',
}));

// ─── WStat component ─────────────────────────────────────────────────────────

function WStat({
  label,
  value,
  valueSuffix,
  sub,
  subClass,
}: {
  label: string;
  value: string;
  valueSuffix?: string;
  sub: string;
  subClass?: string;
}) {
  return (
    <div className="rounded-[10px] border border-hairline bg-surface-soft p-[12px_14px]">
      <div className="mb-1 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
        {label}
      </div>
      <div className="font-serif text-[26px] font-medium leading-[1.1] tracking-[-0.3px] text-ink">
        {value}
        {valueSuffix && (
          <small className="font-mono text-[14px] font-normal text-muted">{valueSuffix}</small>
        )}
      </div>
      <div className={`mt-1 font-mono text-[11px] text-muted ${subClass ?? ''}`}>{sub}</div>
    </div>
  );
}

// ─── Gauge component ─────────────────────────────────────────────────────────

type GaugeColor = 'primary' | 'success' | 'warning' | 'muted-soft';

const gaugeFillStyle: Record<GaugeColor, string> = {
  primary: '#cc785c',
  success: '#5db872',
  warning: '#d4a017',
  'muted-soft': '#8e8b82',
};

function Gauge({
  percent,
  label,
  color,
}: {
  percent: number;
  label: string;
  color?: GaugeColor;
}) {
  return (
    <div className="flex min-w-[130px] items-center gap-[7px]">
      <div className="h-[6px] flex-1 overflow-hidden rounded-[3px] bg-surface-card">
        {percent > 0 && color && (
          <div
            className="h-full rounded-[3px]"
            style={{ width: `${percent}%`, background: gaugeFillStyle[color] }}
          />
        )}
      </div>
      <span className="min-w-[36px] text-right font-mono text-[11px] text-muted">{label}</span>
    </div>
  );
}

// ─── Worker data ──────────────────────────────────────────────────────────────

type WorkerStatus = 'running' | 'validating' | 'cooling-down' | 'idle' | 'passive';

type WorkerRow = {
  name: string;
  id: string;
  subtitle: string;
  status: WorkerStatus;
  taskId: string | null;
  task: string;
  loadPercent: number;
  loadLabel: string;
  loadColor?: GaugeColor;
  cpuMem: string;
  workspace: string;
  age: string;
};

const statusPill: Record<WorkerStatus, PillKind> = {
  running: 'running',
  validating: 'validating',
  'cooling-down': 'default',
  idle: 'info-soft',
  passive: 'default',
};

const WORKER_ROWS: WorkerRow[] = [
  {
    name: 'codex',
    id: 'w-cx-1',
    subtitle: 'code edit / large diff',
    status: 'running',
    taskId: 'CON-1247',
    task: 'Add OAuth login for vendor portal',
    loadPercent: 70,
    loadLabel: '7 / 10',
    loadColor: 'primary',
    cpuMem: '62% · 1.4G',
    workspace: 'ws-acme-plt-1d3f',
    age: '4m 18s',
  },
  {
    name: 'codex',
    id: 'w-cx-2',
    subtitle: 'code edit / large diff',
    status: 'running',
    taskId: 'CON-1249',
    task: 'Webhook signature mismatch on /api/stripe',
    loadPercent: 48,
    loadLabel: '5 / 10',
    loadColor: 'success',
    cpuMem: '41% · 0.9G',
    workspace: 'ws-stripe-771a',
    age: '11m 04s',
  },
  {
    name: 'claude-code',
    id: 'w-cc-1',
    subtitle: 'spec → impl / agentic',
    status: 'validating',
    taskId: 'CON-1248',
    task: 'Per-tenant rate limits on /api/checkout',
    loadPercent: 88,
    loadLabel: '5 / 6',
    loadColor: 'warning',
    cpuMem: '79% · 2.1G',
    workspace: 'ws-growth-c812',
    age: '14m 27s',
  },
  {
    name: 'claude-code',
    id: 'w-cc-2',
    subtitle: 'spec → impl / agentic',
    status: 'cooling-down',
    taskId: null,
    task: '— finished R-9141 12s ago',
    loadPercent: 12,
    loadLabel: '0 / 6',
    loadColor: 'muted-soft',
    cpuMem: '8% · 0.3G',
    workspace: 'ws-acme-plt-99e1 (gc)',
    age: '—',
  },
  {
    name: 'openhands',
    id: 'w-oh-1',
    subtitle: 'browser / multi-step',
    status: 'running',
    taskId: 'CON-1250',
    task: 'Bulk CSV import · gathering Linear context',
    loadPercent: 38,
    loadLabel: '3 / 8',
    loadColor: 'success',
    cpuMem: '29% · 0.6G',
    workspace: 'ws-acme-plt-b4f0',
    age: '2m 51s',
  },
  {
    name: 'symphony-runner',
    id: 'w-sr-1',
    subtitle: 'deterministic / scripted',
    status: 'idle',
    taskId: null,
    task: '—',
    loadPercent: 0,
    loadLabel: '0 / 16',
    cpuMem: '2% · 0.1G',
    workspace: '—',
    age: '—',
  },
  {
    name: 'symphony-runner',
    id: 'w-sr-2',
    subtitle: 'deterministic / scripted',
    status: 'running',
    taskId: 'cron-drift',
    task: 'Nightly schema-drift sweep',
    loadPercent: 25,
    loadLabel: '4 / 16',
    loadColor: 'success',
    cpuMem: '22% · 0.5G',
    workspace: 'ws-cron-3a91',
    age: '23m 11s',
  },
  {
    name: 'human-review',
    id: 'pool',
    subtitle: 'async reply pool · 4 humans',
    status: 'passive',
    taskId: null,
    task: '7 tasks pending human input',
    loadPercent: 0,
    loadLabel: '—',
    cpuMem: '—',
    workspace: '—',
    age: '—',
  },
];

// ─── Worker table row ─────────────────────────────────────────────────────────

function WorkerTableRow({ row }: { row: WorkerRow }) {
  return (
    <tr className="border-b border-hairline-soft last:border-b-0">
      <td className="px-4 py-[13px] align-middle">
        <div className="flex flex-col gap-[2px]">
          <span className="text-[13px] font-semibold text-ink">
            {row.name} · <span className="font-mono text-[11px] font-normal text-muted">{row.id}</span>
          </span>
          <span className="font-mono text-[11px] text-muted">{row.subtitle}</span>
        </div>
      </td>
      <td className="px-4 py-[13px] align-middle">
        <Pill kind={statusPill[row.status]}>{row.status}</Pill>
      </td>
      <td className="px-4 py-[13px] align-middle">
        {row.taskId ? (
          <span className="text-[13px] text-ink">
            <span className="font-mono text-[11px] text-muted">{row.taskId}</span>{' '}
            {row.task}
          </span>
        ) : (
          <span className="text-[12px] text-muted">{row.task}</span>
        )}
      </td>
      <td className="px-4 py-[13px] align-middle">
        <Gauge percent={row.loadPercent} label={row.loadLabel} color={row.loadColor} />
      </td>
      <td className="px-4 py-[13px] align-middle font-mono text-[12px] text-muted">
        {row.cpuMem}
      </td>
      <td className="px-4 py-[13px] align-middle font-mono text-[12px] text-muted">
        {row.workspace}
      </td>
      <td className="px-4 py-[13px] align-middle font-mono text-[12px] text-muted">
        {row.age}
      </td>
    </tr>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export function WorkersPage() {
  return (
    <AppShell rail={<Rail items={workerRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Workers']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Worker pool{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="running">3 / 8 busy</Pill>
                  <Pill kind="default">5 idle</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                All worker kinds across the gateway · each task gets its own isolated workspace ·
                auto-scaling within <strong className="font-semibold text-body-strong">quota 12</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">View capacity history</Btn>
              <Btn variant="primary">+ Add worker</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex flex-col gap-4 overflow-hidden">
            {/* Stats strip */}
            <div className="grid grid-cols-4 gap-3">
              <WStat
                label="Active runs"
                value="11"
                valueSuffix=" · /16"
                sub="+3 in last 10 min"
              />
              <WStat
                label="Worker capacity"
                value="3 / 8"
                sub="claude-code near limit (88%)"
                subClass="text-primary-active"
              />
              <WStat
                label="Isolated workspaces"
                value="11"
                sub="avg lifespan 6m 12s"
              />
              <WStat
                label="Spend · 24h"
                value="¥18,420"
                sub="model & compute · budget 75%"
              />
            </div>

            {/* Worker table */}
            <div className="min-h-0 flex-1 overflow-auto rounded-[10px] border border-hairline bg-canvas">
              <table className="w-full border-collapse text-left">
                <thead>
                  <tr>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '220px' }}
                    >
                      Worker
                    </th>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '120px' }}
                    >
                      Status
                    </th>
                    <th className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
                      Running task
                    </th>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '180px' }}
                    >
                      Load
                    </th>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '120px' }}
                    >
                      CPU / mem
                    </th>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '200px' }}
                    >
                      Workspace
                    </th>
                    <th
                      className="border-b border-t border-hairline bg-surface-soft px-4 py-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                      style={{ width: '80px' }}
                    >
                      Age
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {WORKER_ROWS.map((row) => (
                    <WorkerTableRow key={`${row.name}-${row.id}`} row={row} />
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
