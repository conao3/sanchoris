import { ApolloClient, HttpLink, InMemoryCache } from '@apollo/client';
import { ApolloProvider, useMutation, useQuery } from '@apollo/client/react';
import React from 'react';
import { Button } from 'react-aria-components';
import { createRoot } from 'react-dom/client';
import {
  CreateTaskDocument,
  type MvpShellQuery,
  MvpShellDocument,
  ValidateWorkflowCanvasDocument,
} from './graphql/generated/graphql';
import { InboxPage } from './pages/InboxPage';
import { LoginPage } from './pages/LoginPage';
import { PullRequestsPage } from './pages/PullRequestsPage';
import { QueuePage } from './pages/QueuePage';
import { WorkersPage } from './pages/WorkersPage';
import './style.css';

type PanelMetric = {
  label: string;
  value: string;
  tone?: 'green' | 'amber' | 'blue';
};

type ShellData = NonNullable<MvpShellQuery>;
type ShellTask = ShellData['tasks'][number];
type ShellWorkflow = ShellData['workflowSpecs'][number];
type ShellRun = ShellData['runs'][number];
type ShellProject = ShellData['projectProfiles'][number];
type ShellConversation = ShellData['conversations'][number];

const apolloClient = new ApolloClient({
  link: new HttpLink({ uri: '/api/graphql' }),
  cache: new InMemoryCache(),
});

function AdminHealthPage() {
  const { data, error, loading, refetch } = useQuery(MvpShellDocument, {
    fetchPolicy: 'no-cache',
  });
  const checkedAt = new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  }).format(new Date());

  return (
    <main className="min-h-screen bg-slate-50 px-6 py-10 text-slate-950">
      <section className="mx-auto flex w-full max-w-3xl flex-col gap-6 rounded-3xl border border-slate-200 bg-white p-8 shadow-sm">
        <div className="flex flex-col gap-2">
          <p className="text-sm font-semibold uppercase tracking-[0.24em] text-blue-600">Sanchoris Admin</p>
          <h1 className="text-4xl font-bold tracking-tight text-slate-950">GraphQL health</h1>
          <p className="text-base leading-7 text-slate-600">
            Checks the Rust backend through <code className="rounded bg-slate-100 px-1.5 py-0.5 text-sm">/api/graphql</code>.
          </p>
        </div>

        <dl className="grid gap-4 rounded-2xl border border-slate-200 bg-slate-50 p-5 sm:grid-cols-3">
          <StatusItem label="Status" value={error ? 'error' : loading ? 'checking' : 'ok'} />
          <StatusItem label="Viewer" value={data?.viewer.displayName ?? 'unknown'} />
          <StatusItem label="Last checked" value={checkedAt} />
        </dl>

        {error ? (
          <div className="rounded-2xl border border-red-200 bg-red-50 p-4 text-sm text-red-700" role="alert">
            {error.message}
          </div>
        ) : null}

        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <Button
            className="rounded-full bg-blue-600 px-5 py-2.5 text-sm font-semibold text-white shadow-sm outline-none transition hover:bg-blue-700 pressed:bg-blue-800 focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-slate-300"
            isDisabled={loading}
            onPress={() => void refetch()}
          >
            {loading ? 'Checking...' : 'Check again'}
          </Button>
          <p className="text-sm text-slate-500">Powered by Apollo Client and generated TypedDocumentNode documents.</p>
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
  const { data, error, loading, refetch } = useQuery(MvpShellDocument, {
    fetchPolicy: 'cache-first',
  });
  const [createTask, createTaskState] = useMutation(CreateTaskDocument);
  const [validateWorkflow, validationState] = useMutation(ValidateWorkflowCanvasDocument);

  const project = data?.projectProfiles[0];
  const conversation = data?.conversations[0];
  const workflow = data?.workflowSpecs[0];
  const run = data?.runs[0];

  async function handleCreateTask() {
    if (!project || !conversation || !workflow) return;

    await createTask({
      variables: {
        input: {
          conversationId: conversation.id,
          projectId: project.id,
          workflowId: workflow.id,
          title: 'Create a native task from built-in chat',
          description: 'Preview mutation wired through Apollo Client and GraphQL Code Generator.',
          priority: 'MEDIUM',
          worker: 'codex',
        },
      },
    });
  }

  async function handleValidateWorkflow() {
    if (!workflow) return;
    await validateWorkflow({ variables: { workflowId: workflow.id } });
  }

  return (
    <main className="min-h-screen bg-[#f4f1ea] text-stone-950">
      <div className="mx-auto flex w-full max-w-[1520px] flex-col gap-5 px-4 py-4 sm:px-6 lg:px-8">
        <ShellHeader loading={loading} onRefresh={() => void refetch()} />

        {error ? (
          <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700" role="alert">
            GraphQL query failed: {error.message}
          </div>
        ) : null}

        <section className="grid gap-5 xl:grid-cols-[360px_minmax(0,1fr)_380px]">
          <div className="flex flex-col gap-5">
            <ChatPanel conversation={conversation} onCreateTask={() => void handleCreateTask()} creating={createTaskState.loading} />
            <TaskBoard tasks={data?.tasks ?? []} />
          </div>

          <div className="flex min-w-0 flex-col gap-5">
            <WorkflowCanvas
              workflow={workflow}
              onValidate={() => void handleValidateWorkflow()}
              validating={validationState.loading}
              validationSummary={validationState.data?.validateWorkflowCanvas.valid ? 'valid' : undefined}
            />
            <RunWorkspaceVerification run={run} />
          </div>

          <div className="flex flex-col gap-5">
            <ProjectProfile project={project} />
            <ReviewPanels run={run} />
          </div>
        </section>
      </div>
    </main>
  );
}

function ShellHeader({ loading, onRefresh }: { loading: boolean; onRefresh: () => void }) {
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
        <a className="nav-pill" href="/admin/health">GraphQL</a>
        <Button className="nav-pill" isDisabled={loading} onPress={onRefresh}>{loading ? 'Loading' : 'Refresh'}</Button>
      </nav>
    </header>
  );
}

function ChatPanel({
  conversation,
  onCreateTask,
  creating,
}: {
  conversation?: ShellConversation;
  onCreateTask: () => void;
  creating: boolean;
}) {
  return (
    <section id="chat" className="panel">
      <PanelTitle eyebrow="Built-in chat" title={conversation?.title ?? 'Task intake'} />
      <div className="space-y-3">
        {(conversation?.messages ?? []).map((message) => (
          <ChatBubble key={message.id} author={message.author} text={message.body} muted={message.author === 'sanchoris'} />
        ))}
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
          <span className="text-xs font-medium text-stone-500">Source of truth: GraphQL native task</span>
          <Button className="action-button" isDisabled={creating} onPress={onCreateTask}>
            {creating ? 'Creating...' : 'Create task'}
          </Button>
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

function TaskBoard({ tasks }: { tasks: ShellTask[] }) {
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
              <span className="status-chip">{formatEnum(task.status)}</span>
            </div>
            <dl className="mt-3 grid grid-cols-3 gap-2 text-xs">
              <TaskMeta label="Priority" value={formatEnum(task.priority)} />
              <TaskMeta label="Worker" value={task.assignedWorker} />
              <TaskMeta label="Run" value={task.latestRunId} />
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

function WorkflowCanvas({
  workflow,
  onValidate,
  validating,
  validationSummary,
}: {
  workflow?: ShellWorkflow;
  onValidate: () => void;
  validating: boolean;
  validationSummary?: string;
}) {
  const blocks = workflow?.blocks ?? [];

  return (
    <section id="workflow" className="panel min-w-0">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
        <PanelTitle eyebrow="Editable workflow canvas" title={workflow?.name ?? 'MVP delivery graph'} />
        <Button className="rounded-lg border border-stone-300 bg-stone-50 px-3 py-2 text-xs font-medium text-stone-600" isDisabled={validating} onPress={onValidate}>
          {validating ? 'validating' : validationSummary ?? 'validate GraphQL workflow'}
        </Button>
      </div>

      <div className="workflow-canvas mt-4" aria-label="Workflow canvas summary">
        {blocks.map((block, index) => (
          <React.Fragment key={block.id}>
            <article className={`workflow-block workflow-${block.state.toLowerCase()}`}>
              <p className="text-xs font-semibold uppercase text-stone-500">{formatEnum(block.state)}</p>
              <h3 className="mt-1 text-base font-semibold text-stone-950">{block.kind}</h3>
              <p className="mt-1 text-sm text-stone-600">{block.label}</p>
            </article>
            {index < blocks.length - 1 ? <div className="workflow-arrow" aria-hidden="true" /> : null}
          </React.Fragment>
        ))}
      </div>

      <pre className="mt-4 overflow-x-auto rounded-lg border border-stone-300 bg-[#24211c] p-4 text-xs leading-6 text-stone-100">
        {workflow?.yaml ?? 'Loading workflow YAML from GraphQL...'}
      </pre>
    </section>
  );
}

function RunWorkspaceVerification({ run }: { run?: ShellRun }) {
  return (
    <section className="grid gap-5 lg:grid-cols-3">
      <MetricPanel
        eyebrow="Run"
        title={run?.workerKind ?? 'Codex worker'}
        metrics={[
          { label: 'Status', value: run ? formatEnum(run.status) : 'loading', tone: 'blue' },
          { label: 'Commit', value: run?.commitHash ?? 'pending' },
          { label: 'Log', value: run?.logUri ?? 'not recorded' },
        ]}
      />
      <MetricPanel
        eyebrow="Workspace"
        title="Isolated worktree"
        metrics={[
          { label: 'Branch', value: run?.workspace.branch ?? 'loading' },
          { label: 'Path', value: run?.workspace.worktreePath ?? 'loading' },
          { label: 'Invariant', value: '1 task = 1 worktree', tone: 'green' },
        ]}
      />
      <MetricPanel
        eyebrow="Verification"
        title="Check pipeline"
        metrics={[
          { label: 'Command', value: run?.verification.command ?? 'loading' },
          { label: 'Exit code', value: run?.verification.exitCode?.toString() ?? 'waiting' },
          { label: 'Result', value: run?.verification.summary ?? 'Required before PR', tone: 'amber' },
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

function ProjectProfile({ project }: { project?: ShellProject }) {
  return (
    <section className="panel">
      <PanelTitle eyebrow="Project profile" title={project?.name ?? 'Sanchoris'} />
      <dl className="mt-4 grid gap-3 text-sm">
        <ProfileRow label="Repository" value={project?.repositoryPath ?? 'loading'} />
        <ProfileRow label="Default branch" value={project?.defaultBranch ?? 'loading'} />
        <ProfileRow label="Worker policy" value={project?.workerPolicy ?? 'loading'} />
        <ProfileRow label="Check command" value={project?.checkCommand ?? 'loading'} />
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

function ReviewPanels({ run }: { run?: ShellRun }) {
  return (
    <section id="review" className="panel">
      <PanelTitle eyebrow="CreatePR / merge / gate" title="Human review inbox" />
      <div className="mt-4 grid gap-3">
        <ReviewCard title="Gate review" state={run?.gate.state ?? 'loading'} body={`Review target: ${run?.gate.reviewTarget ?? 'CreatePR'}`} />
        <ReviewCard title="CreatePR" state={run?.pullRequest.status ?? 'waiting'} body={`Source branch: ${run?.pullRequest.sourceBranch ?? 'pending'}`} />
        <ReviewCard title="Merge" state={run?.merge.status ?? 'blocked'} body={`Method: ${run?.merge.method ?? 'squash'}`} />
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

function formatEnum(value: string) {
  return value
    .toLowerCase()
    .split('_')
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(' ');
}

function App() {
  const path = window.location.pathname;

  if (path === '/workers') {
    return <WorkersPage />;
  }
  if (path === '/pull-requests') {
    return <PullRequestsPage />;
  }
  if (path === '/queue') {
    return <QueuePage />;
  }
  if (path === '/inbox') {
    return <InboxPage />;
  }
  if (path === '/login') {
    return <LoginPage />;
  }
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
    <ApolloProvider client={apolloClient}>
      <App />
    </ApolloProvider>
  </React.StrictMode>,
);
