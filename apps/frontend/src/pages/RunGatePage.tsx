import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, Crumbs } from '../components/shell/Screen';
import { Btn, Pill, Mono } from '../components/shell/primitives';
import { ACCOUNT_ITEMS, WORKSPACE_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── W7 Hero ──────────────────────────────────────────────────────────────────

function W7Hero() {
  return (
    <div className="flex flex-shrink-0 items-center gap-4 border-b border-hairline bg-surface-soft px-6 py-[14px]">
      <div className="min-w-0 flex-1">
        <h3
          className="mb-[6px] cursor-pointer font-serif text-[22px] font-medium leading-[1.15] tracking-[-0.3px] text-ink"
          onClick={() => navigate('/runs/r-9143')}
        >
          <span className="mr-[6px] font-mono text-[13px] font-normal text-muted">CON-1247</span>
          Add OAuth login for vendor portal
        </h3>
        <div className="flex flex-wrap items-center gap-[14px] text-[11.5px] text-muted">
          <span>
            workflow{' '}
            <strong className="font-mono text-[13px] font-semibold text-body-strong">
              delivery/default v0.3.3
            </strong>
          </span>
          <span className="rounded-full bg-[rgba(204,120,92,0.16)] px-[7px] py-[2px] text-[10px] font-semibold tracking-[0.4px] text-primary-active">
            just published
          </span>
          <span className="text-muted-soft">·</span>
          <span>
            source <strong className="font-semibold text-body-strong">Linear</strong>
          </span>
          <span className="text-muted-soft">·</span>
          <span>
            worker <strong className="font-semibold text-body-strong">Claude Code</strong>
          </span>
          <span className="text-muted-soft">·</span>
          <span>
            started{' '}
            <strong className="font-mono font-semibold text-body-strong">13:42 JST · today</strong>
          </span>
          <span className="text-muted-soft">·</span>
          <span>
            age <strong className="font-mono font-semibold text-body-strong">00:51:18</strong>
          </span>
        </div>
      </div>
      <div className="flex flex-shrink-0 gap-[6px]">
        <Pill kind="gate">
          <span className="h-[6px] w-[6px] rounded-full bg-white/85" aria-hidden="true" />
          awaiting Gate #3 · 22m
        </Pill>
        <span className="inline-flex items-center gap-[6px] rounded-full bg-surface-card px-[10px] py-[3px] font-mono text-[11.5px] font-medium leading-[1.4] whitespace-nowrap text-ink">
          state 15 / 18
        </span>
      </div>
    </div>
  );
}

// ─── W7 Beads DAG ─────────────────────────────────────────────────────────────

type BeadKind = 'done' | 'curr' | 'pend';

type BeadDef = {
  label: string;
  age?: string;
  autoBadge?: string | { label: string; style?: string };
  kind: BeadKind;
  isGate?: boolean;
  num?: string;
  pendConnAfter?: boolean;
};

const BEADS: BeadDef[] = [
  { label: 'Source', age: '0.4s', kind: 'done' },
  { label: 'Normalize', age: '0.9s', kind: 'done' },
  { label: 'Prioritize', age: '0.2s', kind: 'done' },
  { label: 'Workflow', age: '0.1s', kind: 'done' },
  { label: 'Worker', age: '0.1s', kind: 'done' },
  { label: 'Workspace', age: '3.1s', kind: 'done' },
  { label: 'Execute', age: '3m 41s', kind: 'done' },
  { label: 'Validate', age: '14.6s', kind: 'done' },
  { label: 'Gate #1', age: 'peer-review · 3m', kind: 'done', isGate: true },
  { label: 'Merge', age: '2.4s', kind: 'done' },
  { label: 'Build', age: '38s', kind: 'done' },
  { label: 'Gate #2', autoBadge: 'auto', kind: 'done', isGate: true },
  { label: 'Deploy stg', age: '52s', kind: 'done' },
  { label: 'Smoke', age: '1m 12s', kind: 'done', pendConnAfter: true },
  { label: 'Gate #3', age: 'awaiting · 22m', autoBadge: '0 / 2', num: '15', kind: 'curr', isGate: true, pendConnAfter: true },
  { label: 'Deploy prod', age: '—', num: '16', kind: 'pend', pendConnAfter: true },
  { label: 'Gate #4', age: 'auto-gate', num: '17', kind: 'pend', isGate: true, pendConnAfter: true },
  { label: 'Update', age: '—', num: '18', kind: 'pend' },
];

function BeadDot({ bead }: { bead: BeadDef }) {
  if (bead.kind === 'done') {
    return (
      <span className="relative z-[2] inline-flex h-[20px] w-[20px] flex-shrink-0 items-center justify-center rounded-full bg-success text-[10.5px] font-bold text-white">
        ✓
      </span>
    );
  }
  if (bead.kind === 'curr') {
    return (
      <span
        className="w7-curr-dot relative z-[2] inline-flex h-[20px] w-[20px] flex-shrink-0 items-center justify-center rounded-full bg-primary text-[9.5px] font-bold text-white"
        style={{ boxShadow: '0 0 0 4px rgba(204,120,92,0.22)' }}
      >
        {bead.num}
      </span>
    );
  }
  return (
    <span className="relative z-[2] inline-flex h-[20px] w-[20px] flex-shrink-0 items-center justify-center rounded-full bg-surface-card text-[9.5px] font-bold text-muted-soft">
      {bead.num}
    </span>
  );
}

function Bead({ bead }: { bead: BeadDef }) {
  const labelColor =
    bead.kind === 'curr'
      ? 'text-primary-active font-semibold'
      : bead.kind === 'pend'
        ? 'text-muted-soft'
        : bead.isGate
          ? 'text-body-strong font-semibold'
          : 'text-muted';

  const ageColor = bead.kind === 'curr' ? 'text-primary' : 'text-muted-soft';

  return (
    <div className="flex min-w-0 flex-col items-center gap-[4px]">
      <BeadDot bead={bead} />
      <span
        className={`min-w-[56px] text-center text-[9px] leading-[1.25] ${labelColor}`}
      >
        {bead.label}
        {bead.autoBadge && bead.kind === 'curr' && (
          <span
            className="mt-[2px] block font-mono text-[7.5px] font-bold uppercase tracking-[0.5px]"
            style={{ color: 'var(--color-primary-active)', background: 'rgba(204,120,92,0.16)', padding: '1px 4px', borderRadius: '3px', display: 'inline-block', marginLeft: '4px' }}
          >
            {bead.autoBadge as string}
          </span>
        )}
        {bead.autoBadge && bead.kind === 'done' && (
          <span
            className="mt-[2px] block font-mono text-[7.5px] font-bold uppercase tracking-[0.5px]"
            style={{ color: '#386b46', background: 'rgba(93,184,114,0.18)', padding: '1px 4px', borderRadius: '3px', display: 'inline-block', marginLeft: '2px' }}
          >
            {bead.autoBadge as string}
          </span>
        )}
        {bead.age && (
          <span className={`mt-[1px] block font-mono text-[8px] ${ageColor}`}>
            {bead.age}
          </span>
        )}
      </span>
    </div>
  );
}

function W7Beads() {
  return (
    <>
      <style>{`
        @keyframes w7pulse {
          50% { box-shadow: 0 0 0 9px rgba(204,120,92,0.06); }
        }
        .w7-curr-dot {
          animation: w7pulse 1.8s ease-in-out infinite;
        }
      `}</style>
      <div className="flex flex-shrink-0 items-start border-b border-hairline bg-canvas px-6 py-4 pb-[18px]">
        {BEADS.map((bead, i) => (
          <div key={i} className="contents">
            <Bead bead={bead} />
            {i < BEADS.length - 1 && (
              <div
                className={`h-[2px] min-w-[6px] flex-1 self-start ${
                  bead.pendConnAfter ? 'bg-surface-card' : 'bg-success'
                }`}
                style={{ marginTop: '9px' }}
              />
            )}
          </div>
        ))}
      </div>
    </>
  );
}

// ─── Gate History ─────────────────────────────────────────────────────────────

function GateHistory() {
  return (
    <div className="mb-[14px] rounded-[8px] border border-hairline bg-surface-soft p-[10px_14px]">
      <h6 className="mb-[6px] text-[10px] font-semibold uppercase tracking-[1.4px] text-muted">
        Gate history (this run)
      </h6>
      <div className="flex items-baseline gap-[10px] border-b border-dashed border-hairline-soft py-[5px] text-[11.5px]">
        <span className="flex-shrink-0 rounded-[3px] bg-[rgba(232,165,90,0.25)] px-[5px] py-[1px] font-mono text-[9px] font-bold tracking-[0.4px] text-[#7a4d10]">
          #1
        </span>
        <span className="min-w-[124px] font-semibold text-ink">peer-review</span>
        <span className="flex-1 text-[#386b46]">
          approved by <strong className="font-semibold text-body-strong">@yamato-77</strong> · codeowner of{' '}
          <span className="rounded-[3px] bg-white/40 px-[4px] py-[1px] font-mono text-[10.5px]">
            apps/auth/**
          </span>
        </span>
        <span className="flex-shrink-0 font-mono text-[10px] text-muted-soft">
          13:45 JST · waited 03m
        </span>
      </div>
      <div className="flex items-baseline gap-[10px] border-b border-dashed border-hairline-soft py-[5px] text-[11.5px]">
        <span className="flex-shrink-0 rounded-[3px] bg-[rgba(232,165,90,0.25)] px-[5px] py-[1px] font-mono text-[9px] font-bold tracking-[0.4px] text-[#7a4d10]">
          #2
        </span>
        <span className="min-w-[124px] font-semibold text-ink">staging-deploy</span>
        <span className="flex-1 text-[#386b46]">
          <strong className="font-semibold text-body-strong">auto-approved</strong> · checks green, no migrations
        </span>
        <span className="flex-shrink-0 font-mono text-[10px] text-muted-soft">
          14:02 JST · waited 00m
        </span>
      </div>
      <div className="flex items-baseline gap-[10px] py-[5px] text-[11.5px]">
        <span className="flex-shrink-0 rounded-[3px] bg-[rgba(232,165,90,0.25)] px-[5px] py-[1px] font-mono text-[9px] font-bold tracking-[0.4px] text-[#7a4d10]">
          #3
        </span>
        <span className="min-w-[124px] font-semibold text-ink">prod-deploy</span>
        <span className="flex-1 font-medium text-primary-active">
          <strong className="font-bold text-primary-active">awaiting</strong> since 14:11 JST · 0 of 2 approvers responded
        </span>
        <span className="flex-shrink-0 font-mono text-[10px] text-muted-soft">
          22m elapsed · timeout 3h 38m
        </span>
      </div>
    </div>
  );
}

// ─── AND Rule ─────────────────────────────────────────────────────────────────

function AndRule() {
  return (
    <div className="my-3 mb-[14px] flex items-stretch gap-[10px] rounded-[8px] border border-hairline bg-surface-soft p-3">
      {/* KO slot */}
      <div className="flex min-w-0 flex-1 items-center gap-[9px] rounded-[7px] border-[1.5px] bg-[rgba(232,165,90,0.05)] p-[9px_12px] [border-color:rgba(232,165,90,0.55)]">
        <span className="inline-flex h-[30px] w-[30px] flex-shrink-0 items-center justify-center rounded-full bg-primary text-[11px] font-semibold text-white">
          KO
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[12.5px] font-semibold text-ink">@oncall</div>
          <div className="mt-[1px] font-mono text-[10.5px] text-[#7a4d10]">pending · KO is on-call</div>
        </div>
      </div>
      {/* AND connector */}
      <span className="inline-flex flex-shrink-0 items-center self-center rounded-full bg-[rgba(204,120,92,0.2)] px-[11px] font-mono text-[10px] font-bold tracking-[1.2px] text-primary-active">
        AND
      </span>
      {/* HK slot */}
      <div className="flex min-w-0 flex-1 items-center gap-[9px] rounded-[7px] border-[1.5px] bg-[rgba(232,165,90,0.05)] p-[9px_12px] [border-color:rgba(232,165,90,0.55)]">
        <span className="inline-flex h-[30px] w-[30px] flex-shrink-0 items-center justify-center rounded-full bg-surface-card text-[11px] font-semibold text-ink">
          HK
        </span>
        <div className="min-w-0 flex-1">
          <div className="text-[12.5px] font-semibold text-ink">@pm</div>
          <div className="mt-[1px] font-mono text-[10.5px] text-[#7a4d10]">pending · HK leading product</div>
        </div>
      </div>
    </div>
  );
}

// ─── Code Diff ────────────────────────────────────────────────────────────────

function CodeDiff() {
  return (
    <div className="mb-[14px] rounded-[8px] bg-surface-dark p-[10px_14px] font-mono text-[11px] leading-[1.6] text-on-dark">
      <div className="mb-[2px] text-[10px] text-muted-soft">
        staging tested vs what&apos;s shipping to prod · +5 / -2
      </div>
      <div>
        <span className="text-[#e58176]">
          {'- if (Date.now() > exp) return redirect("/login");'}
        </span>
      </div>
      <div>
        <span className="text-[#7fcc8f]">
          {'+ const skewMs = opts?.allowSkewMs ?? 90_000;'}
        </span>
      </div>
      <div>
        <span className="text-[#7fcc8f]">
          {'+ if (Date.now() > exp + skewMs) return redirect("/login");'}
        </span>
      </div>
      <div>
        <span className="text-[#e58176]">
          {'- export function loginWithSession(jwt: string) {'}
        </span>
      </div>
      <div>
        <span className="text-[#7fcc8f]">
          {'+ export function loginWithSession(jwt: string, opts?: LoginOpts) {'}
        </span>
      </div>
      <div className="text-on-dark">
        {'  '}
        <span className="text-muted-soft">
          {'// + audit.log on every vendor session (apps/auth/oauth.ts)'}
        </span>
      </div>
    </div>
  );
}

// ─── Property Grid ────────────────────────────────────────────────────────────

type PropRow = { label: string; value: ReactNode };

const PROP_ROWS: PropRow[] = [
  {
    label: 'Risk',
    value: <span className="font-medium text-primary-active">medium · touches auth</span>,
  },
  {
    label: 'Policy',
    value: <Mono>prod-deploy</Mono>,
  },
  {
    label: 'Staging',
    value: (
      <>
        <span className="font-semibold text-[#386b46]">✓</span> 1h 09m soak ·{' '}
        <strong className="font-semibold">0 errors</strong> · p95 412ms
      </>
    ),
  },
  {
    label: 'Canary plan',
    value: '5% eu-west · soak 30m · auto-revert on SLO breach',
  },
  {
    label: 'PR',
    value: <Mono>acme-platform · #482 (merged 14:00 JST)</Mono>,
  },
  {
    label: 'Auto-approve?',
    value: (
      <span className="text-muted">
        no — prod-deploy never auto-approves (by design)
      </span>
    ),
  },
];

function PropGrid() {
  return (
    <div
      className="mb-[14px]"
      style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '10px 16px' }}
    >
      {PROP_ROWS.map((row) => (
        <div
          key={row.label}
          className="flex items-baseline gap-[10px] border-b border-dashed border-hairline-soft py-[6px] text-[12px]"
        >
          <span className="min-w-[96px] flex-shrink-0 text-[10.5px] font-semibold uppercase tracking-[1.1px] text-muted">
            {row.label}
          </span>
          <span className="font-medium text-ink">{row.value}</span>
        </div>
      ))}
    </div>
  );
}

// ─── Gate Approval Panel ──────────────────────────────────────────────────────

function GatePanel() {
  return (
    <div className="min-h-0 overflow-auto border-r border-hairline px-[22px] py-[18px]">
      <GateHistory />

      <h4 className="mb-[4px] font-serif text-[20px] font-medium leading-[1] tracking-[-0.3px] text-ink">
        Approve prod deploy
      </h4>
      <p className="mb-[14px] text-[12px] text-muted">
        Gate <strong className="font-semibold text-body-strong">#3 prod-deploy</strong> · AND rule
        on approvers · matches policy{' '}
        <strong className="font-semibold text-body-strong">prod-deploy</strong>
      </p>

      <AndRule />
      <CodeDiff />
      <PropGrid />

      {/* Actions */}
      <div className="mt-[14px] flex items-center gap-2 border-t border-hairline pt-[14px]">
        <button
          type="button"
          className="inline-flex h-[34px] cursor-not-allowed items-center gap-[7px] rounded-[7px] border px-[14px] text-[13px] font-medium"
          style={{
            background: 'var(--color-surface-card)',
            color: 'var(--color-muted-soft)',
            borderColor: 'var(--color-surface-card)',
          }}
          disabled
        >
          Approve &amp; continue
          <span
            className="rounded-[3px] font-mono text-[10.5px]"
            style={{ marginLeft: '6px', padding: '1px 5px', background: 'rgba(255,255,255,0.4)' }}
          >
            1 of 2
          </span>
        </button>
        <Btn variant="secondary">Defer 1h</Btn>
        <Btn variant="danger">Reject with note</Btn>
        <span className="ml-auto font-mono text-[11px] text-muted">
          awaiting you (KO) · HK has not opened the request yet
        </span>
      </div>
    </div>
  );
}

// ─── Workpad Checklist ────────────────────────────────────────────────────────

const WORKPAD_ITEMS = [
  'Read existing auth router, locate OIDC handler',
  'Sketch OAuth flow for vendor identity provider',
  <>Add <span className="rounded-[3px] bg-surface-soft px-[5px] py-[1px] font-mono text-[10.5px]">/auth/vendor/callback</span></>,
  'Wire vendor sessions to existing JWT layer',
  'Playwright happy-path · 9/9 green',
];

function WorkpadCard() {
  return (
    <div className="rounded-[8px] border border-hairline bg-canvas p-[11px_13px]">
      <ul className="m-0 list-none p-0">
        {WORKPAD_ITEMS.map((item, i) => (
          <li
            key={i}
            className="flex items-start gap-2 py-[3px] text-[12px] leading-[1.55] text-muted-soft line-through decoration-muted-soft"
          >
            <span className="relative mt-[2px] h-[13px] w-[13px] flex-shrink-0 rounded-[3px] border-[1.4px] border-primary bg-primary">
              <span className="absolute inset-0 text-center font-semibold leading-[11px] text-[9px] text-white">
                ✓
              </span>
            </span>
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ─── Agent Timeline ───────────────────────────────────────────────────────────

const TOOL_CALLS = [
  'mcp__linear · get CON-1247',
  'gh · checkout -b issue-con-1247',
  'git · commit 3f2a',
  'vitest · 42 / 0',
  'playwright · 9 / 0',
  'gh · pr create #482',
];

function AgentTimelineCard() {
  return (
    <div className="rounded-[8px] border border-hairline bg-canvas p-[11px_13px]">
      <div className="flex flex-wrap gap-1">
        {TOOL_CALLS.map((tool, i) => (
          <span
            key={i}
            className="inline-flex items-center gap-1 rounded-[4px] px-[7px] py-[2px] font-mono text-[10px]"
            style={{ background: 'rgba(93,184,114,0.18)', color: '#386b46' }}
          >
            {tool}
          </span>
        ))}
      </div>
      <div className="mt-2 font-mono text-[11px] text-muted-soft">
        last action 14:11:23 JST · idle since 22m (waiting on Gate #3)
      </div>
    </div>
  );
}

// ─── Right Rail ───────────────────────────────────────────────────────────────

function RunRail() {
  return (
    <div className="flex min-h-0 flex-col gap-3 overflow-auto bg-surface-soft px-[18px] py-4">
      <div>
        <h5 className="mb-[6px] flex items-baseline gap-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Workpad{' '}
          <span className="font-mono text-[10px] font-medium text-muted-soft">5 items · 5 done</span>
        </h5>
        <WorkpadCard />
      </div>

      <div>
        <h5 className="mb-[6px] flex items-baseline gap-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Agent timeline{' '}
          <span className="font-mono text-[10px] font-medium text-muted-soft">last 6 tool calls</span>
        </h5>
        <AgentTimelineCard />
      </div>

      <div>
        <h5 className="mb-[6px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Quick context
        </h5>
        <div className="rounded-[8px] border border-hairline bg-canvas p-[11px_13px] text-[11.5px] leading-[1.55] text-body">
          Worker is parked on Gate #3.{' '}
          <strong className="font-semibold text-ink">No tools fire</strong> until both approvers
          respond. AND-rule is strict — one approval is not enough.
        </div>
      </div>
    </div>
  );
}

// ─── W7 Body ──────────────────────────────────────────────────────────────────

function W7Body() {
  return (
    <div
      className="min-h-0 overflow-hidden"
      style={{ display: 'grid', gridTemplateColumns: '1.45fr 1fr', flex: '1' }}
    >
      <GatePanel />
      <RunRail />
    </div>
  );
}

// ─── W7 Footer ────────────────────────────────────────────────────────────────

function W7Foot() {
  return (
    <div className="flex flex-shrink-0 items-center gap-[18px] border-t border-hairline bg-surface-soft px-6 py-[11px] text-[12px]">
      <a className="cursor-pointer text-primary no-underline hover:underline">
        ↗ Open in Workflow editor (delivery/default v0.3.3)
      </a>
      <span className="text-muted-soft">·</span>
      <a
        className="cursor-pointer text-primary no-underline hover:underline"
        onClick={() => navigate('/pull-requests')}
      >↗ Open PR #482</a>
      <span className="text-muted-soft">·</span>
      <a className="cursor-pointer text-primary no-underline hover:underline">
        ↗ Open Linear CON-1247
      </a>
      <span className="ml-auto font-mono text-[11px] text-muted">
        workspace ws-acme-plt-1d3f · auto-GC after Gate #4
      </span>
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

export function RunGatePage() {
  const railItems = WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'queue' }));

  return (
    <AppShell rail={<Rail items={railItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', { label: 'Runs', href: '/gates' }, 'R-9143 · CON-1247']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Run · awaiting Gate #3 prod-deploy{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="gate">
                    <span
                      className="h-[6px] w-[6px] rounded-full bg-white/85"
                      aria-hidden="true"
                    />
                    awaiting Gate #3 · 22m
                  </Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Live operational view · passed Gate #1, auto-approved Gate #2, now waiting on the
                AND-rule at Gate #3
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Pause run</Btn>
              <Btn variant="secondary">Take over</Btn>
            </div>
          </div>
        </ScreenHead>

        {/* W7 body sections — full-bleed, no ScreenBody padding */}
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <W7Hero />
          <W7Beads />
          <W7Body />
          <W7Foot />
        </div>
      </Screen>
    </AppShell>
  );
}
