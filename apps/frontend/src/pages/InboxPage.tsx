import type { ReactNode } from 'react';

export function InboxPage() {
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-canvas font-sans text-ink">
      <Topbar />
      <div className="flex min-h-0 flex-1">
        <Rail />
        <Screen />
      </div>
    </div>
  );
}

function Topbar() {
  return (
    <header className="flex h-14 flex-none items-center gap-4 border-b border-hairline bg-canvas px-5">
      <Wordmark />
      <span className="h-[22px] w-px bg-hairline" aria-hidden="true" />
      <WorkspaceSwitcher />
      <div className="flex flex-1 justify-center">
        <SearchBar />
      </div>
      <ModelBadge />
      <TopbarIcon symbol="🔔" badge={3} ariaLabel="Notifications" />
      <TopbarIcon symbol="⚙" ariaLabel="Settings" />
      <Avatar variant="me" initials="AT" />
    </header>
  );
}

function Wordmark() {
  return (
    <div className="flex items-center gap-2 font-serif text-[22px] font-medium leading-none tracking-[-0.4px] text-ink">
      <span className="relative inline-block size-4" aria-hidden="true">
        <span className="absolute inset-0 rounded-[2px] border-[1.4px] border-ink" />
        <span className="absolute left-0 top-0 size-[7px] bg-primary" />
      </span>
      sanchoris<span className="text-primary">.</span>
    </div>
  );
}

function WorkspaceSwitcher() {
  return (
    <button
      type="button"
      className="inline-flex items-center gap-1.5 rounded-sm px-2 py-1.5 text-sm text-body hover:bg-surface-soft"
    >
      <span className="inline-flex size-[18px] items-center justify-center rounded-xs bg-surface-dark text-[10px] font-bold text-on-dark">
        A
      </span>
      <span className="font-medium text-ink">acme-org</span>
      <span className="font-light text-muted-soft">/</span>
      <span className="inline-flex size-[18px] items-center justify-center rounded-xs bg-primary text-[10px] font-bold text-on-primary">
        P
      </span>
      <span className="font-semibold text-ink">acme-platform</span>
    </button>
  );
}

function SearchBar() {
  return (
    <div className="flex h-[34px] w-[440px] items-center gap-2.5 rounded-sm border border-hairline bg-surface-soft px-3 text-sm text-muted">
      <span aria-hidden="true">⌕</span>
      <span className="flex-1">Search projects, runs, gates…</span>
      <Kbd>⌘K</Kbd>
    </div>
  );
}

function ModelBadge() {
  return (
    <span className="inline-flex h-7 items-center gap-1.5 rounded-sm border border-hairline bg-surface-soft px-2.5 font-mono text-sm text-body-strong">
      <span className="size-1.5 rounded-full bg-accent-teal" aria-hidden="true" />
      sonnet-4.5
    </span>
  );
}

function TopbarIcon({ symbol, badge, ariaLabel }: { symbol: string; badge?: number; ariaLabel: string }) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      className="relative inline-flex size-8 items-center justify-center rounded-sm text-sm text-muted hover:bg-surface-soft"
    >
      <span aria-hidden="true">{symbol}</span>
      {badge !== undefined ? (
        <span className="absolute right-1 top-1 inline-flex h-[13px] min-w-[13px] items-center justify-center rounded-full bg-primary px-[3px] text-[9px] font-bold leading-none text-on-primary">
          {badge}
        </span>
      ) : null}
    </button>
  );
}

type AvatarVariant = 'me' | 'coral' | 'teal' | 'navy' | 'amber' | 'default';

function Avatar({ variant, initials }: { variant: AvatarVariant; initials: string }) {
  const styleMap: Record<AvatarVariant, string> = {
    me: 'bg-ink text-canvas',
    coral: 'bg-primary text-on-primary',
    teal: 'bg-accent-teal text-on-primary',
    navy: 'bg-surface-dark text-on-dark',
    amber: 'bg-accent-amber text-[#4a3306]',
    default: 'bg-surface-card text-ink',
  };
  return (
    <span
      className={`inline-flex size-7 flex-none items-center justify-center rounded-full text-[11px] font-semibold ${styleMap[variant]}`}
    >
      {initials}
    </span>
  );
}

function Rail() {
  return (
    <aside className="flex w-56 flex-none flex-col gap-0.5 border-r border-hairline bg-surface-soft px-3 pb-3.5 pt-4">
      <RailSection label="Workspace" />
      <RailItem icon="📥" label="Inbox" badge="9" tone="coral" active />
      <RailItem icon="📦" label="Projects" badge="5" />
      <RailItem icon="🚦" label="Gates" badge="3" />
      <RailItem icon="🏃" label="Runs" badge="2" />
      <RailSection label="Account" />
      <RailItem icon="⚙" label="Settings" />
      <div className="flex-1" />
      <RailHelp />
    </aside>
  );
}

function RailSection({ label }: { label: string }) {
  return (
    <div className="px-3 pb-1.5 pt-3.5 text-[10.5px] font-semibold uppercase tracking-[1.5px] text-muted-soft">
      {label}
    </div>
  );
}

function RailItem({
  icon,
  label,
  badge,
  tone = 'default',
  active = false,
}: {
  icon: string;
  label: string;
  badge?: string;
  tone?: 'default' | 'coral';
  active?: boolean;
}) {
  return (
    <a
      href="#"
      className={`flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-sm no-underline ${
        active ? 'bg-surface-cream-strong font-semibold text-ink' : 'text-body hover:bg-surface-cream-strong/60'
      }`}
    >
      <span
        aria-hidden="true"
        className={`flex size-4 flex-none items-center justify-center text-sm ${active ? 'text-primary' : 'text-muted'}`}
      >
        {icon}
      </span>
      <span className="flex-1">{label}</span>
      {badge !== undefined ? (
        <span
          className={`rounded-pill border px-1.5 font-mono text-[10.5px] font-medium leading-snug ${
            tone === 'coral' ? 'border-primary bg-primary text-on-primary' : 'border-hairline bg-canvas text-muted'
          }`}
        >
          {badge}
        </span>
      ) : null}
    </a>
  );
}

function RailHelp() {
  return (
    <div className="mt-2 flex flex-col gap-1 border-t border-hairline px-2.5 pb-2 pt-2 text-xs text-muted">
      <div className="flex items-center justify-between gap-1.5">
        <span>command palette</span>
        <Kbd>⌘K</Kbd>
      </div>
      <div className="flex items-center justify-between gap-1.5">
        <span>quick switch</span>
        <Kbd>⌘P</Kbd>
      </div>
    </div>
  );
}

function Kbd({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-0.5 rounded-xs border border-b-2 border-hairline bg-canvas px-1.5 py-[1.5px] font-mono text-[11px] leading-none text-muted">
      {children}
    </span>
  );
}

function KbdInverse({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-0.5 rounded-xs border border-b-2 border-white/30 bg-white/20 px-1.5 py-[1.5px] font-mono text-[11px] leading-none text-on-primary">
      {children}
    </span>
  );
}

function Screen() {
  return (
    <main className="flex min-w-0 flex-1 flex-col overflow-hidden bg-canvas">
      <ScreenHead />
      <ScreenBody />
    </main>
  );
}

function ScreenHead() {
  return (
    <header className="flex-none border-b border-hairline px-7 pb-3.5 pt-5">
      <Crumbs items={['acme-org', 'Queue', 'Human review']} />
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
            Human review inbox
            <span className="ml-3 inline-flex gap-2 align-middle">
              <Pill kind="gate">7 awaiting</Pill>
              <Pill kind="failed">2 blocked &gt; 4h</Pill>
            </span>
          </h1>
          <p className="mt-1.5 text-sm leading-[1.5] text-muted">
            Tasks paused on questions only a human can answer · assigned to you ·{' '}
            <b className="font-semibold text-body-strong">you owe 4</b>
          </p>
        </div>
        <div className="flex flex-none items-center gap-2 pt-1">
          <button
            type="button"
            className="inline-flex h-[34px] items-center rounded-sm border border-hairline bg-canvas px-3.5 text-base font-medium text-ink hover:border-primary/40"
          >
            Snooze all
          </button>
          <button
            type="button"
            className="inline-flex h-[34px] items-center gap-1.5 rounded-sm border border-primary bg-primary px-3.5 text-base font-medium text-on-primary hover:bg-primary-active"
          >
            Open command palette ·<KbdInverse>⌘K</KbdInverse>
          </button>
        </div>
      </div>
    </header>
  );
}

function Crumbs({ items }: { items: string[] }) {
  return (
    <div className="mb-2 flex items-center gap-[7px] text-sm text-muted">
      {items.map((c, i) => (
        <span key={c} className="flex items-center gap-[7px]">
          {i > 0 ? <span className="text-[10px] text-muted-soft">/</span> : null}
          <span className={i === items.length - 1 ? 'font-medium text-ink' : ''}>{c}</span>
        </span>
      ))}
    </div>
  );
}

function ScreenBody() {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-7 pb-6 pt-4">
      <div className="flex min-h-0 flex-1 flex-col gap-3.5 overflow-hidden">
        <HRGroup label="Now" count="2 · blocking active runs">
          {nowCards}
        </HRGroup>
        <HRGroup label="Today" count="3 · loose acceptance">
          {todayCards}
        </HRGroup>
        <HRGroup label="This week" count="2 · non-blocking">
          {thisWeekCards}
        </HRGroup>
      </div>
    </div>
  );
}

function HRGroup({ label, count, children }: { label: string; count: string; children: ReactNode }) {
  return (
    <section>
      <header className="mb-2 flex items-baseline gap-2.5">
        <span className="text-xs font-semibold uppercase tracking-[1.4px] text-muted">{label}</span>
        <span className="font-mono text-sm text-muted-soft">{count}</span>
      </header>
      <div className="grid grid-cols-2 gap-3">{children}</div>
    </section>
  );
}

type HRCardAction = { label: ReactNode; kind: 'primary' | 'secondary' | 'ghost' };

type HRCardProps = {
  id: string;
  pill: { label: string; kind: 'failed' | 'warn' | 'done' };
  tag?: { label: string; kind: 'coral-soft' | 'amber' };
  title: ReactNode;
  desc: ReactNode;
  prop?: ReactNode;
  actions: HRCardAction[];
  urgent?: boolean;
};

function HRCard({ id, pill, tag, title, desc, prop, actions, urgent = false }: HRCardProps) {
  return (
    <article
      className={`rounded-input border px-4 py-3.5 ${
        urgent ? 'border-primary/40 bg-primary/[0.03]' : 'border-hairline bg-canvas'
      }`}
    >
      <header className="mb-1.5 flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs text-muted">{id}</span>
        <Pill kind={pill.kind}>{pill.label}</Pill>
        {tag ? <Tag kind={tag.kind}>{tag.label}</Tag> : null}
      </header>
      <h3 className="mb-1.5 text-base font-medium leading-[1.35] text-ink">{title}</h3>
      <p className="mb-2.5 text-sm leading-[1.5] text-muted">{desc}</p>
      {prop ? (
        <div className="mb-2.5 rounded-sm border border-hairline-soft bg-surface-soft px-2.5 py-[7px] font-mono text-[11.5px] text-body">
          {prop}
        </div>
      ) : null}
      <footer className="flex items-center gap-1.5">
        {actions.map((a, i) => (
          <Btn key={i} kind={a.kind}>
            {a.label}
          </Btn>
        ))}
      </footer>
    </article>
  );
}

type PillKind = 'gate' | 'failed' | 'warn' | 'done' | 'running';

function Pill({ kind, children }: { kind: PillKind; children: ReactNode }) {
  const bgMap: Record<PillKind, string> = {
    gate: 'bg-accent-amber text-[#4a3306]',
    failed: 'bg-error text-on-primary',
    warn: 'bg-[rgba(212,160,23,0.16)] text-[#856200]',
    done: 'bg-surface-card text-muted',
    running: 'bg-primary text-on-primary',
  };
  return (
    <span
      className={`inline-flex items-center gap-1.5 whitespace-nowrap rounded-pill px-2.5 py-[3px] text-[11.5px] font-medium leading-[1.4] ${bgMap[kind]}`}
    >
      {kind === 'done' ? <span aria-hidden="true" className="size-1.5 rounded-full bg-muted-soft" /> : null}
      {children}
    </span>
  );
}

function Tag({ kind, children }: { kind: 'coral-soft' | 'amber'; children: ReactNode }) {
  const bgMap = {
    'coral-soft': 'bg-[rgba(204,120,92,0.16)] text-primary-active',
    amber: 'bg-[rgba(232,165,90,0.22)] text-[#7a4d10]',
  };
  return (
    <span className={`ml-auto inline-flex rounded-xs px-1.5 py-0.5 font-mono text-[11px] leading-[1.4] ${bgMap[kind]}`}>
      {children}
    </span>
  );
}

function Btn({ kind, children }: { kind: 'primary' | 'secondary' | 'ghost'; children: ReactNode }) {
  const cls = {
    primary: 'border-primary bg-primary text-on-primary hover:bg-primary-active',
    secondary: 'border-hairline bg-canvas text-ink hover:border-primary/40',
    ghost: 'border-transparent bg-transparent text-body hover:bg-surface-soft',
  }[kind];
  return (
    <button
      type="button"
      className={`inline-flex h-7 items-center gap-1.5 rounded-sm border px-2.5 text-sm font-medium leading-none ${cls}`}
    >
      {children}
    </button>
  );
}

function Mono({ children }: { children: ReactNode }) {
  return <span className="font-mono">{children}</span>;
}

const nowCards = (
  <>
    <HRCard
      urgent
      id="CON-1241 · R-9077"
      pill={{ label: 'blocked 4h 12m', kind: 'failed' }}
      tag={{ label: 'claude-code paused', kind: 'coral-soft' }}
      title={
        <>
          Choose canonical schema for <Mono>orders_v2</Mono> migration
        </>
      }
      desc={
        <>
          Two conflicting field shapes detected during <b className="font-semibold text-body-strong">normalize</b>.
          Agent surfaces them as <Mono>{'{customer_id:int}'}</Mono> (legacy) vs <Mono>{'{customerId:uuid}'}</Mono>{' '}
          (new). Pick canonical before retry.
        </>
      }
      prop={
        <>
          <b className="font-semibold text-ink">Proposed:</b> adopt new shape · codemod 142 callsites · one-shot
          migration window 12 m
        </>
      }
      actions={[
        { label: 'Resolve · pick new shape', kind: 'primary' },
        { label: 'Reassign', kind: 'secondary' },
        { label: 'Open task ↗', kind: 'ghost' },
      ]}
    />
    <HRCard
      urgent
      id="CON-1255 · R-9136"
      pill={{ label: 'blocked 1h 04m', kind: 'failed' }}
      tag={{ label: 'secret rotation', kind: 'amber' }}
      title="Confirm new Stripe API key issued by owner"
      desc={
        <>
          Worker found expired secret in <Mono>stripe-bridge/.env</Mono> · owner{' '}
          <b className="font-semibold text-body-strong">kanagawa-ops</b> rotated 13:08 JST. Acknowledge so run can
          re-fetch from vault.
        </>
      }
      prop={
        <>
          <b className="font-semibold text-ink">Proposed:</b> re-pull <Mono>stripe.live</Mono> from vault · redeploy
          stripe-bridge to staging first
        </>
      }
      actions={[
        { label: 'Acknowledge', kind: 'primary' },
        { label: 'Re-rotate', kind: 'secondary' },
        { label: 'Open task ↗', kind: 'ghost' },
      ]}
    />
  </>
);

const todayCards = (
  <>
    <HRCard
      id="CON-1252 · R-9120"
      pill={{ label: 'awaiting 38m', kind: 'warn' }}
      title={<>Acceptance unclear for &ldquo;make checkout faster&rdquo;</>}
      desc={
        <>
          Pick a baseline metric and target. Current p95 = <Mono>1.18s</Mono> across desktop + mobile.
        </>
      }
      prop={
        <>
          <b className="font-semibold text-ink">Proposed:</b> target p95 ≤ 800 ms desktop · budget 2 weeks
        </>
      }
      actions={[
        { label: 'Accept proposal', kind: 'primary' },
        { label: 'Edit criteria', kind: 'secondary' },
      ]}
    />
    <HRCard
      id="CON-1250 · R-9145"
      pill={{ label: 'awaiting 22m', kind: 'warn' }}
      title="Confirm Linear rollup — 3 tickets are duplicates of ENG-1182"
      desc="CON-1250, ENG-1184, ENG-1185 all describe bulk CSV import. Agent wants to deduplicate and continue under one parent."
      prop={
        <>
          <b className="font-semibold text-ink">Proposed:</b> rollup under CON-1250 · close ENG-1184/1185 as dup
        </>
      }
      actions={[
        { label: 'Confirm rollup', kind: 'primary' },
        { label: 'Keep separate', kind: 'secondary' },
      ]}
    />
    <HRCard
      id="CON-1253 · R-9128"
      pill={{ label: 'awaiting 17m', kind: 'warn' }}
      title="Approve copy change to triage-worker refusal path"
      desc="+3 lines · tone shift from formal to plain. No behaviour change expected."
      prop={
        <>
          <b className="font-semibold text-ink">Proposed:</b> apply as-is · rollout via percentile 10% → 100%
        </>
      }
      actions={[
        { label: 'Approve', kind: 'primary' },
        { label: 'View diff', kind: 'secondary' },
      ]}
    />
  </>
);

const thisWeekCards = (
  <>
    <HRCard
      id="CON-1230 · R-9012"
      pill={{ label: 'queued 2d', kind: 'done' }}
      title="Review proposed workflow change for delivery/default"
      desc={
        <>
          Adds a <Mono>policy-review</Mono> step before Gate. Tagged for owner approval.
        </>
      }
      actions={[
        { label: 'Open diff', kind: 'secondary' },
        { label: 'Reassign', kind: 'ghost' },
      ]}
    />
    <HRCard
      id="CON-1225 · R-8998"
      pill={{ label: 'queued 3d', kind: 'done' }}
      title="Confirm dependency upgrade plan for Q3"
      desc="Agent drafted 9-step bump (TypeScript, undici, vitest). Awaiting owner direction."
      actions={[
        { label: 'Read plan', kind: 'secondary' },
        { label: 'Reassign', kind: 'ghost' },
      ]}
    />
  </>
);
