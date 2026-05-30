import type { ReactNode } from 'react';

type InboxItem = {
  id: string;
  kind: 'PR' | 'RUN' | 'ISSUE';
  priority: 'P0' | 'P1' | 'P2';
  title: string;
  project: string;
  age: string;
  active?: boolean;
};

const items: InboxItem[] = [
  { id: '1247', kind: 'PR', priority: 'P0', title: 'Add OAuth login for vendor portal', project: 'vendor-portal', age: '2h', active: true },
  { id: '1246', kind: 'PR', priority: 'P0', title: 'Webhook sig mismatch /api/stripe', project: 'payments-core', age: '3h' },
  { id: 'R-9142', kind: 'RUN', priority: 'P1', title: 'R-9142 worktree test failed', project: 'vendor-portal', age: '1h' },
  { id: '1245', kind: 'PR', priority: 'P1', title: 'Refactor billing pipeline', project: 'payments-core', age: '5h' },
  { id: 'I-512', kind: 'ISSUE', priority: 'P1', title: 'Migrate logger to tracing', project: 'platform-shared', age: '8h' },
  { id: 'I-509', kind: 'ISSUE', priority: 'P2', title: 'Add dark mode toggle', project: 'vendor-portal', age: '1d' },
  { id: 'I-508', kind: 'ISSUE', priority: 'P2', title: 'Bump rust 1.85', project: 'platform-shared', age: '1d' },
  { id: 'I-505', kind: 'ISSUE', priority: 'P2', title: 'Improve dashboard load time', project: 'ops-console', age: '2d' },
  { id: 'I-501', kind: 'ISSUE', priority: 'P2', title: 'Investigate flaky cron job', project: 'ops-console', age: '2d' },
];

export function InboxPage() {
  return (
    <main className="flex min-h-screen w-full bg-canvas font-sans text-ink">
      <LeftNav />
      <InboxColumn />
      <DetailPane />
    </main>
  );
}

function LeftNav() {
  return (
    <aside className="flex w-60 flex-none flex-col gap-6 bg-surface-soft px-5 py-6">
      <header className="flex items-center gap-2.5">
        <span className="inline-flex size-[18px] items-center justify-center bg-surface-dark">
          <span className="size-2 bg-canvas" aria-hidden="true" />
        </span>
        <span className="font-serif text-h3 font-medium leading-tight text-ink">
          sanchoris<span className="text-primary">.</span>
        </span>
      </header>

      <button
        type="button"
        className="flex items-center gap-3 rounded-md border border-hairline bg-canvas px-3 py-2.5 text-left transition-colors hover:border-primary/40"
      >
        <span className="size-[22px] flex-none bg-surface-dark" aria-hidden="true" />
        <span className="flex min-w-0 flex-col">
          <span className="truncate text-sm font-medium text-ink">acme-org</span>
          <span className="text-xs text-muted-soft">workspace</span>
        </span>
      </button>

      <nav className="flex flex-col gap-1">
        <NavItem label="Inbox" count={9} active dotColor="primary" />
        <NavItem label="Projects" count={5} dotColor="muted-soft" />
        <NavItem label="Gates" count={3} dotColor="muted-soft" />
        <NavItem label="Runs" count={2} dotColor="muted-soft" />
        <NavItem label="Settings" dotColor="muted-soft" />
      </nav>

      <footer className="mt-auto flex items-center gap-2.5">
        <span className="size-7 flex-none rounded-full bg-accent-teal" aria-hidden="true" />
        <span className="flex flex-col leading-tight">
          <span className="text-sm font-medium text-ink">Akira Tanaka</span>
          <span className="text-xs text-muted-soft">online</span>
        </span>
      </footer>
    </aside>
  );
}

function NavItem({
  label,
  count,
  active = false,
  dotColor,
}: {
  label: string;
  count?: number;
  active?: boolean;
  dotColor: 'primary' | 'muted-soft';
}) {
  const dotClass = dotColor === 'primary' ? 'bg-primary' : 'bg-muted-soft';
  return (
    <button
      type="button"
      className={`flex h-9 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium transition-colors ${
        active ? 'bg-surface-cream-strong text-ink' : 'text-body hover:bg-surface-cream-strong/60'
      }`}
    >
      <span className={`size-1.5 rounded-full ${dotClass}`} aria-hidden="true" />
      <span className="flex-1 text-left">{label}</span>
      {count !== undefined ? <span className="font-mono text-xs text-muted-soft">{count}</span> : null}
    </button>
  );
}

function InboxColumn() {
  return (
    <section className="flex w-[480px] flex-none flex-col border-x border-hairline bg-canvas">
      <header className="flex flex-col gap-3.5 px-6 pt-7 pb-3">
        <div className="flex items-center justify-between">
          <h1 className="font-serif text-h3 font-medium text-ink">Inbox</h1>
          <span className="inline-flex h-[22px] min-w-9 items-center justify-center rounded-pill bg-surface-cream-strong px-2 font-mono text-xs text-muted">
            9
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <FilterChip label="All" active />
          <FilterChip label="PRs" />
          <FilterChip label="Runs" />
          <FilterChip label="Issues" />
        </div>
      </header>

      <div className="px-6 pb-3">
        <div className="flex h-9 items-center gap-2.5 rounded-md border border-hairline bg-surface-soft px-3">
          <span className="font-mono text-xs uppercase tracking-wider text-muted-soft">search</span>
          <span className="text-sm text-muted-soft">Search inbox, project:, type:run...</span>
        </div>
      </div>

      <ul className="flex flex-1 flex-col gap-1.5 overflow-y-auto px-6 pb-6">
        {items.map((item) => (
          <ListItem key={item.id} item={item} />
        ))}
      </ul>
    </section>
  );
}

function FilterChip({ label, active = false }: { label: string; active?: boolean }) {
  return (
    <button
      type="button"
      className={`h-6 rounded-pill px-3 font-mono text-xs uppercase tracking-wider transition-colors ${
        active ? 'bg-surface-dark text-on-dark' : 'bg-surface-cream-strong text-muted hover:bg-surface-card'
      }`}
    >
      {label}
    </button>
  );
}

function ListItem({ item }: { item: InboxItem }) {
  const dotClass =
    item.priority === 'P0' ? 'bg-error' : item.priority === 'P1' ? 'bg-warning' : 'bg-muted-soft';
  return (
    <li>
      <button
        type="button"
        className={`flex w-full items-start gap-3 rounded-md border px-3.5 py-3 text-left transition-colors ${
          item.active
            ? 'border-primary/60 bg-surface-cream-strong shadow-card-soft'
            : 'border-transparent bg-transparent hover:border-hairline-soft hover:bg-surface-soft/60'
        }`}
      >
        <span className={`mt-1.5 size-2 flex-none rounded-full ${dotClass}`} aria-hidden="true" />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="truncate text-md font-medium text-ink">{item.title}</span>
          <span className="flex items-center gap-2 text-xs text-muted">
            <span className="font-mono uppercase tracking-wider text-muted-soft">{item.kind}</span>
            <Dot />
            <span>{item.project}</span>
            <Dot />
            <span>{item.age}</span>
          </span>
        </span>
      </button>
    </li>
  );
}

function Dot() {
  return <span className="text-muted-soft">·</span>;
}

function DetailPane() {
  return (
    <section className="flex flex-1 flex-col bg-surface-soft">
      <DetailHeader />
      <Tabs />
      <DetailBody />
    </section>
  );
}

function DetailHeader() {
  return (
    <header className="flex flex-col gap-4 border-b border-hairline px-7 pt-7 pb-6">
      <div className="flex items-center gap-2 text-xs">
        <span className="inline-flex h-[22px] items-center rounded-pill bg-accent-teal/20 px-2.5 font-mono uppercase tracking-wider text-accent-teal">
          PR · P0
        </span>
        <span className="text-body">conao3/vendor-portal</span>
        <Dot />
        <span className="text-muted">opened 2h ago</span>
        <span className="ml-auto font-mono text-xs text-muted-soft">
          #1247 &nbsp; <kbd className="font-mono">⌘+Enter</kbd> to approve
        </span>
      </div>

      <h2 className="font-serif text-h2 font-medium leading-heading text-ink">
        Add OAuth login for vendor portal
      </h2>

      <dl className="flex items-center gap-2 text-xs">
        <dt className="font-mono uppercase tracking-wider text-muted-soft">agent</dt>
        <dd className="text-body">claude-code · seer-v3</dd>
        <span className="mx-2 h-3 w-px bg-hairline" aria-hidden="true" />
        <dt className="font-mono uppercase tracking-wider text-muted-soft">branch</dt>
        <dd className="font-mono text-body">feature/oauth-vendor-portal</dd>
      </dl>

      <div className="flex items-center gap-2">
        <button
          type="button"
          className="h-9 rounded-md bg-primary px-5 text-sm font-medium text-on-primary transition-colors hover:bg-primary-active"
        >
          Approve
        </button>
        <button
          type="button"
          className="h-9 rounded-md border border-hairline bg-canvas px-5 text-sm font-medium text-body transition-colors hover:border-primary/40"
        >
          Request changes
        </button>
        <button
          type="button"
          className="h-9 rounded-md px-3 text-sm font-medium text-muted transition-colors hover:text-ink"
        >
          Skip →
        </button>
      </div>
    </header>
  );
}

function Tabs() {
  return (
    <nav className="flex items-end gap-1 border-b border-hairline px-7">
      <Tab label="Diff (24)" active />
      <Tab label="Reason" />
      <Tab label="Tests ✓" />
      <Tab label="Memory" />
    </nav>
  );
}

function Tab({ label, active = false }: { label: string; active?: boolean }) {
  return (
    <button
      type="button"
      className={`h-10 border-b-2 px-4 text-sm font-medium transition-colors ${
        active ? 'border-primary text-ink' : 'border-transparent text-muted hover:text-body'
      }`}
    >
      {label}
    </button>
  );
}

function DetailBody() {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-y-auto px-7 py-6">
      <FileBox />
      <SummaryCard />
    </div>
  );
}

function FileBox() {
  return (
    <article className="overflow-hidden rounded-card border border-hairline bg-canvas">
      <header className="flex items-center gap-3 border-b border-hairline px-3.5 py-2.5">
        <span className="inline-flex size-5 items-center justify-center rounded-sm bg-accent-amber/20 font-mono text-xs font-medium text-accent-amber">
          M
        </span>
        <span className="font-mono text-sm text-body">src/auth/oauth_vendor.rs</span>
        <span className="ml-auto flex items-center gap-3 font-mono text-xs">
          <span className="text-success">+38</span>
          <span className="text-error">−4</span>
        </span>
      </header>
      <pre className="overflow-x-auto px-3.5 py-3 font-mono text-xs leading-[1.55] text-body">
        <CodeLine>{'  use anyhow::Result;'}</CodeLine>
        <CodeLine>{'  use axum::extract::{Query, State};'}</CodeLine>
        <CodeLine kind="add">{'+ use openidconnect::{AuthorizationCode, Nonce};'}</CodeLine>
        <CodeLine kind="add">{'+ use crate::auth::session::Session;'}</CodeLine>
        <CodeLine>{' '}</CodeLine>
        <CodeLine>{'  pub async fn callback('}</CodeLine>
        <CodeLine kind="del">{'-     Query(params): Query<HashMap<String, String>>,'}</CodeLine>
        <CodeLine kind="add">{'+     Query(params): Query<OAuthCallback>,'}</CodeLine>
        <CodeLine>{'      State(state): State<AppState>,'}</CodeLine>
        <CodeLine>{'  ) -> Result<Redirect> {'}</CodeLine>
        <CodeLine kind="add">{'+     let nonce = Nonce::new_random();'}</CodeLine>
        <CodeLine kind="add">{'+     let code = AuthorizationCode::new(params.code);'}</CodeLine>
        <CodeLine>{'      // verify state and exchange token'}</CodeLine>
      </pre>
    </article>
  );
}

function CodeLine({ kind, children }: { kind?: 'add' | 'del'; children: ReactNode }) {
  const bg = kind === 'add' ? 'bg-success/10' : kind === 'del' ? 'bg-error/10' : '';
  return <div className={`-mx-3.5 px-3.5 ${bg}`}>{children}</div>;
}

function SummaryCard() {
  return (
    <article className="flex flex-col gap-3 rounded-card border border-hairline bg-canvas px-4 py-4">
      <header className="flex items-center gap-3 text-xs">
        <span className="font-mono uppercase tracking-wider text-muted-soft">Agent note</span>
        <span className="font-mono text-muted">claude-code · seer-v3</span>
      </header>
      <p className="text-sm leading-body text-body">
        Implements OAuth callback for vendor portal. Tests added in tests/oauth_vendor_test.rs cover
        the happy path and the 3 failure modes (invalid nonce, expired code, mismatched state). One
        TODO: rate limit on nonce verification — flagged for review.
      </p>
    </article>
  );
}
