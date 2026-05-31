import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Kbd, Mono, Pill, type PillKind } from '../components/shell/primitives';
import { Avatar, type AvatarVariant } from '../components/shell/Topbar';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Rail items ────────────────────────────────────────────────────────────────

const prRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'prs',
}));

// ─── Local primitives ─────────────────────────────────────────────────────────

function Chip({ active, count, children }: { active?: boolean; count?: number; children: ReactNode }) {
  return (
    <span
      className={`inline-flex cursor-pointer items-center gap-[5px] rounded-full px-[10px] py-[3px] text-[12px] font-medium ${
        active
          ? 'bg-ink text-canvas'
          : 'border border-hairline bg-canvas text-body'
      }`}
    >
      {children}
      {count != null && (
        <span className={`text-[11px] ${active ? 'opacity-60' : 'text-muted'}`}>{count}</span>
      )}
    </span>
  );
}

type CheckState = 'ok' | 'run' | 'fail' | 'pend';

function CheckDot({ state }: { state: CheckState }) {
  const cls: Record<CheckState, string> = {
    ok: 'text-[#386b46] font-bold',
    run: 'text-primary',
    fail: 'text-error font-bold',
    pend: 'text-muted-soft',
  };
  const label: Record<CheckState, string> = {
    ok: '✓',
    run: '◐',
    fail: '×',
    pend: '·',
  };
  return <span className={`text-[13px] leading-none ${cls[state]}`}>{label[state]}</span>;
}

function Checks({ states }: { states: CheckState[] }) {
  return (
    <div className="flex items-center gap-[4px]">
      {states.map((s, i) => (
        <CheckDot key={i} state={s} />
      ))}
    </div>
  );
}

// ─── PR data ──────────────────────────────────────────────────────────────────

type AvatarDef = { initials: string; variant: AvatarVariant };

type PRRow = {
  repo: string;
  num: string;
  branch: string;
  title: string;
  meta: string;
  status: PillKind;
  statusLabel: string;
  checks: CheckState[];
  reviewers: AvatarDef[];
  moreReviewers?: number;
  age: string;
  gate: { kind: PillKind; label: string };
};

const PR_ROWS: PRRow[] = [
  {
    repo: 'acme-platform', num: '#482', branch: 'issue-con-1247',
    title: 'feat(auth): OAuth login for vendor portal',
    meta: 'opened by yamato-77 via run R-9143',
    status: 'validating', statusLabel: 'validating',
    checks: ['ok', 'ok', 'ok', 'run'],
    reviewers: [{ initials: 'OC', variant: 'sm' }, { initials: 'KO', variant: 'coral' }],
    moreReviewers: 1,
    age: '8 m',
    gate: { kind: 'gate', label: 'awaiting' },
  },
  {
    repo: 'acme-platform', num: '#481', branch: 'fix/login-sso-loop',
    title: 'fix(auth): tolerate ±90s JWT skew',
    meta: 'opened by octocat via run R-9141',
    status: 'success', statusLabel: 'ready',
    checks: ['ok', 'ok', 'ok', 'ok'],
    reviewers: [{ initials: 'SH', variant: 'sm' }, { initials: 'SD', variant: 'teal' }],
    age: '1 h',
    gate: { kind: 'done', label: 'cleared' },
  },
  {
    repo: 'acme-platform', num: '#480', branch: 'perf/checkout-lcp',
    title: 'perf(checkout): lazy-load assets, defer non-critical JS',
    meta: 'opened by shibuya-dev · 2 commits ahead of main',
    status: 'default', statusLabel: 'review',
    checks: ['ok', 'ok', 'run', 'pend'],
    reviewers: [{ initials: 'ER', variant: 'amber' }],
    age: '3 h',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'stripe-bridge', num: '#87', branch: 'feat/webhook-retry',
    title: 'feat: retry/backoff on webhook fanout',
    meta: 'opened by octocat via run R-9132',
    status: 'failed', statusLabel: 'failing',
    checks: ['ok', 'ok', 'ok', 'fail'],
    reviewers: [{ initials: 'OC', variant: 'sm' }],
    age: '4 h',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'stripe-bridge', num: '#86', branch: 'chore/release-v0.19',
    title: 'chore: release v0.19.0 (env var STRIPE_WEBHOOK_V2_SECRET)',
    meta: 'opened by kanagawa-ops',
    status: 'success', statusLabel: 'ready',
    checks: ['ok', 'ok', 'ok', 'ok'],
    reviewers: [{ initials: 'KO', variant: 'coral' }, { initials: 'HK', variant: 'sm' }],
    age: '5 h',
    gate: { kind: 'gate', label: 'deploy' },
  },
  {
    repo: 'growth-api', num: '#214', branch: 'chore/ts-5.5',
    title: 'chore: bump TypeScript 5.4 → 5.5',
    meta: 'opened by eto-r via run R-9119',
    status: 'success', statusLabel: 'ready',
    checks: ['ok', 'ok', 'ok', 'ok'],
    reviewers: [{ initials: 'ER', variant: 'amber' }, { initials: 'SH', variant: 'sm' }],
    age: '6 h',
    gate: { kind: 'done', label: 'cleared' },
  },
  {
    repo: 'growth-api', num: '#213', branch: 'feat/tenant-rate-limits',
    title: 'feat: tenant-scoped rate limits on /api/checkout',
    meta: 'opened by yamato-77',
    status: 'running', statusLabel: 'running',
    checks: ['ok', 'run', 'run', 'pend'],
    reviewers: [{ initials: 'OC', variant: 'sm' }, { initials: 'KO', variant: 'coral' }],
    age: '12 h',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'acme-platform', num: '#478', branch: 'feat/audit-log-fill',
    title: 'feat: backfill audit log for 2026-04-30 gap',
    meta: 'opened by octocat',
    status: 'draft', statusLabel: 'draft',
    checks: ['pend', 'pend', 'pend', 'pend'],
    reviewers: [{ initials: 'SH', variant: 'sm' }],
    age: '18 h',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'acme-platform', num: '#476', branch: 'refactor/auth-router',
    title: 'refactor(auth): split router for OIDC vs SAML paths',
    meta: 'opened by sato-h',
    status: 'default', statusLabel: 'review',
    checks: ['ok', 'ok', 'ok', 'ok'],
    reviewers: [
      { initials: 'OC', variant: 'sm' },
      { initials: 'KO', variant: 'coral' },
      { initials: 'SH', variant: 'sm' },
    ],
    age: '1 d',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'acme-platform', num: '#474', branch: 'test/auth-safari-flake',
    title: 'test(auth): stabilise Safari e2e flake on session refresh',
    meta: 'opened by eto-r via run R-9094',
    status: 'default', statusLabel: 'review',
    checks: ['ok', 'ok', 'ok', 'ok'],
    reviewers: [{ initials: 'ER', variant: 'amber' }],
    age: '1 d',
    gate: { kind: 'done', label: '—' },
  },
  {
    repo: 'platform-docs', num: '#38', branch: 'docs/auth-scopes',
    title: 'docs: clarify auth scope changes for v0.19',
    meta: 'opened by kanagawa-ops',
    status: 'draft', statusLabel: 'draft',
    checks: ['ok', 'ok', 'pend', 'pend'],
    reviewers: [{ initials: 'SH', variant: 'sm' }],
    age: '2 d',
    gate: { kind: 'done', label: '—' },
  },
];

// ─── PR table row ─────────────────────────────────────────────────────────────

function PRTableRow({ row }: { row: PRRow }) {
  return (
    <tr className="border-b border-hairline">
      <td className="py-[10px] pr-4 align-top">
        <div className="text-[13px] font-semibold text-ink">
          {row.repo} · {row.num}
        </div>
        <div className="mt-[2px] font-mono text-[11px] text-muted">{row.branch}</div>
      </td>
      <td className="py-[10px] pr-4 align-top">
        <div className="text-[13px] font-medium text-ink">{row.title}</div>
        <div className="mt-[2px] text-[11.5px] text-muted">{row.meta}</div>
      </td>
      <td className="py-[10px] pr-4 align-top">
        <Pill kind={row.status}>{row.statusLabel}</Pill>
      </td>
      <td className="py-[10px] pr-4 align-top">
        <Checks states={row.checks} />
      </td>
      <td className="py-[10px] pr-4 align-top">
        <div className="flex items-center gap-[3px]">
          {row.reviewers.map((av, i) => (
            <Avatar key={i} variant={av.variant} title={av.initials}>
              {av.initials}
            </Avatar>
          ))}
          {row.moreReviewers != null && (
            <span className="inline-flex h-[22px] items-center justify-center rounded-full bg-surface-card px-[6px] font-mono text-[10px] text-muted">
              +{row.moreReviewers}
            </span>
          )}
        </div>
      </td>
      <td className="py-[10px] pr-4 align-top font-mono text-[12px] text-muted">{row.age}</td>
      <td className="py-[10px] align-top">
        <Pill kind={row.gate.kind}>{row.gate.label}</Pill>
      </td>
    </tr>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

export function PullRequestsPage() {
  return (
    <AppShell rail={<Rail items={prRailItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Pull requests']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Pull requests{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="default"><Mono>11 open</Mono></Pill>
                  <Pill kind="warn">1 failing</Pill>
                  <Pill kind="gate">1 awaiting gate</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Opened by agent runs across 4 repos ·{' '}
                <strong className="font-semibold text-body-strong">2 ready to merge</strong>
                {' '}· updated 13:38 JST today
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Open in GitHub ↗</Btn>
              <Btn variant="primary">Bulk approve</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex flex-col gap-4 overflow-hidden">
            <div className="flex flex-wrap items-center gap-[6px]">
              <Chip active count={11}>All repos</Chip>
              <Chip count={6}>acme-platform</Chip>
              <Chip count={2}>stripe-bridge</Chip>
              <Chip count={2}>growth-api</Chip>
              <Chip count={1}>platform-docs</Chip>
              <span className="ml-2 inline-block h-[18px] w-px bg-hairline" aria-hidden="true" />
              <Chip>Status: any</Chip>
              <Chip>Author: any</Chip>
              <span className="ml-auto flex items-center gap-[5px] text-[12px] text-muted">
                Sort by updated ↓ ·{' '}
                <Kbd>J</Kbd>
                <Kbd>K</Kbd>
              </span>
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              <table className="w-full border-collapse text-left">
                <thead>
                  <tr className="border-b border-hairline">
                    <th
                      className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '340px' }}
                    >
                      Repository / branch
                    </th>
                    <th className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted">
                      Title
                    </th>
                    <th
                      className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '100px' }}
                    >
                      Status
                    </th>
                    <th
                      className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '90px' }}
                    >
                      Checks
                    </th>
                    <th
                      className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '130px' }}
                    >
                      Reviewers
                    </th>
                    <th
                      className="pb-[8px] pr-4 text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '80px' }}
                    >
                      Age
                    </th>
                    <th
                      className="pb-[8px] text-[11px] font-semibold uppercase tracking-[0.8px] text-muted"
                      style={{ width: '130px' }}
                    >
                      Gate
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {PR_ROWS.map((row) => (
                    <PRTableRow key={`${row.repo}-${row.num}`} row={row} />
                  ))}
                </tbody>
              </table>
              <div className="mt-[14px] text-[13px] text-muted">
                — No more open PRs.{' '}
                <span className="cursor-pointer font-semibold text-primary">
                  Show 24 merged this week →
                </span>
              </div>
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
