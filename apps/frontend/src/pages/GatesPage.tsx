import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Rail ──────────────────────────────────────────────────────────────────────

const gateRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'queue',
}));

// ─── Kind badge ────────────────────────────────────────────────────────────────

type GateKind = 'merge' | 'deploy' | 'secret' | 'high';

const kindClass: Record<GateKind, string> = {
  merge: 'bg-[rgba(204,120,92,0.16)] text-primary-active',
  deploy: 'bg-[rgba(232,165,90,0.22)] text-[#7a4d10]',
  secret: 'bg-[rgba(198,69,69,0.16)] text-[#862e2e]',
  high: 'bg-surface-dark text-on-dark',
};

const kindLabel: Record<GateKind, string> = {
  merge: 'MERGE',
  deploy: 'DEPLOY',
  secret: 'SECRET ROTATION',
  high: 'HIGH-RISK PATH',
};

function KindBadge({ kind }: { kind: GateKind }) {
  return (
    <span
      className={`rounded-[4px] px-[8px] py-[3px] text-[10.5px] font-bold uppercase tracking-[1.3px] ${kindClass[kind]}`}
    >
      {kindLabel[kind]}
    </span>
  );
}

// ─── Diff block ────────────────────────────────────────────────────────────────

type DiffLine =
  | { type: 'add'; text: string }
  | { type: 'del'; text: string }
  | { type: 'ln'; text: string }
  | { type: 'hd'; text: string }
  | { type: 'kv'; key: string; value: string; valueType: 'add' | 'del' | 'ln' };

function DiffBlock({ lines }: { lines: DiffLine[] }) {
  return (
    <div className="my-2 max-h-[110px] overflow-hidden rounded-[6px] bg-surface-dark p-[8px_10px] font-mono text-[10.5px] leading-[1.5] text-on-dark-soft">
      {lines.map((line, i) => {
        if (line.type === 'hd') {
          return (
            <div key={i} className="text-[10px] text-muted-soft">
              {line.text}
            </div>
          );
        }
        if (line.type === 'add') {
          return (
            <div key={i}>
              <span className="text-[#7fcc8f]">+ {line.text}</span>
            </div>
          );
        }
        if (line.type === 'del') {
          return (
            <div key={i}>
              <span className="text-[#e58176]">- {line.text}</span>
            </div>
          );
        }
        if (line.type === 'ln') {
          return (
            <div key={i} className="text-on-dark">
              {line.text}
            </div>
          );
        }
        if (line.type === 'kv') {
          const valueClass =
            line.valueType === 'add'
              ? 'text-[#7fcc8f]'
              : line.valueType === 'del'
                ? 'text-[#e58176]'
                : 'text-on-dark';
          return (
            <div key={i}>
              <span className="text-on-dark-soft">{line.key}</span>{' '}
              <span className={valueClass}>{line.value}</span>
            </div>
          );
        }
        return null;
      })}
    </div>
  );
}

// ─── Gate card ─────────────────────────────────────────────────────────────────

type GateAction = {
  variant: 'primary' | 'secondary' | 'danger';
  label: string;
};

type GateCardData = {
  kind: GateKind;
  meta: string;
  age: string;
  urgent?: boolean;
  title: string;
  repo: ReactNode;
  diff: DiffLine[];
  policy: ReactNode;
  actions: GateAction[];
};

function GateCard({ card }: { card: GateCardData }) {
  return (
    <article className="rounded-[10px] border border-hairline bg-canvas p-[14px_16px]">
      <div className="mb-[5px] flex items-center gap-2">
        <KindBadge kind={card.kind} />
        <span className="text-[11px] text-muted">{card.meta}</span>
        <span
          className={`ml-auto font-mono text-[10.5px] ${
            card.urgent
              ? 'rounded-full bg-[rgba(204,120,92,0.12)] px-[7px] py-[2px] text-primary'
              : 'text-muted-soft'
          }`}
        >
          {card.age}
        </span>
      </div>
      <div className="mb-[6px] text-[13.5px] font-medium leading-[1.35] text-ink">
        {card.title}
        <span className="ml-[6px] font-mono text-[11px] font-normal text-muted">{card.repo}</span>
      </div>
      <DiffBlock lines={card.diff} />
      <div className="mb-[10px] text-[11px] leading-[1.5] text-muted">{card.policy}</div>
      <div className="flex flex-wrap items-center gap-[6px]">
        {card.actions.map((action, i) => (
          <Btn key={i} variant={action.variant} size="sm">
            {action.label}
          </Btn>
        ))}
      </div>
    </article>
  );
}

// ─── Gate cards data ────────────────────────────────────────────────────────────

const GATE_CARDS: GateCardData[] = [
  {
    kind: 'merge',
    meta: 'PR #482 · acme-platform',
    age: 'awaiting 22 m',
    urgent: true,
    title: 'Merge OAuth login for vendor portal',
    repo: 'main ← issue-con-1247',
    diff: [
      { type: 'hd', text: 'apps/auth/oauth.ts' },
      { type: 'del', text: 'export function loginWithSession(jwt: string) {' },
      { type: 'add', text: 'export function loginWithSession(jwt: string, opts?: LoginOpts) {' },
      { type: 'add', text: '  const skewMs = opts?.allowSkewMs ?? 90_000;' },
      { type: 'ln', text: '    const exp = decodeJwt(jwt).exp * 1000;' },
      { type: 'del', text: '  if (Date.now() > exp) return redirect("/login");' },
      { type: 'add', text: '  if (Date.now() > exp + skewMs) return redirect("/login");' },
    ],
    policy: (
      <>
        Policy <strong className="font-semibold text-body-strong">prod-touching-files</strong> matched{' '}
        <span className="font-mono">apps/auth/**</span>. Two reviewers required ·{' '}
        <strong className="font-semibold text-body-strong">1 of 2</strong> approved (
        <strong className="font-semibold text-body-strong">octocat</strong>). Risk:{' '}
        <strong className="font-semibold" style={{ color: 'var(--primary-active)' }}>
          medium
        </strong>
        .
      </>
    ),
    actions: [
      { variant: 'primary', label: 'Approve & continue' },
      { variant: 'secondary', label: 'Defer 1h' },
      { variant: 'danger', label: 'Reject with note' },
    ],
  },
  {
    kind: 'deploy',
    meta: 'stripe-bridge v0.18.7 → v0.19.0',
    age: 'awaiting 1 h 08 m',
    title: 'Promote stripe-bridge to prod',
    repo: 'staging → prod',
    diff: [
      { type: 'hd', text: 'CHANGELOG (excerpt)' },
      { type: 'ln', text: '## 0.19.0 · 2026-05-24' },
      { type: 'ln', text: '- feat: webhook retry / exponential backoff' },
      { type: 'ln', text: '- feat: env var STRIPE_WEBHOOK_V2_SECRET' },
      { type: 'ln', text: '- fix: drop legacy /webhook/v1 path' },
      { type: 'ln', text: '- chore: bump undici 6.18 → 6.19 (CVE-2026-1142)' },
      { type: 'ln', text: '- 6 commits · 4 contributors' },
    ],
    policy: (
      <>
        Policy <strong className="font-semibold text-body-strong">prod-deploy-business-hours</strong> matched. Now{' '}
        <strong className="font-semibold text-body-strong">23:14 JST</strong> — outside window (09:00–19:00 JST). Risk:{' '}
        <strong className="font-semibold" style={{ color: '#7a4d10' }}>
          low
        </strong>
        .
      </>
    ),
    actions: [
      { variant: 'primary', label: 'Override & deploy' },
      { variant: 'secondary', label: 'Defer to 09:00' },
      { variant: 'danger', label: 'Reject' },
    ],
  },
  {
    kind: 'secret',
    meta: 'vault://stripe.live',
    age: 'awaiting 3 h',
    title: 'Rotate STRIPE_LIVE_KEY after expiry',
    repo: 'all envs',
    diff: [
      { type: 'hd', text: 'vault rotation plan' },
      { type: 'kv', key: 'resource', value: 'stripe.live', valueType: 'ln' },
      { type: 'kv', key: 'issuer  ', value: 'stripe.com / acme-prod-team', valueType: 'ln' },
      { type: 'kv', key: 'old.ts  ', value: '2025-11-20T00:00Z (expired 2026-05-22)', valueType: 'del' },
      { type: 'kv', key: 'new.ts  ', value: '2026-05-24T13:08Z (acknowledged by kanagawa-ops)', valueType: 'add' },
      { type: 'kv', key: 'callers ', value: 'stripe-bridge x4 · web x1', valueType: 'ln' },
    ],
    policy: (
      <>
        Policy <strong className="font-semibold text-body-strong">secret-touching-prod</strong> matched. Owner ack
        present. Risk:{' '}
        <strong className="font-semibold" style={{ color: '#862e2e' }}>
          high
        </strong>{' '}
        (touches live payments).
      </>
    ),
    actions: [
      { variant: 'primary', label: 'Approve rotation' },
      { variant: 'secondary', label: 'Inspect callers' },
      { variant: 'danger', label: 'Halt' },
    ],
  },
  {
    kind: 'high',
    meta: 'apps/billing/**',
    age: 'awaiting 4 h 22 m',
    title: 'Touching billing core: refund logic refactor',
    repo: 'acme-platform · PR #476',
    diff: [
      { type: 'hd', text: 'apps/billing/refund.ts' },
      { type: 'ln', text: '@@ -42,9 +42,15 @@' },
      { type: 'del', text: 'async function refund(intentId: string) {' },
      { type: 'add', text: 'async function refund(intentId: string, reason: RefundReason) {' },
      { type: 'add', text: '  audit.log("refund.start", { intentId, reason });' },
      { type: 'ln', text: '    const result = await stripe.refunds.create({...});' },
      { type: 'add', text: '  audit.log("refund.ok", { id: result.id, intentId });' },
    ],
    policy: (
      <>
        Policy <strong className="font-semibold text-body-strong">billing-touch</strong> matched. Requires SOC2
        reviewer + on-call ack. Risk:{' '}
        <strong className="font-semibold" style={{ color: '#862e2e' }}>
          high
        </strong>
        .
      </>
    ),
    actions: [
      { variant: 'primary', label: 'Approve' },
      { variant: 'secondary', label: 'Request changes' },
      { variant: 'danger', label: 'Reject' },
    ],
  },
];

// ─── Page ──────────────────────────────────────────────────────────────────────

export function GatesPage() {
  return (
    <AppShell rail={<Rail items={gateRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Queue', 'Gate approvals']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Gate approvals{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="gate">4 pending</Pill>
                  <Pill kind="running">2 you own</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Merges, deploys, secret rotations &amp; high-risk paths held by policy ·{' '}
                <strong className="font-semibold text-body-strong">workflow delivery/default v0.3.2</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">View policy</Btn>
              <Btn variant="primary">Bulk approve (2)</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div
            className="grid flex-1 content-start gap-[14px] overflow-hidden"
            style={{ gridTemplateColumns: '1fr 1fr' }}
          >
            {GATE_CARDS.map((card, i) => (
              <GateCard key={i} card={card} />
            ))}
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
