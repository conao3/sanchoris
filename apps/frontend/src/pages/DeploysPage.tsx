import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, type PillKind } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Types ────────────────────────────────────────────────────────────────────

type DotColor = 'green' | 'amber' | 'coral' | 'muted';

type StageData = {
  name: string;
  version: string;
  versionMuted?: boolean;
  versionSuccess?: boolean;
  dot: DotColor;
  health: string;
  paused?: boolean;
  arrowPrimary?: boolean;
};

type DeployRow = {
  env: string;
  message: string;
  pillKind: PillKind;
  pillText: string;
  time: string;
};

type DepCardData = {
  name: string;
  repo: string;
  statusKind: PillKind;
  statusText: string;
  stages: StageData[];
  recentDeploys: DeployRow[];
  gateResume?: { policy: string; description: string };
};

// ─── Data ─────────────────────────────────────────────────────────────────────

const DEP_CARDS: DepCardData[] = [
  {
    name: 'acme-platform',
    repo: 'github.com/acme/acme-platform',
    statusKind: 'default',
    statusText: 'active',
    stages: [
      { name: 'dev', version: 'v0.19.2-a7c', dot: 'green', health: 'healthy' },
      { name: 'staging', version: 'v0.19.0', dot: 'amber', health: 'p95 alert' },
      { name: 'prod', version: 'v0.18.7', dot: 'green', health: '99.97%' },
      { name: 'postdeploy', version: '—', versionMuted: true, dot: 'muted', health: 'ready' },
    ],
    recentDeploys: [
      { env: 'dev', message: 'fix(auth): ±90s skew · #481', pillKind: 'success', pillText: 'ok', time: '12m' },
      { env: 'stg', message: 'perf(checkout): lazy assets · #480', pillKind: 'success', pillText: 'ok', time: '38m' },
      { env: 'stg', message: 'chore: TS 5.5 · #214', pillKind: 'success', pillText: 'ok', time: '1h' },
      { env: 'prod', message: 'chore: TS 5.5 (canary) · #214', pillKind: 'gate', pillText: 'gate', time: 'queued' },
      { env: 'prod', message: 'feat: tenant rate-limits · #213', pillKind: 'done', pillText: 'scheduled', time: '09:00' },
    ],
  },
  {
    name: 'stripe-bridge',
    repo: 'github.com/acme/stripe-bridge',
    statusKind: 'gate',
    statusText: 'paused at gate',
    stages: [
      { name: 'dev', version: 'v0.19.0-rc4', dot: 'green', health: 'healthy' },
      { name: 'staging', version: 'v0.19.0', dot: 'green', health: 'healthy', arrowPrimary: true },
      { name: 'prod · gate', version: 'v0.18.7 → v0.19.0', dot: 'coral', health: '1h 08m hold', paused: true },
      { name: 'postdeploy', version: '—', versionMuted: true, dot: 'muted', health: 'blocked' },
    ],
    recentDeploys: [
      { env: 'stg', message: 'release v0.19.0 · #86', pillKind: 'success', pillText: 'ok', time: '2h' },
      { env: 'dev', message: 'feat: webhook retry · #87', pillKind: 'failed', pillText: 'e2e fail', time: '4h' },
      { env: 'stg', message: 'v0.18.7 baseline restore', pillKind: 'success', pillText: 'ok', time: "y'day" },
      { env: 'prod', message: 'v0.18.7 hotfix', pillKind: 'success', pillText: 'ok', time: '2 d' },
      { env: 'prod', message: 'v0.18.5', pillKind: 'success', pillText: 'ok', time: '5 d' },
    ],
    gateResume: {
      policy: 'prod-deploy-business-hours',
      description: 'resume at 09:00 JST or override now',
    },
  },
  {
    name: 'growth-api',
    repo: 'github.com/acme/growth-api',
    statusKind: 'default',
    statusText: 'active',
    stages: [
      { name: 'dev', version: 'v1.12.0-d31', dot: 'green', health: 'healthy' },
      { name: 'staging', version: 'v1.12.0', dot: 'green', health: 'healthy' },
      { name: 'prod', version: 'v1.11.4', dot: 'green', health: '99.99%' },
      { name: 'postdeploy', version: 'canary 5%', versionSuccess: true, dot: 'green', health: 'soaking' },
    ],
    recentDeploys: [
      { env: 'prod', message: 'canary 5% v1.12.0 EU-west', pillKind: 'running', pillText: 'soaking', time: '6m' },
      { env: 'stg', message: 'feat: tenant rate-limits · #213', pillKind: 'success', pillText: 'ok', time: '52m' },
      { env: 'dev', message: 'chore: TS 5.5 · #214', pillKind: 'success', pillText: 'ok', time: '1h' },
      { env: 'prod', message: 'v1.11.4 hotfix', pillKind: 'success', pillText: 'ok', time: '3 d' },
      { env: 'prod', message: 'v1.11.3', pillKind: 'success', pillText: 'ok', time: '1 w' },
    ],
  },
  {
    name: 'infra-bot',
    repo: 'github.com/acme/infra-bot',
    statusKind: 'default',
    statusText: 'quiet',
    stages: [
      { name: 'dev', version: 'v0.7.1', dot: 'green', health: 'healthy' },
      { name: 'staging', version: 'v0.7.0', dot: 'green', health: 'healthy' },
      { name: 'prod', version: 'v0.7.0', dot: 'green', health: 'healthy' },
      { name: 'postdeploy', version: '—', versionMuted: true, dot: 'muted', health: 'idle' },
    ],
    recentDeploys: [
      { env: 'dev', message: 'fix: handle empty PR body', pillKind: 'success', pillText: 'ok', time: "y'day" },
      { env: 'prod', message: 'v0.7.0', pillKind: 'success', pillText: 'ok', time: '4 d' },
      { env: 'prod', message: 'v0.6.4 hotfix', pillKind: 'success', pillText: 'ok', time: '9 d' },
      { env: 'prod', message: 'v0.6.3', pillKind: 'success', pillText: 'ok', time: '2 w' },
      { env: 'prod', message: 'v0.6.2', pillKind: 'success', pillText: 'ok', time: '3 w' },
    ],
  },
];

// ─── Local components ─────────────────────────────────────────────────────────

const dotBgClass: Record<DotColor, string> = {
  green: 'bg-success',
  amber: 'bg-warning',
  coral: 'bg-primary',
  muted: 'bg-muted-soft',
};

function PillDot({ kind }: { kind?: PillKind }) {
  const isDimmed = kind === 'done' || kind === 'queued';
  return (
    <span
      className={`inline-block h-[6px] w-[6px] flex-shrink-0 rounded-full ${isDimmed ? 'bg-muted-soft' : 'bg-[rgba(255,255,255,0.85)]'}`}
    />
  );
}

function Stage({ stage, isLast }: { stage: StageData; isLast: boolean }) {
  return (
    <div
      className={`relative rounded-[8px] border p-[8px_10px] ${stage.paused ? 'border-primary bg-[rgba(204,120,92,0.06)]' : 'border-hairline bg-surface-soft'}`}
    >
      <div className="mb-[3px] text-[9.5px] font-semibold uppercase tracking-[1.3px] text-muted">
        {stage.name}
      </div>
      <div
        className={`font-mono text-[11px] font-medium ${stage.versionMuted ? 'text-muted' : stage.versionSuccess ? 'text-success' : 'text-ink'}`}
      >
        {stage.version}
      </div>
      <div className="mt-1 flex items-center gap-[5px] text-[10.5px] text-body">
        <span className={`inline-block h-2 w-2 rounded-full ${dotBgClass[stage.dot]}`} />
        {stage.health}
      </div>
      {!isLast && (
        <span
          className={`absolute -right-2 top-1/2 z-[1] -translate-y-1/2 bg-canvas px-[1px] text-[12px] ${stage.arrowPrimary ? 'text-primary' : 'text-muted-soft'}`}
        >
          ›
        </span>
      )}
    </div>
  );
}

function RecentRow({ row }: { row: DeployRow }) {
  const dest = row.pillKind === 'gate' ? '/runs/r-9143/gate' : '/runs/r-9143';
  return (
    <div
      className="grid cursor-pointer grid-cols-[60px_1fr_70px_60px] items-center gap-2 border-b border-dashed border-hairline-soft py-[5px] text-[11.5px] last:border-b-0 hover:bg-surface-soft/60"
      onClick={() => navigate(dest)}
    >
      <span className="font-mono text-[10px] uppercase tracking-[1px] text-muted">{row.env}</span>
      <span className="text-body">{row.message}</span>
      <Pill kind={row.pillKind}>
        <PillDot kind={row.pillKind} />
        {row.pillText}
      </Pill>
      <span className="text-right font-mono text-[10.5px] text-muted-soft">{row.time}</span>
    </div>
  );
}

function DepCard({ card }: { card: DepCardData }) {
  return (
    <div className="flex min-h-0 flex-col rounded-[10px] border border-hairline bg-canvas p-[14px_16px]">
      <div className="mb-3 flex items-center gap-[10px]">
        <span className="font-serif text-[18px] font-medium tracking-[-0.2px] text-ink">
          {card.name}
        </span>
        <span className="font-mono text-[11px] text-muted">{card.repo}</span>
        <span className="ml-auto">
          <Pill kind={card.statusKind}>
            <PillDot kind={card.statusKind} />
            {card.statusText}
          </Pill>
        </span>
      </div>

      <div className="mb-[14px] grid grid-cols-4 gap-[10px]">
        {card.stages.map((stage, i) => (
          <Stage key={stage.name} stage={stage} isLast={i === card.stages.length - 1} />
        ))}
      </div>

      <div className="text-[11.5px] text-body">
        <div className="mb-[6px] text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
          Recent deploys
        </div>
        {card.recentDeploys.map((row, i) => (
          <RecentRow key={i} row={row} />
        ))}
      </div>

      {card.gateResume && (
        <div className="mt-auto flex items-center gap-2 rounded-[8px] border border-[rgba(204,120,92,0.3)] bg-[rgba(204,120,92,0.08)] p-[8px_12px] pt-[14px] text-[11.5px]">
          <div className="flex-1 text-body">
            <b className="text-primary-active">{card.gateResume.policy}</b>
            {' · '}
            {card.gateResume.description}
          </div>
          <Btn variant="primary" size="sm">Resume</Btn>
        </div>
      )}
    </div>
  );
}

// ─── Rail items ───────────────────────────────────────────────────────────────

const deployRailItems = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'projects',
}));

// ─── Page ─────────────────────────────────────────────────────────────────────

export function DeploysPage() {
  return (
    <AppShell rail={<Rail items={deployRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Deploys']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Deploys{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="default">4 projects</Pill>
                  <Pill kind="gate">1 paused at gate</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Pipelines across the gateway-managed projects ·{' '}
                <strong className="font-semibold text-body-strong">27 deploys today</strong>
                {' '}· SLO budget healthy
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Compare versions</Btn>
              <Btn variant="primary">+ New pipeline</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="grid flex-1 grid-cols-2 gap-4 overflow-hidden">
            {DEP_CARDS.map((card) => (
              <DepCard key={card.name} card={card} />
            ))}
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
