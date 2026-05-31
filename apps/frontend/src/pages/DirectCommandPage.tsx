import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Pill } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

const railItems = WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'queue' }));

// ─── Dark Chip (page-local, dark surface variant) ────────────────────────────

function DarkChip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex h-7 cursor-pointer items-center gap-[6px] rounded-[5px] border border-[rgba(255,255,255,0.08)] bg-surface-dark-soft px-[9px] font-mono text-[11px] text-on-dark">
      {children}
    </span>
  );
}

// ─── REPL result table (page-local) ─────────────────────────────────────────

type ReplTableRow = {
  cells: Array<{ text: string; tone?: 'teal' | 'error' | 'success' | 'none' }>;
};

function ReplTable({
  headers,
  rows,
}: {
  headers: string[];
  rows: ReplTableRow[];
}) {
  return (
    <table style={{ borderCollapse: 'collapse', margin: '4px 0' }}>
      <thead>
        <tr>
          {headers.map((h) => (
            <th
              key={h}
              style={{
                padding: '0 12px 4px 0',
                fontFamily: 'var(--font-mono)',
                fontSize: '10.5px',
                textTransform: 'uppercase',
                letterSpacing: '1px',
                color: '#a09d96',
                fontWeight: 500,
                textAlign: 'left',
                borderBottom: '1px solid rgba(255,255,255,0.1)',
              }}
            >
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, ri) => (
          <tr key={ri}>
            {row.cells.map((cell, ci) => (
              <td
                key={ci}
                style={{
                  padding: '2px 12px 2px 0',
                  fontFamily: 'var(--font-mono)',
                  fontSize: '12px',
                  textAlign: 'left',
                  color:
                    cell.tone === 'teal'
                      ? '#c8e1d6'
                      : cell.tone === 'error'
                        ? '#c64545'
                        : cell.tone === 'success'
                          ? '#5db872'
                          : '#a09d96',
                }}
              >
                {cell.text}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ─── REPL command block ──────────────────────────────────────────────────────

type CmdPart =
  | { kind: 'plain'; text: string }
  | { kind: 'flag'; text: string }
  | { kind: 'val'; text: string };

type ReplResultItem =
  | { type: 'inline'; parts: Array<{ text: string; tone?: 'teal' | 'error' | 'success' | 'amber' | 'none' }> }
  | { type: 'table'; headers: string[]; rows: ReplTableRow[] }
  | { type: 'code'; lines: string[] };

type CmdEntry = {
  parts: CmdPart[];
  time: string;
  titleParts?: Array<{ text: string; tone?: 'default' | 'ok' | 'err' }>;
  results: ReplResultItem[];
};

function CmdBlock({ entry }: { entry: CmdEntry }) {
  return (
    <div className="mb-[14px]">
      {/* Input line */}
      <div className="flex items-baseline gap-0 font-mono text-[12.5px] leading-[1.6]">
        <span className="mr-2 font-bold text-accent-teal">san›</span>
        <span className="text-on-dark">
          {entry.parts.map((part, i) =>
            part.kind === 'flag' ? (
              <span key={i} className="text-accent-amber">
                {part.text}
              </span>
            ) : part.kind === 'val' ? (
              <span key={i} style={{ color: '#c8e1d6' }}>
                {part.text}
              </span>
            ) : (
              <span key={i}>{part.text}</span>
            ),
          )}
        </span>
        <span className="ml-2 text-[10.5px] text-muted-soft">{entry.time}</span>
      </div>
      {/* Result */}
      <div
        className="mt-1 pl-4 font-mono text-[12.5px] leading-[1.6] text-on-dark-soft"
        style={{ borderLeft: '2px solid rgba(255,255,255,0.08)' }}
      >
        {entry.titleParts && (
          <div className="mb-1 font-sans text-[12px] font-semibold uppercase tracking-[1.2px] text-on-dark">
            {entry.titleParts.map((p, i) =>
              p.tone === 'ok' ? (
                <span key={i} className="text-success">
                  {p.text}
                </span>
              ) : p.tone === 'err' ? (
                <span key={i} className="text-error">
                  {p.text}
                </span>
              ) : (
                <span key={i}>{p.text}</span>
              ),
            )}
          </div>
        )}
        {entry.results.map((res, ri) =>
          res.type === 'table' ? (
            <ReplTable key={ri} headers={res.headers} rows={res.rows} />
          ) : res.type === 'code' ? (
            <div key={ri} className="text-on-dark-soft">
              {res.lines.map((line, li) => (
                <div key={li} dangerouslySetInnerHTML={{ __html: line }} />
              ))}
            </div>
          ) : (
            <span key={ri}>
              {res.parts.map((p, pi) =>
                p.tone === 'teal' ? (
                  <span key={pi} style={{ color: '#c8e1d6' }}>
                    {p.text}
                  </span>
                ) : p.tone === 'error' ? (
                  <span key={pi} className="text-error">
                    {p.text}
                  </span>
                ) : p.tone === 'success' ? (
                  <span key={pi} className="text-success">
                    {p.text}
                  </span>
                ) : p.tone === 'amber' ? (
                  <span key={pi} className="text-accent-amber">
                    {p.text}
                  </span>
                ) : (
                  <span key={pi}>{p.text}</span>
                ),
              )}
            </span>
          ),
        )}
      </div>
    </div>
  );
}

// ─── Command history data ────────────────────────────────────────────────────

const CMD_ENTRIES: CmdEntry[] = [
  {
    parts: [{ kind: 'plain', text: 'queue list --project acme-platform --priority p0 --json=false' }],
    time: '13:14 JST',
    titleParts: [{ text: '2 tasks' }],
    results: [
      {
        type: 'table',
        headers: ['id', 'title', 'age', 'status'],
        rows: [
          {
            cells: [
              { text: 'CON-1249', tone: 'teal' },
              { text: 'Webhook signature mismatch on /api/stripe', tone: 'none' },
              { text: '52m', tone: 'none' },
              { text: 'running', tone: 'error' },
            ],
          },
          {
            cells: [
              { text: 'CON-1247', tone: 'teal' },
              { text: 'Add OAuth login for vendor portal', tone: 'none' },
              { text: '12m', tone: 'none' },
              { text: 'running', tone: 'success' },
            ],
          },
        ],
      },
    ],
  },
  {
    parts: [
      { kind: 'plain', text: 'run start ' },
      { kind: 'flag', text: 'CON-1248' },
      { kind: 'plain', text: ' --worker ' },
      { kind: 'val', text: 'claude-code' },
      { kind: 'plain', text: ' --workflow ' },
      { kind: 'val', text: 'delivery/default' },
    ],
    time: '13:21 JST',
    titleParts: [{ text: 'run started' }],
    results: [
      {
        type: 'inline',
        parts: [
          { text: '✓ ', tone: 'success' },
          { text: 'R-9142', tone: 'teal' },
          { text: ' · workspace ' },
          { text: 'ws-growth-c812', tone: 'teal' },
          { text: ' · step ' },
          { text: 'normalize', tone: 'amber' },
          { text: ' → ' },
          { text: 'ok', tone: 'success' },
          { text: ' 0.9s · step ' },
          { text: 'prioritize', tone: 'amber' },
          { text: ' → ' },
          { text: 'ok', tone: 'success' },
          { text: ' 0.2s' },
        ],
      },
    ],
  },
  {
    parts: [{ kind: 'plain', text: 'memory show project/acme-platform/auth-sso.md --head 6' }],
    time: '13:24 JST',
    titleParts: [{ text: 'project/acme-platform/auth-sso.md' }],
    results: [
      {
        type: 'code',
        lines: [
          '# Auth &amp; SSO behaviour',
          '',
          'The web app supports two identity sources: <span style="color:#e8a55a">acme-saml</span> (employees) and <span style="color:#e8a55a">workos-oidc</span> (customers).',
          'All sessions are JWT, 60min lifetime, refreshed silently via hidden iframe.',
          'Treat redirect to <span style="color:#c8e1d6">/auth/error?code=session</span> as recoverable.',
        ],
      },
    ],
  },
  {
    parts: [{ kind: 'plain', text: 'gate list --pending --json' }],
    time: '13:31 JST',
    titleParts: [{ text: '6 pending gates' }],
    results: [
      {
        type: 'code',
        lines: [
          '[',
          '  { id: <span style="color:#c8e1d6">"g-2210"</span>, kind: <span style="color:#c8e1d6">"merge"</span>,  pr: <span style="color:#c8e1d6">482</span>, age: <span style="color:#e8a55a">"22m"</span>,  policy: <span style="color:#e8a55a">"prod-touching-files"</span> },',
          '  { id: <span style="color:#c8e1d6">"g-2208"</span>, kind: <span style="color:#c8e1d6">"deploy"</span>, env: <span style="color:#c8e1d6">"prod"</span>, age: <span style="color:#e8a55a">"1h08m"</span>, policy: <span style="color:#e8a55a">"prod-deploy-business-hours"</span> },',
          '  { id: <span style="color:#c8e1d6">"g-2207"</span>, kind: <span style="color:#c8e1d6">"secret"</span>, src: <span style="color:#c8e1d6">"vault://stripe.live"</span>, age: <span style="color:#e8a55a">"3h"</span> },',
          '  <span style="color:#e8a55a">… 3 more</span>',
          ']',
        ],
      },
    ],
  },
  {
    parts: [
      { kind: 'plain', text: 'gate approve ' },
      { kind: 'flag', text: 'g-2210' },
      { kind: 'plain', text: ' --note ' },
      { kind: 'val', text: '"reviewed; merge clean"' },
    ],
    time: '13:38 JST',
    titleParts: [{ text: 'policy check' }],
    results: [
      {
        type: 'inline',
        parts: [
          { text: '✗ ', tone: 'error' },
          { text: 'requires 2 reviewers · only 1 approval present (' },
          { text: 'octocat', tone: 'teal' },
          { text: '). Need: 1 more from ' },
          { text: '@acme/code-owners', tone: 'amber' },
          { text: '.' },
        ],
      },
    ],
  },
  {
    parts: [
      { kind: 'plain', text: 'worker status --kind ' },
      { kind: 'val', text: 'claude-code' },
    ],
    time: '13:42 JST',
    results: [
      {
        type: 'table',
        headers: ['worker', 'busy', 'idle', 'load'],
        rows: [
          {
            cells: [
              { text: 'w-cc-1', tone: 'teal' },
              { text: '1', tone: 'none' },
              { text: '0', tone: 'none' },
              { text: '88%', tone: 'error' },
            ],
          },
          {
            cells: [
              { text: 'w-cc-2', tone: 'teal' },
              { text: '0', tone: 'none' },
              { text: '1', tone: 'none' },
              { text: '8%', tone: 'success' },
            ],
          },
        ],
      },
    ],
  },
];

// ─── Page ─────────────────────────────────────────────────────────────────────

export function DirectCommandPage() {
  return (
    <AppShell rail={<Rail items={railItems} bottomItems={ACCOUNT_ITEMS} />}>
      {/* @keyframes for cursor blink — injected inline, does not touch style.css */}
      <style>{`@keyframes cblink { 50% { background: transparent; } }`}</style>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Tools', 'Direct command']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Direct command{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="info-soft">role: operator</Pill>
                  <Pill kind="default">
                    <span className="font-mono">session 04f1</span>
                  </Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Operator REPL into the gateway · session{' '}
                <span className="font-mono">04f1 · 12m active</span> ·{' '}
                <strong className="font-semibold text-body-strong">role: operator</strong> · signed
                in as kanagawa-ops
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">History</Btn>
              <Btn variant="primary">+ New session</Btn>
            </div>
          </div>
        </ScreenHead>

        <ScreenBody>
          {/* Negative margins counteract ScreenBody's padding so the dark surface fills edge-to-edge */}
          <div className="-mx-7 -mb-6 -mt-[18px] flex min-h-0 flex-1 flex-col overflow-hidden bg-surface-dark text-on-dark">

            {/* Recent history bar */}
            <div
              className="flex flex-shrink-0 flex-wrap items-center gap-[6px] px-7 py-3"
              style={{
                background: '#252320',
                borderBottom: '1px solid rgba(255,255,255,0.06)',
              }}
            >
              <span className="mr-[6px] text-[10.5px] font-semibold uppercase tracking-[1.3px] text-on-dark-soft">
                Recent
              </span>
              <DarkChip>
                <span className="text-accent-teal">queue list</span>
                {' --project acme-platform --priority p0'}
              </DarkChip>
              <DarkChip>
                <span className="text-accent-teal">run start</span>
                {' CON-1249 --worker codex'}
              </DarkChip>
              <DarkChip>
                <span className="text-accent-teal">gate list</span>
                {' --pending'}
              </DarkChip>
              <DarkChip>
                <span className="text-accent-teal">memory show</span>
                {' project/acme-platform/auth-sso.md'}
              </DarkChip>
            </div>

            {/* Scrollable command area */}
            <div className="min-h-0 flex-1 overflow-auto px-7 py-[18px]">
              {CMD_ENTRIES.map((entry, i) => (
                <CmdBlock key={i} entry={entry} />
              ))}
            </div>

            {/* Prompt section */}
            <div
              className="relative flex-shrink-0 px-7 pb-[14px] pt-[14px]"
              style={{
                background: '#1f1e1b',
                borderTop: '1px solid rgba(255,255,255,0.06)',
              }}
            >
              {/* Autocomplete dropdown */}
              <div
                className="absolute bottom-full left-7 right-[280px] overflow-hidden rounded-t-[8px] border border-[rgba(255,255,255,0.1)]"
                style={{
                  background: '#252320',
                  boxShadow: '0 -8px 28px rgba(0,0,0,0.3)',
                  marginBottom: '-1px',
                }}
              >
                {/* Selected item */}
                <div
                  className="flex items-center gap-[10px] px-[12px] py-[7px] font-mono text-[12px] text-on-dark"
                  style={{
                    background: 'rgba(204,120,92,0.15)',
                    borderLeft: '2px solid #cc785c',
                    borderBottom: '1px solid rgba(255,255,255,0.04)',
                  }}
                >
                  <span className="font-semibold text-on-dark">run logs</span>
                  <span className="text-accent-amber">&lt;run-id&gt;</span>
                  <span className="ml-auto font-sans text-[11px] text-muted-soft">
                    stream worker output
                  </span>
                  <span className="font-sans text-[10px] text-muted-soft">↵</span>
                </div>
                {/* Other items */}
                {[
                  {
                    cmd: 'run cancel',
                    arg: '<run-id>',
                    doc: 'stop and gc workspace',
                    extra: null,
                  },
                  {
                    cmd: 'run takeover',
                    arg: '<run-id>',
                    doc: 'switch to interactive',
                    extra: null,
                  },
                  {
                    cmd: 'run retry',
                    arg: '<run-id>',
                    doc: 'replay a single step',
                    extra: '--from <step>',
                  },
                ].map((item) => (
                  <div
                    key={item.cmd}
                    className="flex items-center gap-[10px] px-[14px] py-[7px] font-mono text-[12px] text-on-dark-soft"
                    style={{ borderBottom: '1px solid rgba(255,255,255,0.04)' }}
                  >
                    <span className="font-semibold text-on-dark">{item.cmd}</span>
                    <span className="text-accent-amber">{item.arg}</span>
                    {item.extra && <span className="text-on-dark-soft">{item.extra}</span>}
                    <span className="ml-auto font-sans text-[11px] text-muted-soft">{item.doc}</span>
                  </div>
                ))}
              </div>

              {/* Input row with blinking cursor */}
              <div className="flex items-center gap-2 font-mono text-[13px]">
                <span className="font-bold text-accent-teal">san›</span>
                <span className="text-on-dark">run </span>
                <span
                  className="inline-block h-4 w-2 align-[-3px] bg-primary"
                  style={{ marginLeft: '1px', animation: 'cblink 1s steps(1) infinite' }}
                />
                <span
                  className="ml-auto font-sans text-[11px] text-on-dark-soft"
                >
                  Tab autocomplete ·{' '}
                  <span
                    className="inline-flex items-center rounded-[4px] border px-[6px] py-[1.5px] font-mono text-[11px] leading-none"
                    style={{
                      background: 'rgba(255,255,255,0.06)',
                      borderColor: 'rgba(255,255,255,0.1)',
                      color: '#a09d96',
                    }}
                  >
                    ⏎
                  </span>{' '}
                  run
                </span>
              </div>
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
