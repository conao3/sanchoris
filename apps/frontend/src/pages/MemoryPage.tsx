import { useState } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, Crumbs } from '../components/shell/Screen';
import { Btn } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── KTree ────────────────────────────────────────────────────────────────────

type TreeGroup = {
  label: string;
  leaves: string[];
  expanded: boolean;
  selectedLeaf?: string;
};

function KTree() {
  const [groups, setGroups] = useState<TreeGroup[]>([
    {
      label: 'Global',
      expanded: true,
      leaves: ['Org tone & voice', 'Security baselines', 'PII handling'],
    },
    {
      label: 'Program · delivery-gateway',
      expanded: true,
      leaves: ['Delivery cadence', 'Naming conventions'],
    },
    {
      label: 'Project · acme-platform',
      expanded: true,
      leaves: [
        'Stack overview',
        'Auth + SSO behaviour',
        'Linear parent rollup pitfall',
        'Code conventions',
        'Runbook: Stripe webhook',
      ],
      selectedLeaf: 'Linear parent rollup pitfall',
    },
    {
      label: 'Window · runs',
      expanded: false,
      leaves: [],
    },
    {
      label: 'Task · CON-1247',
      expanded: false,
      leaves: [],
    },
  ]);

  function toggle(idx: number) {
    setGroups((prev) =>
      prev.map((g, i) => (i === idx ? { ...g, expanded: !g.expanded } : g)),
    );
  }

  return (
    <div className="overflow-auto border-r border-hairline bg-surface-soft py-[18px]">
      <div className="px-[18px] pb-1 pt-[10px] text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted-soft">
        Scopes
      </div>
      {groups.map((group, gi) => (
        <div key={group.label}>
          <div
            className="flex cursor-pointer items-center gap-[7px] px-[18px] py-[6px] text-[13px] font-medium text-ink"
            onClick={() => toggle(gi)}
          >
            <span className="text-[10px] text-muted-soft">
              {group.expanded ? '▾' : '▸'}
            </span>
            {group.label}
          </div>
          {group.expanded &&
            group.leaves.map((leaf) => {
              const isSelected = leaf === group.selectedLeaf;
              return (
                <div
                  key={leaf}
                  className={`cursor-pointer text-[12.5px] ${
                    isSelected
                      ? 'border-l-[3px] border-primary bg-surface-cream-strong pl-[33px] pr-[18px] py-[5px] font-medium text-ink'
                      : 'py-[5px] pl-[36px] pr-[18px] text-body hover:bg-canvas'
                  }`}
                >
                  {leaf}
                </div>
              );
            })}
        </div>
      ))}
    </div>
  );
}

// ─── KDoc ─────────────────────────────────────────────────────────────────────

function Avatar({ initials, className }: { initials: string; className?: string }) {
  return (
    <span
      className={`inline-flex h-[18px] w-[18px] flex-shrink-0 items-center justify-center rounded-full bg-surface-card text-[9px] font-semibold text-ink ${className ?? ''}`}
    >
      {initials}
    </span>
  );
}

function KDoc() {
  return (
    <div className="overflow-auto p-[22px_28px]">
      <div className="mb-2 font-mono text-[12px] text-muted-soft">
        project/acme-platform/linear-parent-rollup-pitfall.md
      </div>
      <h2 className="m-0 mb-[6px] font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.5px] text-ink">
        Linear parent rollup pitfall
      </h2>
      <div className="mb-4 flex items-center gap-3 text-[11.5px] text-muted">
        <span>
          Edited <strong className="text-ink">2026-05-24 13:08 JST</strong>
        </span>
        <span>·</span>
        <span className="flex items-center gap-[5px]">
          by <Avatar initials="KO" className="-mb-[3px] ml-1" /> kanagawa-ops
        </span>
        <span>·</span>
        <span>
          read by agents <strong className="text-ink">47x</strong> last 7d
        </span>
      </div>

      <p className="mb-[9px] text-[13px] leading-[1.65] text-body">
        When tickets arrive from Linear with a{' '}
        <code className="rounded-[4px] bg-surface-soft px-[6px] py-[1px] font-mono text-[11.5px] text-ink">
          parent
        </code>{' '}
        set, the gateway must <strong>not</strong> treat the child as an
        independent task. Always check the parent's status first and roll up
        children that look like duplicates.
      </p>

      <h3 className="mb-[6px] mt-[18px] font-serif text-[20px] font-medium leading-[1.2] tracking-[-0.3px] text-ink">
        The pitfall
      </h3>
      <p className="mb-[9px] text-[13px] leading-[1.65] text-body">
        If we naively run{' '}
        <code className="rounded-[4px] bg-surface-soft px-[6px] py-[1px] font-mono text-[11.5px] text-ink">
          delivery/default
        </code>{' '}
        on every Linear webhook, we end up doing the same work multiple times
        when a customer files three near-identical bug reports that share a
        parent like{' '}
        <code className="rounded-[4px] bg-surface-soft px-[6px] py-[1px] font-mono text-[11.5px] text-ink">
          ENG-1182
        </code>
        . Worse, separate runs open separate PRs that step on each other.
      </p>

      <h3 className="mb-[6px] mt-[18px] font-serif text-[20px] font-medium leading-[1.2] tracking-[-0.3px] text-ink">
        Detection
      </h3>
      <pre className="my-2 mb-3 overflow-hidden rounded-[8px] bg-surface-dark p-[12px_14px] font-mono text-[11.5px] leading-[1.55] text-on-dark">
        <span className="text-muted-soft">{'// in normalize step'}</span>
        {'\n'}
        <span className="text-[#c8e1d6]">{'if'}</span>
        {' (ticket.parent && ticket.parent.state !== '}
        <span className="text-accent-amber">{'"done"'}</span>
        {') {\n  '}
        <span className="text-[#c8e1d6]">const</span>
        {' siblings = '}
        <span className="text-[#c8e1d6]">await</span>
        {' linear.list({ parent: ticket.parent.id });\n  '}
        <span className="text-[#c8e1d6]">if</span>
        {' (rollupSimilarity(siblings) > '}
        <span className="text-accent-amber">{'0.82'}</span>
        {') {\n    '}
        <span className="text-[#c8e1d6]">return</span>
        {' { rollupInto: ticket.parent.id, dedupe: siblings };\n  }\n}'}
      </pre>

      <h3 className="mb-[6px] mt-[18px] font-serif text-[20px] font-medium leading-[1.2] tracking-[-0.3px] text-ink">
        What &ldquo;rollup&rdquo; means here
      </h3>
      <ul className="mb-[10px] mt-[6px] pl-5 text-[13px] leading-[1.6] text-body">
        <li>Pause the child run · mark as dup of parent in Linear</li>
        <li>
          Append child&apos;s repro / context to the parent&apos;s workpad
        </li>
        <li>
          Continue executing under the <strong>parent</strong>&apos;s task
          session only
        </li>
      </ul>

      <h3 className="mb-[6px] mt-[18px] font-serif text-[20px] font-medium leading-[1.2] tracking-[-0.3px] text-ink">
        Known cases
      </h3>
      <ul className="mb-[10px] mt-[6px] pl-5 text-[13px] leading-[1.6] text-body">
        <li>
          <strong>CON-1250</strong> with siblings ENG-1184 / ENG-1185 (bulk CSV
          import)
        </li>
        <li>
          <strong>ENG-1182</strong> — original rollup that motivated this note
        </li>
      </ul>
    </div>
  );
}

// ─── KSide ────────────────────────────────────────────────────────────────────

function KSide() {
  return (
    <div className="overflow-auto border-l border-hairline bg-surface-soft p-[18px_16px]">
      {/* Backlinks */}
      <div>
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Backlinks · 8
        </h5>
        {[
          { id: 'WF', label: <>delivery/default · <span className="font-mono">normalize</span> step</> },
          { id: 'WF', label: <>hotfix · <span className="font-mono">normalize</span> step</> },
          { id: 'RB', label: <>Runbook: triage Linear bursts</> },
          { id: 'MN', label: <>Code conventions · on Linear</> },
          { id: 'TK', label: <>CON-1250 (active)</> },
          { id: 'TK', label: <>CON-1230</> },
        ].map((bk, i) => (
          <div
            key={i}
            className="flex items-baseline gap-[7px] border-b border-hairline-soft py-[6px] text-[12px] text-body last:border-b-0"
          >
            <span className="flex-shrink-0 font-mono text-[10.5px] text-muted-soft">
              {bk.id}
            </span>
            <span>{bk.label}</span>
          </div>
        ))}
      </div>

      {/* Recently edited */}
      <div className="mt-[18px]">
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Recently edited
        </h5>
        {[
          { id: '13:08', label: <>this note · <strong className="text-ink">+ 4 lines</strong></> },
          { id: '11:48', label: <>Auth + SSO behaviour</> },
          { id: '09:33', label: <>Stack overview</> },
          { id: "y'day", label: <>Runbook: Stripe webhook</> },
        ].map((bk, i) => (
          <div
            key={i}
            className="flex items-baseline gap-[7px] border-b border-hairline-soft py-[6px] text-[12px] text-body last:border-b-0"
          >
            <span className="flex-shrink-0 font-mono text-[10.5px] text-muted-soft">
              {bk.id}
            </span>
            <span>{bk.label}</span>
          </div>
        ))}
      </div>

      {/* Auto-pinned */}
      <div className="mt-[18px]">
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
          Auto-pinned by agents
        </h5>
        {[
          { label: <>Linear parent rollup pitfall <span className="text-[11px] text-muted">(this)</span></> },
          { label: <>Auth + SSO behaviour</> },
          { label: <>Code conventions</> },
        ].map((bk, i) => (
          <div
            key={i}
            className="flex items-baseline gap-[7px] border-b border-hairline-soft py-[6px] text-[12px] text-body last:border-b-0"
          >
            <span className="flex-shrink-0 font-mono text-[10.5px] text-muted-soft">
              ★
            </span>
            <span>{bk.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Page ──────────────────────────────────────────────────────────────────────

const memoryRailItems = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'memory',
}));

export function MemoryPage() {
  return (
    <AppShell
      rail={<Rail items={memoryRailItems} bottomItems={ACCOUNT_ITEMS} />}
    >
      <Screen>
        <ScreenHead>
          <Crumbs
            items={[
              'acme-org',
              'Memory',
              'acme-platform',
              'Linear parent rollup pitfall',
            ]}
          />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Memory
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Context the gateway re-reads before every run · 5 scopes ·{' '}
                <strong className="font-semibold text-body-strong">
                  74 notes total
                </strong>{' '}
                · auto-pinned by usage
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Backlinks (8)</Btn>
              <Btn variant="primary">Open in editor ↗</Btn>
            </div>
          </div>
        </ScreenHead>
        <div className="grid min-h-0 flex-1 overflow-hidden" style={{ gridTemplateColumns: '280px 1fr 280px' }}>
          <KTree />
          <KDoc />
          <KSide />
        </div>
      </Screen>
    </AppShell>
  );
}
