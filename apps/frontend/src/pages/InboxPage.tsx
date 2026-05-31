import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, Tag, type PillKind, type TagTone } from '../components/shell/primitives';

// ─── Rail items ───────────────────────────────────────────────────────────────

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

const WORKSPACE_ITEMS: RailItemDef[] = [
  { key: 'space', icon: <SpaceIcon />, label: 'Space' },
  { key: 'projects', icon: <ProjectsIcon />, label: 'Projects', badge: '7' },
  { key: 'queue', icon: <QueueIcon />, label: 'Queue', badge: '28', badgeTone: 'coral' },
  { key: 'prs', icon: <PRsIcon />, label: 'PRs', badge: '12' },
  { key: 'workflows', icon: <WorkflowsIcon />, label: 'Workflows' },
  { key: 'workers', icon: <WorkersIcon />, label: 'Workers' },
  { key: 'memory', icon: <MemoryIcon />, label: 'Memory' },
  { key: 'channels', icon: <ChannelsIcon />, label: 'Channels' },
];

const ACCOUNT_ITEMS: RailItemDef[] = [
  { key: 'settings', icon: <SettingsIcon />, label: 'Settings' },
];

// ─── HR card types ─────────────────────────────────────────────────────────────

type HRCard = {
  id: string;
  runId: string;
  pill: { kind: PillKind; label: string };
  tag?: { tone: TagTone; label: string };
  title: string;
  desc: string;
  prop?: string;
  actions: Array<{ variant: 'primary' | 'secondary' | 'ghost'; label: string }>;
  urgent?: boolean;
  hidden?: boolean;
};

type HRGroup = {
  label: string;
  count: string;
  cards: HRCard[];
};

// ─── Data ──────────────────────────────────────────────────────────────────────

const nowCards: HRCard[] = [
  {
    id: 'CON-1241',
    runId: 'R-9077',
    pill: { kind: 'failed', label: 'blocked 4h 12m' },
    tag: { tone: 'coral-soft', label: 'claude-code paused' },
    title: 'Choose canonical schema for orders_v2 migration',
    desc: 'Two conflicting field shapes detected during normalize. Agent surfaces them as {customer_id:int} (legacy) vs {customerId:uuid} (new). Pick canonical before retry.',
    prop: 'Proposed: adopt new shape · codemod 142 callsites · one-shot migration window 12 m',
    actions: [
      { variant: 'primary', label: 'Resolve · pick new shape' },
      { variant: 'secondary', label: 'Reassign' },
      { variant: 'ghost', label: 'Open task ↗' },
    ],
    urgent: true,
  },
  {
    id: 'CON-1255',
    runId: 'R-9136',
    pill: { kind: 'failed', label: 'blocked 1h 04m' },
    tag: { tone: 'amber', label: 'secret rotation' },
    title: 'Confirm new Stripe API key issued by owner',
    desc: 'Worker found expired secret in stripe-bridge/.env · owner kanagawa-ops rotated 13:08 JST. Acknowledge so run can re-fetch from vault.',
    prop: 'Proposed: re-pull stripe.live from vault · redeploy stripe-bridge to staging first',
    actions: [
      { variant: 'primary', label: 'Acknowledge' },
      { variant: 'secondary', label: 'Re-rotate' },
      { variant: 'ghost', label: 'Open task ↗' },
    ],
    urgent: true,
  },
];

const todayCards: HRCard[] = [
  {
    id: 'CON-1252',
    runId: 'R-9120',
    pill: { kind: 'warn', label: 'awaiting 38m' },
    title: 'Acceptance unclear for "make checkout faster"',
    desc: 'Pick a baseline metric and target. Current p95 = 1.18s across desktop + mobile.',
    prop: 'Proposed: target p95 ≤ 800 ms desktop · budget 2 weeks',
    actions: [
      { variant: 'primary', label: 'Accept proposal' },
      { variant: 'secondary', label: 'Edit criteria' },
    ],
  },
  {
    id: 'CON-1250',
    runId: 'R-9145',
    pill: { kind: 'warn', label: 'awaiting 22m' },
    title: 'Confirm Linear rollup — 3 tickets are duplicates of ENG-1182',
    desc: 'CON-1250, ENG-1184, ENG-1185 all describe bulk CSV import. Agent wants to deduplicate and continue under one parent.',
    prop: 'Proposed: rollup under CON-1250 · close ENG-1184/1185 as dup',
    actions: [
      { variant: 'primary', label: 'Confirm rollup' },
      { variant: 'secondary', label: 'Keep separate' },
    ],
  },
  {
    id: 'CON-1253',
    runId: 'R-9128',
    pill: { kind: 'warn', label: 'awaiting 17m' },
    title: 'Approve copy change to triage-worker refusal path',
    desc: '+3 lines · tone shift from formal to plain. No behaviour change expected.',
    prop: 'Proposed: apply as-is · rollout via percentile 10% → 100%',
    actions: [
      { variant: 'primary', label: 'Approve' },
      { variant: 'secondary', label: 'View diff' },
    ],
  },
  { id: '', runId: '', pill: { kind: 'default', label: '' }, title: '', desc: '', actions: [], hidden: true },
];

const thisWeekCards: HRCard[] = [
  {
    id: 'CON-1230',
    runId: 'R-9012',
    pill: { kind: 'done', label: 'queued 2d' },
    title: 'Review proposed workflow change for delivery/default',
    desc: 'Adds a policy-review step before Gate. Tagged for owner approval.',
    actions: [
      { variant: 'secondary', label: 'Open diff' },
      { variant: 'ghost', label: 'Reassign' },
    ],
  },
  {
    id: 'CON-1225',
    runId: 'R-8998',
    pill: { kind: 'done', label: 'queued 3d' },
    title: 'Confirm dependency upgrade plan for Q3',
    desc: 'Agent drafted 9-step bump (TypeScript, undici, vitest). Awaiting owner direction.',
    actions: [
      { variant: 'secondary', label: 'Read plan' },
      { variant: 'ghost', label: 'Reassign' },
    ],
  },
];

const HR_GROUPS: HRGroup[] = [
  { label: 'Now', count: '2 · blocking active runs', cards: nowCards },
  { label: 'Today', count: '3 · loose acceptance', cards: todayCards },
  { label: 'This week', count: '2 · non-blocking', cards: thisWeekCards },
];

// ─── HR card component ─────────────────────────────────────────────────────────

function HRCardView({ card }: { card: HRCard }) {
  if (card.hidden) {
    return <div className="invisible" aria-hidden="true" />;
  }
  return (
    <article
      className={`rounded-[10px] border p-[14px_16px] ${
        card.urgent
          ? 'border-[rgba(204,120,92,0.4)] bg-[rgba(204,120,92,0.03)]'
          : 'border-hairline bg-canvas'
      }`}
    >
      <div className="mb-[6px] flex flex-wrap items-center gap-2">
        <span className="font-mono text-[11px] text-muted">
          {card.id} · {card.runId}
        </span>
        <Pill kind={card.pill.kind}>{card.pill.label}</Pill>
        {card.tag && (
          <Tag tone={card.tag.tone} className="ml-auto">
            {card.tag.label}
          </Tag>
        )}
      </div>
      <h3 className="mb-[6px] text-[14px] font-medium leading-[1.35] text-ink">{card.title}</h3>
      <p className="mb-[10px] text-[12px] leading-[1.5] text-muted">{card.desc}</p>
      {card.prop && (
        <div className="mb-[10px] rounded-[6px] border border-hairline-soft bg-surface-soft px-[10px] py-[7px] font-mono text-[11.5px] text-body">
          <strong className="font-semibold text-ink">Proposed:</strong>{' '}
          {card.prop.replace(/^Proposed: /, '')}
        </div>
      )}
      <div className="flex items-center gap-[6px]">
        {card.actions.map((action, i) => (
          <Btn key={i} variant={action.variant} size="sm">
            {action.label}
          </Btn>
        ))}
      </div>
    </article>
  );
}

function HRGroupView({ group }: { group: HRGroup }) {
  return (
    <div>
      <div className="mb-2 flex items-baseline gap-[10px]">
        <span className="text-[11px] font-semibold uppercase tracking-[1.4px] text-muted">
          {group.label}
        </span>
        <span className="font-mono text-[12px] text-muted-soft">{group.count}</span>
      </div>
      <div className="grid grid-cols-2 gap-3">
        {group.cards.map((card, i) => (
          <HRCardView key={card.id || i} card={card} />
        ))}
      </div>
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

const inboxRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((item) => ({
  ...item,
  active: item.key === 'queue',
}));

export function InboxPage() {
  return (
    <AppShell
      rail={
        <Rail items={inboxRailItems} bottomItems={ACCOUNT_ITEMS} />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Queue', 'Human review']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Human review inbox{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="gate">7 awaiting</Pill>
                  <Pill kind="failed">2 blocked &gt; 4h</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Tasks paused on questions only a human can answer · assigned to you ·{' '}
                <strong className="font-semibold text-body-strong">you owe 4</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Snooze all</Btn>
              <Btn variant="primary">Open command palette · ⌘K</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex flex-1 flex-col gap-[14px] overflow-hidden">
            {HR_GROUPS.map((group) => (
              <HRGroupView key={group.label} group={group} />
            ))}
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
