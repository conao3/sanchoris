import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Empty-state heading section ─────────────────────────────────────────────

function W1Header() {
  return (
    <div className="mb-4 flex items-start gap-[14px]">
      <div className="flex h-12 w-12 flex-shrink-0 items-center justify-center rounded-[10px] border border-dashed border-hairline text-muted-soft">
        <svg viewBox="0 0 18 18" width="22" height="22" fill="none" stroke="currentColor" strokeWidth="1.4">
          <circle cx="4" cy="4" r="2" />
          <circle cx="14" cy="4" r="2" />
          <circle cx="9" cy="14" r="2" />
          <path d="M4.5 5.5l4 7M13.5 5.5l-4 7" />
        </svg>
      </div>
      <div>
        <h2 className="mb-1 font-serif text-[26px] font-medium leading-[1.15] tracking-[-0.5px] text-ink" style={{ margin: '0 0 4px' }}>
          No workflows yet.
        </h2>
        <p className="m-0 max-w-[580px] text-[13px] leading-[1.5] text-muted">
          A workflow defines the{' '}
          <strong className="font-semibold text-ink">state machine</strong> every Task is dispatched
          through &mdash; like AWS Step Functions, but tracker-driven and human-on-the-loop.
        </p>
      </div>
    </div>
  );
}

// ─── Template cards ───────────────────────────────────────────────────────────

type CardDef = {
  primary?: boolean;
  rec?: string;
  label: string;
  title: string;
  desc: string;
  meta: React.ReactNode;
  btnVariant: 'primary' | 'secondary';
  btnLabel: string;
};

const CARDS: CardDef[] = [
  {
    primary: true,
    rec: 'Recommended',
    label: '3 sec import',
    title: 'Import Symphony WORKFLOW.md',
    desc: 'Detected a Symphony spec at the repo root. Sanchoris will translate it into a visual workflow you can edit.',
    meta: (
      <>
        <span className="font-mono">repo/WORKFLOW.md</span>
        <span className="inline-block h-2 w-2 rounded-full bg-accent-teal" />
        <span>
          <strong className="font-semibold text-ink">9 nodes</strong> detected
        </span>
      </>
    ),
    btnVariant: 'primary',
    btnLabel: 'Import →',
  },
  {
    label: 'Template',
    title: 'Linear → GitHub delivery',
    desc: 'The default that ships with Sanchoris. Source from Linear, deliver as PR, pause on a merge-approval gate.',
    meta: <>10 nodes · Linear · GitHub · merge gate</>,
    btnVariant: 'secondary',
    btnLabel: 'Use template',
  },
  {
    label: 'Template',
    title: 'GitHub PR review loop',
    desc: 'Review every PR with an agent before a human looks. Output: PR comments + a verdict, never a merge.',
    meta: <>6 nodes · GitHub event · review · comment</>,
    btnVariant: 'secondary',
    btnLabel: 'Use template',
  },
];

function W1Card({ card }: { card: CardDef }) {
  const baseCard = 'flex flex-col rounded-[10px] border p-[14px]';
  const cardClass = card.primary
    ? `${baseCard} border-[rgba(204,120,92,0.45)] bg-surface-soft`
    : `${baseCard} border-hairline bg-canvas`;

  return (
    <div className={cardClass}>
      <div className="mb-[10px] flex items-center justify-between gap-2">
        {card.rec ? (
          <span className="rounded-[4px] bg-primary px-[7px] py-[3px] text-[10px] font-bold uppercase tracking-[1.2px] text-white">
            {card.rec}
          </span>
        ) : (
          <span />
        )}
        <span className="text-[10.5px] font-semibold uppercase tracking-[1.2px] text-muted">
          {card.label}
        </span>
      </div>
      <h3 className="mb-[6px] font-serif text-[18px] font-medium leading-[1.25] tracking-[-0.2px] text-ink" style={{ margin: '0 0 6px' }}>
        {card.title}
      </h3>
      <p className="mb-[10px] flex-1 text-[12px] leading-[1.5] text-muted" style={{ margin: '0 0 10px' }}>
        {card.desc}
      </p>
      <div
        className={`mb-3 flex items-center gap-2 rounded-[6px] border border-hairline-soft px-2 py-[5px] text-[11px] text-body ${card.primary ? 'bg-[rgba(255,255,255,0.7)]' : 'bg-surface-soft'}`}
      >
        {card.meta}
      </div>
      <div>
        <Btn variant={card.btnVariant} size="sm">
          {card.btnLabel}
        </Btn>
      </div>
    </div>
  );
}

// ─── Right rail ───────────────────────────────────────────────────────────────

function W1Rail() {
  return (
    <aside
      className="w-[260px] flex-shrink-0 border-l border-hairline bg-surface-soft px-5 py-[22px]"
    >
      <h4 className="mb-[10px] font-serif text-[17px] font-medium leading-[1.2] tracking-[-0.2px] text-ink" style={{ margin: '0 0 10px' }}>
        Why workflows?
      </h4>
      <ul className="m-0 list-none p-0 text-[12.5px] leading-[1.55] text-body">
        <li className="border-b border-dashed border-hairline-soft py-[9px]">
          <strong className="font-semibold text-ink">Versioned dispatch.</strong> Every change ships
          as a new version. Roll back any run to any version, anytime.
        </li>
        <li className="border-b border-dashed border-hairline-soft py-[9px]">
          <strong className="font-semibold text-ink">Policy-driven gates.</strong> Approvals are
          part of the graph, not afterthoughts grafted onto a CI job.
        </li>
        <li className="py-[9px]">
          <strong className="font-semibold text-ink">Visible end-to-end.</strong> Operators can see
          what the agent will do before it does it &mdash; and after.
        </li>
      </ul>
      <div className="mt-[14px] cursor-pointer text-[12px] text-primary">
        Read the workflow guide →
      </div>
      <div className="mt-[18px] border-t border-hairline-soft pt-[14px] text-[11px] leading-[1.5] text-muted-soft">
        <strong className="mb-1 block font-semibold text-muted">No runs to show.</strong>
        A run will appear here the moment the first task arrives through a configured channel.
      </div>
    </aside>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export function WorkflowsPage() {
  return (
    <AppShell
      rail={
        <Rail
          items={WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'workflows' }))}
          bottomItems={ACCOUNT_ITEMS}
        />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', 'Workflows']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Workflows{' '}
                <span className="ml-2 font-sans text-[18px] font-normal text-muted">0</span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Versioned dispatch logic · turns every incoming Task into a run ·{' '}
                <strong className="font-semibold text-body-strong">0 workflows on this project</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Browse marketplace</Btn>
              <Btn variant="primary">+ New workflow</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex min-h-0 flex-1" style={{ padding: 0, overflow: 'hidden' }}>
            {/* Main column */}
            <div className="flex min-w-0 flex-1 flex-col overflow-auto px-7 py-[22px]">
              <W1Header />

              {/* 3-column card grid */}
              <div className="mb-4 grid gap-3" style={{ gridTemplateColumns: '1.1fr 1fr 1fr' }}>
                {CARDS.map((card) => (
                  <W1Card key={card.title} card={card} />
                ))}
              </div>

              <a className="cursor-pointer py-1 text-[13px] text-primary">
                or start from blank canvas →
              </a>

              <div className="mt-auto flex items-center gap-[14px] border-t border-hairline-soft pt-4 text-[11.5px] text-muted-soft">
                <span>
                  Already have a Symphony <span className="font-mono">WORKFLOW.md</span> in another
                  repo?
                </span>
                <a className="cursor-pointer font-medium text-primary">Browse repos →</a>
                <span className="ml-auto font-mono text-[10.5px]">
                  project: acme-platform · workflows: 0
                </span>
              </div>
            </div>

            {/* Right rail */}
            <W1Rail />
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
