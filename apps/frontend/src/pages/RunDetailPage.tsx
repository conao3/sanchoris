import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Crumbs, Screen, ScreenHead } from '../components/shell/Screen';
import { Btn, Pill } from '../components/shell/primitives';
import { ACCOUNT_ITEMS, WORKSPACE_ITEMS } from '../components/shell/railItems';

// ─── Page-local avatar helpers ────────────────────────────────────────────────
// Avatar from Topbar doesn't support sm+color combinations needed here.

function SmAvatar({ initials, coral }: { initials: string; coral?: boolean }) {
  return (
    <span
      className={`inline-flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center rounded-full text-[9.5px] font-semibold ${
        coral ? 'bg-primary text-white' : 'bg-surface-card text-ink'
      }`}
    >
      {initials}
    </span>
  );
}

function XsAvatar({ initials, coral }: { initials: string; coral?: boolean }) {
  return (
    <span
      className={`inline-flex h-[18px] w-[18px] flex-shrink-0 items-center justify-center rounded-full text-[9px] font-semibold ${
        coral ? 'bg-primary text-white' : 'bg-surface-card text-ink'
      }`}
    >
      {initials}
    </span>
  );
}

// ─── Workpad ──────────────────────────────────────────────────────────────────

function CheckItem({ done, children }: { done?: boolean; children: ReactNode }) {
  return (
    <li
      className={`flex items-start gap-2 py-[3px] text-[12.5px] leading-[1.6] ${
        done ? 'text-muted-soft line-through decoration-muted-soft' : 'text-body'
      }`}
    >
      <span
        className={`relative mt-[3px] flex h-[14px] w-[14px] flex-shrink-0 items-center justify-center rounded-[3px] border-[1.4px] ${
          done ? 'border-primary bg-primary' : 'border-hairline bg-canvas'
        }`}
      >
        {done && (
          <span className="text-[10px] font-semibold leading-none text-white">✓</span>
        )}
      </span>
      <span>{children}</span>
    </li>
  );
}

function CmdLine({ children }: { children: ReactNode }) {
  return (
    <div className="mb-1 rounded-[5px] border border-hairline-soft bg-canvas px-[9px] py-[6px] font-mono text-[11.5px] text-body last:mb-0">
      <span className="text-primary">$ </span>
      {children}
    </div>
  );
}

function WpBlock({
  label,
  right,
  children,
}: {
  label: string;
  right: string;
  children: ReactNode;
}) {
  return (
    <div className="rounded-[8px] border border-hairline bg-surface-soft px-[14px] py-[12px]">
      <div className="mb-2 flex items-center gap-2">
        <span className="text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
          {label}
        </span>
        <span className="ml-auto font-mono text-[11px] text-muted-soft">{right}</span>
      </div>
      {children}
    </div>
  );
}

function Workpad() {
  return (
    <div className="flex flex-col gap-[14px] overflow-auto border-r border-hairline px-[20px] py-[16px]">
      <h4 className="m-0 font-serif text-[16px] font-medium tracking-[-0.2px] text-ink">
        Workpad
      </h4>

      <WpBlock label="Plan" right="3 / 5 done">
        <ul className="m-0 list-none p-0">
          <CheckItem done>Read existing auth router, locate OIDC handler</CheckItem>
          <CheckItem done>Sketch OAuth flow for vendor identity provider</CheckItem>
          <CheckItem done>
            Add <span className="font-mono">/auth/vendor/callback</span> endpoint
          </CheckItem>
          <CheckItem>Wire vendor sessions to existing JWT layer</CheckItem>
          <CheckItem>Add Playwright happy-path test</CheckItem>
        </ul>
      </WpBlock>

      <WpBlock label="Acceptance criteria" right="3 items">
        <ul className="m-0 list-none p-0">
          <CheckItem>Vendor users sign in via OAuth and land on the vendor dashboard</CheckItem>
          <CheckItem>Existing OIDC customer flow is unchanged (regression)</CheckItem>
          <CheckItem>Audit log entry written for every vendor session</CheckItem>
        </ul>
      </WpBlock>

      <WpBlock label="Validation" right="defined in workflow">
        <CmdLine>pnpm vitest run apps/auth</CmdLine>
        <CmdLine>pnpm playwright test e2e/auth/vendor.spec.ts</CmdLine>
        <CmdLine>{'pnpm typecheck && pnpm lint'}</CmdLine>
      </WpBlock>
    </div>
  );
}

// ─── Timeline ─────────────────────────────────────────────────────────────────

type ToolBadge = { label: string; state?: 'ok' | 'fail' };

function ToolBadgeView({ badge }: { badge: ToolBadge }) {
  const cls =
    badge.state === 'ok'
      ? 'bg-[rgba(93,184,114,0.18)] text-[#386b46]'
      : badge.state === 'fail'
        ? 'bg-[rgba(198,69,69,0.16)] text-[#862e2e]'
        : 'bg-surface-dark text-on-dark';
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-[4px] px-[7px] py-[2px] font-mono text-[10.5px] ${cls}`}
    >
      {badge.label}
    </span>
  );
}

type TurnData = {
  coral?: boolean;
  initials: string;
  name: string;
  time: string;
  body: ReactNode;
  tools?: ToolBadge[];
};

const TURNS: TurnData[] = [
  {
    coral: true,
    initials: 'KO',
    name: 'kanagawa-ops',
    time: '13:38:12 JST',
    body: (
      <>
        Take CON-1247 — add OAuth for the new vendor portal. Use the existing JWT session layer.
        Don't touch the OIDC customer path.
      </>
    ),
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:38:18 JST',
    body: (
      <>
        Picked up. Reading <span className="font-mono">apps/auth/router.ts</span> and the memory
        note "Auth + SSO behaviour".
      </>
    ),
    tools: [{ label: 'mcp__linear' }, { label: 'mcp__memory' }, { label: 'gh' }],
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:39:02 JST',
    body: (
      <>
        Located the OIDC handler at L142. Vendor provider returns a different{' '}
        <span className="font-mono">aud</span> claim — I'll route on that.
      </>
    ),
    tools: [{ label: 'git · checkout -b issue-con-1247', state: 'ok' }],
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:40:18 JST',
    body: (
      <>
        Drafting <span className="font-mono">/auth/vendor/callback</span>. Will write the audit log
        entry next.
      </>
    ),
    tools: [{ label: 'fs · edit' }, { label: 'fs · create' }],
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:41:24 JST',
    body: <>Ran unit tests for auth module.</>,
    tools: [{ label: 'vitest · 42 pass / 0 fail', state: 'ok' }],
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:42:05 JST',
    body: (
      <>
        Ran Playwright happy path.
        <div className="mt-[6px] flex flex-wrap gap-[5px]">
          <ToolBadgeView badge={{ label: 'playwright · 8 pass / 1 fail', state: 'fail' }} />
        </div>
        <div className="mt-1">
          One Safari case failed on the cookie-rewrite hop — investigating now.
        </div>
      </>
    ),
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:42:38 JST',
    body: (
      <>
        Looks like the same ITP edge as the prior login-loop. Adding a top-level redirect fallback
        per memory note.
      </>
    ),
  },
  {
    coral: true,
    initials: 'KO',
    name: 'kanagawa-ops',
    time: '13:42:51 JST',
    body: <>Good. Keep the OIDC path untouched, please.</>,
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:43:11 JST',
    body: <>Done. Re-running Playwright now.</>,
    tools: [
      { label: 'playwright · re-run vendor.spec.ts', state: 'ok' },
      { label: 'git · status' },
    ],
  },
  {
    initials: 'cx',
    name: 'codex',
    time: '13:44:02 JST',
    body: (
      <>
        Committed{' '}
        <span className="font-mono">3f2a · feat(auth): OAuth vendor callback + ITP fallback</span>.
        Opened PR <span className="font-mono">#482</span>. Workflow will hold at{' '}
        <strong className="font-semibold">gate</strong> while merge policy resolves.
      </>
    ),
    tools: [
      { label: 'git · commit 3f2a', state: 'ok' },
      { label: 'gh · pr create #482', state: 'ok' },
    ],
  },
];

function TurnView({ turn }: { turn: TurnData }) {
  return (
    <div className="flex gap-3 border-b border-dashed border-hairline-soft py-[10px] last:border-b-0">
      <SmAvatar initials={turn.initials} coral={turn.coral} />
      <div className="min-w-0 flex-1 text-[12.5px] leading-[1.5] text-body">
        <div className="mb-1 flex items-baseline gap-[6px]">
          <span className="text-[11.5px] font-semibold text-ink">{turn.name}</span>
          <span className="font-mono text-[10.5px] text-muted-soft">{turn.time}</span>
        </div>
        <div>{turn.body}</div>
        {turn.tools && (
          <div className="mt-[6px] flex flex-wrap gap-[5px]">
            {turn.tools.map((t, i) => (
              <ToolBadgeView key={i} badge={t} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Timeline() {
  return (
    <div className="overflow-auto px-[18px] py-[16px]">
      {TURNS.map((turn, i) => (
        <TurnView key={i} turn={turn} />
      ))}
    </div>
  );
}

// ─── Right rail (ts-rail) ─────────────────────────────────────────────────────

function TsRailCard({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-[10px] border border-hairline bg-canvas p-4">{children}</div>
  );
}

function TsRailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between py-[5px] text-[12px] text-body">
      <span className="text-muted">{label}</span>
      {children}
    </div>
  );
}

function AvatarStack() {
  return (
    <div className="inline-flex items-center">
      <span className="inline-flex h-[22px] w-[22px] items-center justify-center rounded-full border-2 border-canvas bg-surface-card text-[9.5px] font-semibold text-ink">
        OC
      </span>
      <span className="-ml-2 inline-flex h-[22px] w-[22px] items-center justify-center rounded-full border-2 border-canvas bg-primary text-[9.5px] font-semibold text-white">
        KO
      </span>
      <span className="-ml-2 inline-flex h-[22px] w-[22px] items-center justify-center rounded-full border-2 border-canvas bg-surface-card text-[10px] font-medium text-body">
        +1
      </span>
    </div>
  );
}

function TsRail() {
  return (
    <div className="flex flex-col gap-[14px] overflow-auto border-l border-hairline bg-surface-soft px-[18px] py-[16px]">
      <div>
        <h5 className="m-0 mb-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
          Pull request
        </h5>
        <TsRailCard>
          <TsRailRow label="PR">
            <span className="font-mono font-semibold text-ink">#482</span>
          </TsRailRow>
          <TsRailRow label="Branch">
            <span className="font-mono text-[11px]">issue-con-1247</span>
          </TsRailRow>
          <TsRailRow label="Status">
            <Pill kind="validating">
              <span
                className="h-[6px] w-[6px] rounded-full bg-[rgba(255,255,255,0.85)]"
                aria-hidden="true"
              />
              validating
            </Pill>
          </TsRailRow>
          <TsRailRow label="Reviewers">
            <AvatarStack />
          </TsRailRow>
        </TsRailCard>
      </div>

      <div>
        <h5 className="m-0 mb-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
          Environment
        </h5>
        <TsRailCard>
          <TsRailRow label="Workspace">
            <span className="font-mono text-[11px]">ws-acme-plt-1d3f</span>
          </TsRailRow>
          <TsRailRow label="Image">
            <span className="font-mono text-[11px]">node 22 / pnpm 9</span>
          </TsRailRow>
          <TsRailRow label="Quota used">
            <span className="font-mono text-[11px]">62% CPU · 1.4G</span>
          </TsRailRow>
          <TsRailRow label="Files touched">
            <span className="font-mono text-[11px] font-semibold text-ink">+184 / -72 · 9</span>
          </TsRailRow>
        </TsRailCard>
      </div>

      <div>
        <h5 className="m-0 mb-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">
          Sync &amp; links
        </h5>
        <TsRailCard>
          <TsRailRow label="Linear">
            <span className="font-mono text-[11px] text-primary">CON-1247 ↗</span>
          </TsRailRow>
          <TsRailRow label="Slack thread">
            <span className="font-mono text-[11px] text-primary">#oncall-1d3f ↗</span>
          </TsRailRow>
          <TsRailRow label="Linked memory">
            <span className="text-[11px]">Auth + SSO, ITP cookie</span>
          </TsRailRow>
        </TsRailCard>
      </div>
    </div>
  );
}

// ─── Validation bar ───────────────────────────────────────────────────────────

function VBlock({
  label,
  sub,
  fail,
  expand,
}: {
  label: string;
  sub: string;
  fail?: boolean;
  expand?: boolean;
}) {
  return (
    <div
      className={`flex min-w-[200px] items-center gap-[10px] rounded-[8px] border px-[12px] py-[8px] ${
        fail
          ? 'border-[rgba(198,69,69,0.4)] bg-[rgba(198,69,69,0.06)]'
          : 'border-hairline bg-canvas'
      }`}
    >
      <div
        className={`flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center rounded-full text-[12px] font-bold ${
          fail
            ? 'bg-[rgba(198,69,69,0.16)] text-[#862e2e]'
            : 'bg-[rgba(93,184,114,0.18)] text-[#386b46]'
        }`}
      >
        {fail ? '!' : '✓'}
      </div>
      <div className="flex flex-col gap-[2px] leading-[1.3]">
        <span className="text-[12.5px] font-medium text-ink">{label}</span>
        <span className="font-mono text-[10.5px] text-muted">{sub}</span>
      </div>
      {expand && (
        <span className="ml-auto cursor-pointer font-mono text-[10.5px] text-primary">
          expand ↓
        </span>
      )}
    </div>
  );
}

function ValidationBar() {
  return (
    <div className="flex items-center gap-[14px] bg-surface-soft px-[24px] py-[14px]">
      <span className="mr-[6px] flex-shrink-0 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
        Validation
      </span>
      <VBlock label="vitest" sub="42 pass / 0 fail · 11.4s" />
      <VBlock label="typecheck" sub="tsc · 0 errors" />
      <VBlock label="lint" sub="eslint · 0 warn" />
      <VBlock label="playwright" sub="8 pass / 1 fail · safari ITP retry" fail expand />
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

const railItems = WORKSPACE_ITEMS.map((item) => ({
  ...item,
  active: item.key === 'queue',
}));

export function RunDetailPage() {
  return (
    <AppShell rail={<Rail items={railItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', 'Runs', 'R-9143 · CON-1247']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Add OAuth login for vendor portal{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="running">
                    <span
                      className="h-[6px] w-[6px] rounded-full bg-[rgba(255,255,255,0.85)]"
                      aria-hidden="true"
                    />
                    execute · step 7/10
                  </Pill>
                  <span className="inline-flex items-center gap-[6px] rounded-full bg-surface-card px-[10px] py-[3px] font-mono text-[11.5px] font-medium leading-[1.4] whitespace-nowrap text-ink">
                    04:18 elapsed
                  </span>
                </span>
              </h1>
              <div className="mt-[6px] flex flex-wrap items-center gap-[14px] text-[13px] text-muted">
                <span className="font-mono text-[12px]">CON-1247</span>
                <span aria-hidden="true">·</span>
                <span>
                  worker <strong className="font-semibold text-ink">codex w-cx-1</strong>
                </span>
                <span aria-hidden="true">·</span>
                <span>
                  branch <span className="font-mono">issue-con-1247</span>
                </span>
                <span aria-hidden="true">·</span>
                <span>
                  workspace <span className="font-mono">ws-acme-plt-1d3f</span>
                </span>
                <span aria-hidden="true">·</span>
                <span className="flex items-center gap-[5px]">
                  owner <XsAvatar initials="KO" coral /> kanagawa-ops
                </span>
              </div>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Pause</Btn>
              <Btn variant="secondary">Take over</Btn>
              <Btn variant="primary">Open PR #482 ↗</Btn>
            </div>
          </div>
        </ScreenHead>

        {/* F04 body: 2-row grid — no padding to keep columns full-bleed */}
        <div
          className="min-h-0 flex-1 overflow-hidden"
          style={{ display: 'grid', gridTemplateRows: '1fr 110px' }}
        >
          {/* ts-main: 3-column grid */}
          <div
            className="min-h-0 overflow-hidden border-b border-hairline"
            style={{ display: 'grid', gridTemplateColumns: '460px 1fr 340px' }}
          >
            <Workpad />
            <Timeline />
            <TsRail />
          </div>
          <ValidationBar />
        </div>
      </Screen>
    </AppShell>
  );
}
