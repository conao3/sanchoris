import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, Tag, type PillKind, type TagTone } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

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
  navigateTo: string;
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
    navigateTo: '/runs/r-9143',
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
    navigateTo: '/runs/r-9143',
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
    navigateTo: '/runs/r-9143',
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
    navigateTo: '/runs/r-9143',
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
    navigateTo: '/pull-requests',
  },
  { id: '', runId: '', pill: { kind: 'default', label: '' }, title: '', desc: '', actions: [], hidden: true, navigateTo: '' },
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
    navigateTo: '/gates',
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
    navigateTo: '/runs/r-9143',
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
      className={`rounded-[10px] border p-[14px_16px] cursor-pointer ${
        card.urgent
          ? 'border-[rgba(204,120,92,0.4)] bg-[rgba(204,120,92,0.03)]'
          : 'border-hairline bg-canvas'
      }`}
      onClick={() => navigate(card.navigateTo)}
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
