import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import React from 'react';
import { Button } from 'react-aria-components';
import { createRoot } from 'react-dom/client';
import './style.css';

type HealthResponse = {
  status: string;
  service: string;
};

type TaskCard = {
  id: string;
  title: string;
  source: string;
  status: string;
  priority: string;
  worker: string;
};

type WorkflowBlock = {
  name: string;
  label: string;
  status: string;
};

type PanelMetric = {
  label: string;
  value: string;
  tone?: 'green' | 'amber' | 'blue';
};

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      staleTime: 30_000,
    },
  },
});

const tasks: TaskCard[] = [
  {
    id: 'SAN-104',
    title: 'Build MVP Web UI shell',
    source: 'Built-in chat',
    status: 'Running',
    priority: 'High',
    worker: 'Codex',
  },
  {
    id: 'SAN-097',
    title: 'Record workflow YAML draft',
    source: 'Native task',
    status: 'Review',
    priority: 'Medium',
    worker: 'Codex',
  },
  {
    id: 'SAN-088',
    title: 'Verify workspace cleanup state',
    source: 'Built-in chat',
    status: 'Queued',
    priority: 'Low',
    worker: 'Codex',
  },
];

const workflowBlocks: WorkflowBlock[] = [
  { name: 'ChatInput', label: 'Capture message', status: 'source' },
  { name: 'CreateTask', label: 'Native task', status: 'ready' },
  { name: 'CreateWorkspace', label: 'Branch + worktree', status: 'ready' },
  { name: 'RunWorker', label: 'Codex worker', status: 'active' },
  { name: 'RunVerification', label: 'Check + build', status: 'pending' },
  { name: 'Gate', label: 'Human approval', status: 'waiting' },
  { name: 'CreatePR', label: 'Open draft PR', status: 'pending' },
  { name: 'Merge', label: 'Merge after gate', status: 'locked' },
];

async function fetchHealth(): Promise<HealthResponse> {
  const response = await fetch('/api/v1/health', {
    headers: {
      Accept: 'application/json',
    },
  });

  if (!response.ok) {
    throw new Error(`Health check failed: ${response.status} ${response.statusText}`);
  }

  return response.json() as Promise<HealthResponse>;
}

function AdminHealthPage() {
  const healthQuery = useQuery({
    queryKey: ['backend', 'health'],
    queryFn: fetchHealth,
  });

  const checkedAt = healthQuery.dataUpdatedAt
    ? new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'medium',
      }).format(new Date(healthQuery.dataUpdatedAt))
    : 'Not checked yet';

  return (
    <main className="min-h-screen bg-slate-50 px-6 py-10 text-slate-950">
      <section className="mx-auto flex w-full max-w-3xl flex-col gap-6 rounded-3xl border border-slate-200 bg-white p-8 shadow-sm">
        <div className="flex flex-col gap-2">
          <p className="text-sm font-semibold uppercase tracking-[0.24em] text-blue-600">Sanchoris Admin</p>
          <h1 className="text-4xl font-bold tracking-tight text-slate-950">Backend health</h1>
          <p className="text-base leading-7 text-slate-600">
            Checks the Rust backend through <code className="rounded bg-slate-100 px-1.5 py-0.5 text-sm">/api/v1/health</code>.
          </p>
        </div>

        <dl className="grid gap-4 rounded-2xl border border-slate-200 bg-slate-50 p-5 sm:grid-cols-3">
          <StatusItem label="Status" value={healthQuery.data?.status ?? (healthQuery.isError ? 'error' : 'checking')} />
          <StatusItem label="Service" value={healthQuery.data?.service ?? 'sanchoris-backend'} />
          <StatusItem label="Last checked" value={checkedAt} />
        </dl>

        {healthQuery.isError ? (
          <div className="rounded-2xl border border-red-200 bg-red-50 p-4 text-sm text-red-700" role="alert">
            {healthQuery.error instanceof Error ? healthQuery.error.message : 'Unknown health check error'}
          </div>
        ) : null}

        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <Button
            className="rounded-full bg-blue-600 px-5 py-2.5 text-sm font-semibold text-white shadow-sm outline-none transition hover:bg-blue-700 pressed:bg-blue-800 focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-slate-300"
            isDisabled={healthQuery.isFetching}
            onPress={() => void healthQuery.refetch()}
          >
            {healthQuery.isFetching ? 'Checking...' : 'Check again'}
          </Button>
          <p className="text-sm text-slate-500">Powered by TanStack Query and React Aria Components.</p>
        </div>
      </section>
    </main>
  );
}

function StatusItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-xl bg-white p-4 shadow-sm ring-1 ring-slate-200">
      <dt className="text-xs font-semibold uppercase tracking-wide text-slate-500">{label}</dt>
      <dd className="mt-2 break-words text-lg font-semibold text-slate-950">{value}</dd>
    </div>
  );
}

function MvpShell() {
  return (
    <main className="min-h-screen bg-[#f4f1ea] text-stone-950">
      <div className="mx-auto flex w-full max-w-[1520px] flex-col gap-5 px-4 py-4 sm:px-6 lg:px-8">
        <ShellHeader />

        <section className="grid gap-5 xl:grid-cols-[360px_minmax(0,1fr)_380px]">
          <div className="flex flex-col gap-5">
            <ChatPanel />
            <TaskBoard />
          </div>

          <div className="flex min-w-0 flex-col gap-5">
            <WorkflowCanvas />
            <RunWorkspaceVerification />
          </div>

          <div className="flex flex-col gap-5">
            <ProjectProfile />
            <ReviewPanels />
          </div>
        </section>
      </div>
    </main>
  );
}

function ShellHeader() {
  return (
    <header className="flex flex-col gap-4 border-b border-stone-300/80 pb-4 lg:flex-row lg:items-center lg:justify-between">
      <div>
        <p className="text-xs font-semibold uppercase text-emerald-700">Sanchoris MVP control plane</p>
        <h1 className="mt-1 text-3xl font-semibold tracking-tight text-stone-950 sm:text-4xl">Native task delivery shell</h1>
      </div>

      <nav className="flex flex-wrap items-center gap-2 text-sm font-medium text-stone-700" aria-label="Primary sections">
        <a className="nav-pill nav-pill-active" href="#chat">Chat</a>
        <a className="nav-pill" href="#tasks">Tasks</a>
        <a className="nav-pill" href="#workflow">Workflow</a>
        <a className="nav-pill" href="#review">Gate</a>
        <a className="nav-pill" href="/admin/health">Health</a>
      </nav>
    </header>
  );
}

function ChatPanel() {
  return (
    <section id="chat" className="panel">
      <PanelTitle eyebrow="Built-in chat" title="Task intake" />
      <div className="space-y-3">
        <ChatBubble author="User" text="Create a static MVP shell that exposes the delivery flow from chat to merge." />
        <ChatBubble author="Sanchoris" text="Drafting native task SAN-104 and assigning the editable MVP workflow." muted />
        <ChatBubble author="User" text="Use Codex, verify with check and build, then wait at the merge gate." />
      </div>
      <div className="mt-4 rounded-lg border border-stone-300 bg-white p-3">
        <label className="text-xs font-semibold uppercase text-stone-500" htmlFor="message-draft">
          New message
        </label>
        <textarea
          id="message-draft"
          className="mt-2 min-h-24 w-full resize-none rounded-md border border-stone-300 bg-stone-50 p-3 text-sm text-stone-900 outline-none focus:border-emerald-600"
          defaultValue="Turn this request into a native task and run it in an isolated worktree."
        />
        <div className="mt-3 flex items-center justify-between gap-3">
          <span className="text-xs font-medium text-stone-500">Source of truth: native task</span>
          <Button className="action-button">Create task</Button>
        </div>
      </div>
    </section>
  );
}

function ChatBubble({ author, text, muted = false }: { author: string; text: string; muted?: boolean }) {
  return (
    <div className={`rounded-lg border p-3 ${muted ? 'border-stone-200 bg-stone-50' : 'border-emerald-200 bg-emerald-50'}`}>
      <p className="text-xs font-semibold uppercase text-stone-500">{author}</p>
      <p className="mt-1 text-sm leading-6 text-stone-800">{text}</p>
    </div>
  );
}

function TaskBoard() {
  return (
    <section id="tasks" className="panel">
      <PanelTitle eyebrow="Native task board" title="Active work" />
      <div className="grid gap-3">
        {tasks.map((task) => (
          <article key={task.id} className="rounded-lg border border-stone-300 bg-white p-3 shadow-sm">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-xs font-semibold text-emerald-700">{task.id}</p>
                <h3 className="mt-1 text-sm font-semibold text-stone-950">{task.title}</h3>
              </div>
              <span className="status-chip">{task.status}</span>
            </div>
            <dl className="mt-3 grid grid-cols-3 gap-2 text-xs">
              <TaskMeta label="Source" value={task.source} />
              <TaskMeta label="Priority" value={task.priority} />
              <TaskMeta label="Worker" value={task.worker} />
            </dl>
          </article>
        ))}
      </div>
    </section>
  );
}

function TaskMeta({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="font-medium text-stone-500">{label}</dt>
      <dd className="mt-1 font-semibold text-stone-800">{value}</dd>
    </div>
  );
}

function WorkflowCanvas() {
  return (
    <section id="workflow" className="panel min-w-0">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <PanelTitle eyebrow="Editable workflow canvas" title="MVP delivery graph" />
        <div className="rounded-lg border border-stone-300 bg-stone-50 px-3 py-2 text-xs font-medium text-stone-600">
          workflow.sanchoris.mvp.v1.yaml
        </div>
      </div>

      <div className="workflow-canvas mt-4" aria-label="Workflow canvas summary">
        {workflowBlocks.map((block, index) => (
          <React.Fragment key={block.name}>
            <article className={`workflow-block workflow-${block.status}`}>
              <p className="text-xs font-semibold uppercase text-stone-500">{block.status}</p>
              <h3 className="mt-1 text-base font-semibold text-stone-950">{block.name}</h3>
              <p className="mt-1 text-sm text-stone-600">{block.label}</p>
            </article>
            {index < workflowBlocks.length - 1 ? <div className="workflow-arrow" aria-hidden="true" /> : null}
          </React.Fragment>
        ))}
      </div>

      <pre className="mt-4 overflow-x-auto rounded-lg border border-stone-300 bg-[#24211c] p-4 text-xs leading-6 text-stone-100">
{`version: sanchoris.mvp.v1
blocks:
  - ChatInput
  - CreateTask
  - CreateWorkspace
  - RunWorker
  - RunVerification
  - Gate
  - CreatePR
  - Merge`}
      </pre>
    </section>
  );
}

function RunWorkspaceVerification() {
  return (
    <section className="grid gap-5 lg:grid-cols-3">
      <MetricPanel
        eyebrow="Run"
        title="Codex worker"
        metrics={[
          { label: 'Status', value: 'Running', tone: 'blue' },
          { label: 'Commit', value: 'pending' },
          { label: 'Log', value: 's3://runs/san-104/transcript' },
        ]}
      />
      <MetricPanel
        eyebrow="Workspace"
        title="Isolated worktree"
        metrics={[
          { label: 'Branch', value: 'task/frontend-mvp-shell' },
          { label: 'Path', value: 'sanchoris.task/frontend-mvp-shell' },
          { label: 'Invariant', value: '1 task = 1 worktree', tone: 'green' },
        ]}
      />
      <MetricPanel
        eyebrow="Verification"
        title="Check pipeline"
        metrics={[
          { label: 'Commands', value: 'pnpm check, pnpm build' },
          { label: 'Exit code', value: 'waiting' },
          { label: 'Result', value: 'Required before PR', tone: 'amber' },
        ]}
      />
    </section>
  );
}

function MetricPanel({ eyebrow, title, metrics }: { eyebrow: string; title: string; metrics: PanelMetric[] }) {
  return (
    <section className="panel">
      <PanelTitle eyebrow={eyebrow} title={title} compact />
      <dl className="mt-4 space-y-3">
        {metrics.map((metric) => (
          <div key={metric.label} className="rounded-lg border border-stone-300 bg-white p-3">
            <dt className="text-xs font-semibold uppercase text-stone-500">{metric.label}</dt>
            <dd className={`mt-1 break-words text-sm font-semibold ${metric.tone ? `metric-${metric.tone}` : 'text-stone-900'}`}>
              {metric.value}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function ProjectProfile() {
  return (
    <section className="panel">
      <PanelTitle eyebrow="Project profile" title="Sanchoris" />
      <dl className="mt-4 grid gap-3 text-sm">
        <ProfileRow label="Repository" value="/home/conao/ghq/github.com/conao3/sanchoris" />
        <ProfileRow label="Default branch" value="master" />
        <ProfileRow label="Worker policy" value="Codex CLI in task worktree" />
        <ProfileRow label="External channels" value="Supporting surfaces after MVP" />
      </dl>
    </section>
  );
}

function ProfileRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-stone-300 bg-white p-3">
      <dt className="text-xs font-semibold uppercase text-stone-500">{label}</dt>
      <dd className="mt-1 break-words font-medium text-stone-900">{value}</dd>
    </div>
  );
}

function ReviewPanels() {
  return (
    <section id="review" className="panel">
      <PanelTitle eyebrow="CreatePR / merge / gate" title="Human review inbox" />
      <div className="mt-4 grid gap-3">
        <ReviewCard title="Gate review" state="Waiting for verification" body="Review task summary, branch, worktree, planned command, and latest worker output before CreatePR." />
        <ReviewCard title="CreatePR" state="Ready after gate" body="Create a draft pull request from the worker commit and attach check results to the run." />
        <ReviewCard title="Merge" state="Locked" body="Merge stays blocked until the configured Gate block is approved by a human reviewer." />
      </div>
    </section>
  );
}

function ReviewCard({ title, state, body }: { title: string; state: string; body: string }) {
  return (
    <article className="rounded-lg border border-stone-300 bg-white p-3 shadow-sm">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-stone-950">{title}</h3>
        <span className="status-chip">{state}</span>
      </div>
      <p className="mt-2 text-sm leading-6 text-stone-600">{body}</p>
    </article>
  );
}

function PanelTitle({ eyebrow, title, compact = false }: { eyebrow: string; title: string; compact?: boolean }) {
  return (
    <div>
      <p className="text-xs font-semibold uppercase text-emerald-700">{eyebrow}</p>
      <h2 className={`${compact ? 'text-lg' : 'text-xl'} mt-1 font-semibold tracking-tight text-stone-950`}>{title}</h2>
    </div>
  );
}

function App() {
  const path = window.location.pathname;

  if (path === '/admin/health') {
    return <AdminHealthPage />;
  }

  return <MvpShell />;
}

const root = document.getElementById('root');

if (!root) {
  throw new Error('Root element was not found.');
}

createRoot(root).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </React.StrictMode>,
);
