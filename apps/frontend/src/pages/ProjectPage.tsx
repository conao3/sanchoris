import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, Tag, type PillKind, type TagTone } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Rail ──────────────────────────────────────────────────────────────────────

const projectRailItems = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'projects',
}));

// ─── Page-local primitives ───────────────────────────────────────────────────

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

type SourceType = 'linear' | 'github';

const SOURCE_ICON_CLASS: Record<SourceType, string> = {
  linear: 'bg-[rgba(93,184,166,0.2)] text-[#2c6e62]',
  github: 'bg-surface-dark text-on-dark',
};

const SOURCE_ICON_LABEL: Record<SourceType, string> = {
  linear: 'L',
  github: 'G',
};

function SourceBadge({ type }: { type: SourceType }) {
  return (
    <span
      className={`inline-flex h-[18px] w-[18px] items-center justify-center rounded-[4px] font-mono text-[10px] font-bold ${SOURCE_ICON_CLASS[type]}`}
    >
      {SOURCE_ICON_LABEL[type]}
    </span>
  );
}

type AvatarTone = 'default' | 'coral' | 'amber';

const AVATAR_CLASS: Record<AvatarTone, string> = {
  default: 'bg-surface-card text-ink',
  coral: 'bg-primary text-white',
  amber: 'bg-accent-amber text-[#4a3306]',
};

function SmAvatar({ initials, tone = 'default' }: { initials: string; tone?: AvatarTone }) {
  return (
    <span
      className={`inline-flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center rounded-full border-2 border-canvas text-[9.5px] font-semibold ${AVATAR_CLASS[tone]}`}
    >
      {initials}
    </span>
  );
}

type BarTone = 'default' | 'navy' | 'amber' | 'red' | 'green';

const BAR_FILL_CLASS: Record<BarTone, string> = {
  default: 'bg-primary',
  navy: 'bg-surface-dark',
  amber: 'bg-warning',
  red: 'bg-error',
  green: 'bg-success',
};

function ProgressBar({ pct, tone = 'default' }: { pct: number; tone?: BarTone }) {
  return (
    <div className="inline-block h-[6px] w-[80px] overflow-hidden rounded-[3px] bg-surface-card align-middle">
      <div
        className={`h-full rounded-[3px] ${BAR_FILL_CLASS[tone]}`}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

function StepChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-[22px] items-center rounded-[6px] border border-hairline bg-canvas px-[8px] py-[3px] text-[11px] text-body">
      {children}
    </span>
  );
}

// ─── Gate card ────────────────────────────────────────────────────────────────

type GateCardDef = {
  tagTone: TagTone;
  tagLabel: string;
  meta: string;
  title: ReactNode;
  desc: ReactNode;
  primaryLabel: string;
  secondaryLabel: string;
  age: string;
  primaryHref?: string;
  secondaryHref?: string;
};

function GateCard({ card }: { card: GateCardDef }) {
  return (
    <div className="mb-[10px] rounded-[8px] border border-hairline bg-canvas p-[11px_12px]">
      <div className="mb-1 flex items-center gap-[6px]">
        <Tag tone={card.tagTone}>{card.tagLabel}</Tag>
        <span className="ml-auto font-mono text-[11px] text-muted">{card.meta}</span>
      </div>
      <div className="mb-1 text-[13px] font-medium leading-[1.35] text-ink">{card.title}</div>
      <div className="mb-2 text-[11.5px] leading-[1.45] text-muted">{card.desc}</div>
      <div className="flex items-center gap-[6px]">
        <Btn variant="primary" size="sm" onClick={card.primaryHref ? () => navigate(card.primaryHref!) : undefined}>{card.primaryLabel}</Btn>
        <Btn variant="secondary" size="sm" onClick={card.secondaryHref ? () => navigate(card.secondaryHref!) : undefined}>{card.secondaryLabel}</Btn>
        <span className="ml-auto font-mono text-[10.5px] text-muted-soft">{card.age}</span>
      </div>
    </div>
  );
}

// ─── Data ─────────────────────────────────────────────────────────────────────

type RunRow = {
  id: string;
  title: string;
  sub: ReactNode;
  source: SourceType;
  worker: string;
  step: string;
  age: string;
  status: { kind: PillKind; label: string };
  owner: { initials: string; tone: AvatarTone };
  progress: { pct: number; tone: BarTone };
};

const RUNS: RunRow[] = [
  {
    id: 'CON-1247',
    title: 'Add OAuth login for vendor portal',
    sub: <>run R-9143 · branch <span className="font-mono">issue-con-1247</span></>,
    source: 'linear',
    worker: 'codex',
    step: 'validate',
    age: '4m 18s',
    status: { kind: 'running', label: 'running' },
    owner: { initials: 'KO', tone: 'coral' },
    progress: { pct: 72, tone: 'default' },
  },
  {
    id: 'CON-1248',
    title: 'Per-tenant rate limits on /api/checkout',
    sub: <>run R-9142 · branch <span className="font-mono">feat/tenant-rate-limits</span></>,
    source: 'linear',
    worker: 'claude-code',
    step: 'execute',
    age: '14m 27s',
    status: { kind: 'validating', label: 'validating' },
    owner: { initials: 'ER', tone: 'amber' },
    progress: { pct: 55, tone: 'navy' },
  },
  {
    id: 'CON-1249',
    title: 'Webhook signature mismatch on /api/stripe',
    sub: <>run R-9141 · branch <span className="font-mono">hotfix/stripe-sig</span></>,
    source: 'github',
    worker: 'codex',
    step: 'execute',
    age: '11m 04s',
    status: { kind: 'running', label: 'running' },
    owner: { initials: 'OC', tone: 'default' },
    progress: { pct: 38, tone: 'default' },
  },
  {
    id: 'CON-1245',
    title: 'Retry/backoff on webhook fanout',
    sub: <>run R-9138 · PR #87 awaiting merge gate</>,
    source: 'linear',
    worker: 'openhands',
    step: 'gate',
    age: '2h 11m',
    status: { kind: 'gate', label: 'awaiting gate' },
    owner: { initials: 'KO', tone: 'coral' },
    progress: { pct: 92, tone: 'amber' },
  },
  {
    id: 'CON-1241',
    title: 'Migrate orders_v2 schema (conflicting field shapes)',
    sub: <>run R-9077 · paused at normalize step · needs human</>,
    source: 'linear',
    worker: 'claude-code',
    step: 'normalize',
    age: '2 d',
    status: { kind: 'failed', label: 'failed' },
    owner: { initials: 'SH', tone: 'default' },
    progress: { pct: 32, tone: 'red' },
  },
  {
    id: 'CON-1243',
    title: 'Fix login redirect loop after SSO timeout',
    sub: <>run R-9091 · merged via PR #481</>,
    source: 'linear',
    worker: 'codex',
    step: 'done',
    age: '1 d',
    status: { kind: 'done', label: 'done' },
    owner: { initials: 'OC', tone: 'default' },
    progress: { pct: 100, tone: 'green' },
  },
];

const GATE_CARDS: GateCardDef[] = [
  {
    tagTone: 'coral-soft',
    tagLabel: 'merge',
    meta: 'PR #482',
    title: <>Merge OAuth login changes to <span className="font-mono">main</span></>,
    desc: <>Policy <strong className="font-medium text-ink">prod-touching-files</strong> matched <span className="font-mono">apps/auth/**</span>. 1 / 2 reviewers approved.</>,
    primaryLabel: 'Approve',
    secondaryLabel: 'View diff',
    age: '22 m',
    primaryHref: '/runs/r-9143/gate',
    secondaryHref: '/pull-requests',
  },
  {
    tagTone: 'amber',
    tagLabel: 'deploy',
    meta: 'v0.19.0',
    title: 'Deploy stripe-bridge to prod',
    desc: <>Policy <strong className="font-medium text-ink">prod-deploy-business-hours</strong>. Outside window (23:14 JST).</>,
    primaryLabel: 'Override',
    secondaryLabel: 'Defer',
    age: '1 h',
    primaryHref: '/gates',
    secondaryHref: '/gates',
  },
  {
    tagTone: 'err',
    tagLabel: 'secret rotation',
    meta: 'STRIPE_*',
    title: 'Confirm new Stripe webhook secret',
    desc: <>Worker found expired secret in <span className="font-mono">stripe-bridge/.env</span>.</>,
    primaryLabel: 'Confirm',
    secondaryLabel: 'Reject',
    age: '3 h',
    primaryHref: '/gates',
    secondaryHref: '/gates',
  },
];

// ─── Pill with dot indicator ──────────────────────────────────────────────────

function RunPill({ kind, label }: { kind: PillKind; label: string }) {
  const showDot = kind === 'running' || kind === 'validating' || kind === 'gate' || kind === 'failed' || kind === 'done';
  return (
    <Pill kind={kind}>
      {showDot && (
        <span
          className={`h-[6px] w-[6px] rounded-full ${
            kind === 'done' ? 'bg-muted-soft' : 'bg-[rgba(255,255,255,0.85)]'
          }`}
          aria-hidden="true"
        />
      )}
      {label}
    </Pill>
  );
}

// ─── Table row ────────────────────────────────────────────────────────────────

function RunTableRow({ row }: { row: RunRow }) {
  return (
    <tr className="cursor-pointer border-b border-hairline-soft last:border-b-0" onClick={() => navigate('/runs/r-9143')}>
      <td className="px-[14px] py-[11px] align-middle">
        <div>
          <span className="font-mono text-[11px] text-muted">{row.id}</span>
          {' · '}
          <span className="font-medium text-[13px] text-ink">{row.title}</span>
        </div>
        <div className="mt-[2px] text-[11px] text-muted">{row.sub}</div>
      </td>
      <td className="py-[11px] pr-[14px] align-middle" style={{ width: '90px' }}>
        <SourceBadge type={row.source} />
      </td>
      <td className="py-[11px] pr-[14px] align-middle font-mono text-[12px] text-body" style={{ width: '100px' }}>
        {row.worker}
      </td>
      <td className="py-[11px] pr-[14px] align-middle" style={{ width: '140px' }}>
        <StepChip>{row.step}</StepChip>
      </td>
      <td className="py-[11px] pr-[14px] align-middle font-mono text-[12px] text-muted" style={{ width: '80px' }}>
        {row.age}
      </td>
      <td className="py-[11px] pr-[14px] align-middle" style={{ width: '120px' }}>
        <RunPill kind={row.status.kind} label={row.status.label} />
      </td>
      <td className="py-[11px] pr-[14px] align-middle" style={{ width: '80px' }}>
        <SmAvatar initials={row.owner.initials} tone={row.owner.tone} />
      </td>
      <td className="py-[11px] pr-[14px] align-middle" style={{ width: '90px' }}>
        <ProgressBar pct={row.progress.pct} tone={row.progress.tone} />
      </td>
    </tr>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

export function ProjectPage() {
  return (
    <AppShell rail={<Rail items={projectRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Projects', 'acme-platform']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                acme-platform{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="running">4 runs</Pill>
                </span>
              </h1>
              <div className="mt-1 flex flex-wrap items-center gap-3 text-[12px] text-muted">
                <span className="font-mono">github.com/acme/acme-platform</span>
                <span className="text-muted-soft">·</span>
                <span className="inline-flex items-center gap-[6px] rounded-full bg-surface-card px-[8px] py-[2px] text-[11px]">
                  workflow <strong className="font-semibold text-ink">delivery/default v0.3.2</strong>
                </span>
                <span className="inline-flex items-center gap-[6px] rounded-full bg-surface-card px-[8px] py-[2px] text-[11px]">
                  branch <strong className="font-mono font-semibold text-ink">main</strong>
                </span>
                <Pill kind="warn">
                  <span className="h-[6px] w-[6px] rounded-full bg-[rgba(133,98,0,0.4)]" aria-hidden="true" />
                  protection: required-2-reviews
                </Pill>
              </div>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Open in GitHub ↗</Btn>
              <Btn variant="primary">+ Start task</Btn>
            </div>
          </div>

          {/* ptabs — negative bottom margin to sit flush at ScreenHead's border */}
          <div className="-mb-[14px] mt-[14px] flex gap-1">
            {[
              { label: 'Runs', count: '6', active: true },
              { label: 'Workpads', count: '4' },
              { label: 'Workflows', count: '3' },
              { label: 'History', count: '128' },
              { label: 'Gates', count: '3' },
              { label: 'Settings' },
            ].map((tab) => (
              <span
                key={tab.label}
                className={`cursor-pointer px-[14px] py-[8px] text-[13px] ${
                  tab.active
                    ? 'border-b-2 border-primary font-semibold text-ink'
                    : 'border-b-2 border-transparent text-muted'
                }`}
              >
                {tab.label}
                {tab.count && (
                  <span className="ml-1 font-mono text-[11px] text-muted-soft">{tab.count}</span>
                )}
              </span>
            ))}
          </div>
        </ScreenHead>

        {/* F02 body: full-bleed 2-column grid, no padding */}
        <div
          className="min-h-0 flex-1 overflow-hidden"
          style={{ display: 'grid', gridTemplateColumns: '1fr 320px' }}
        >
          {/* pmain: left column */}
          <div className="flex min-h-0 min-w-0 flex-col overflow-hidden">
            {/* ptoolbar */}
            <div className="flex flex-shrink-0 items-center gap-[10px] border-b border-hairline-soft px-7 py-3">
              <Chip active count={9}>All</Chip>
              <Chip count={4}>Running</Chip>
              <Chip count={1}>Awaiting gate</Chip>
              <Chip count={3}>Done</Chip>
              <Chip count={1}>Failed</Chip>
              <span className="ml-auto flex items-center gap-[6px] text-[12px] text-muted">
                <span className="inline-flex items-center rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">/</span>
                {' filter · '}
                <span className="inline-flex items-center rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">J</span>
                <span className="inline-flex items-center rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">K</span>
                {' navigate'}
              </span>
            </div>

            {/* runs table */}
            <div className="min-h-0 flex-1 overflow-auto">
              <table className="w-full border-collapse text-left">
                <thead>
                  <tr>
                    {[
                      { label: 'Task', width: undefined },
                      { label: 'Source', width: '90px' },
                      { label: 'Worker', width: '100px' },
                      { label: 'Current step', width: '140px' },
                      { label: 'Age', width: '80px' },
                      { label: 'Status', width: '120px' },
                      { label: 'Owner', width: '80px' },
                      { label: 'Progress', width: '90px' },
                    ].map(({ label, width }) => (
                      <th
                        key={label}
                        className="border-b border-t border-hairline bg-surface-soft py-[9px] pl-[14px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                        style={width ? { width } : undefined}
                      >
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {RUNS.map((row) => (
                    <RunTableRow key={row.id} row={row} />
                  ))}
                </tbody>
              </table>
            </div>
          </div>

          {/* prail: right column */}
          <div className="flex min-h-0 flex-col overflow-hidden border-l border-hairline bg-surface-soft p-[18px_16px]">
            <h3 className="m-0 mb-3 flex items-center gap-2 font-serif text-[18px] font-medium leading-none tracking-[-0.3px] text-ink">
              Gate approvals{' '}
              <span className="font-mono text-[12px] font-medium text-muted">3</span>
            </h3>
            {GATE_CARDS.map((card, i) => (
              <GateCard key={i} card={card} />
            ))}
          </div>
        </div>
      </Screen>
    </AppShell>
  );
}
