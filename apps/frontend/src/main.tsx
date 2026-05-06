import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import React from 'react';
import { Button } from 'react-aria-components';
import { createRoot } from 'react-dom/client';
import './style.css';

type HealthResponse = {
  status: string;
  service: string;
};

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      staleTime: 30_000,
    },
  },
});

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

function App() {
  const path = window.location.pathname;

  if (path !== '/admin/health') {
    return (
      <main className="grid min-h-screen place-content-center bg-slate-50 px-6 text-center text-slate-950">
        <div className="max-w-xl rounded-3xl border border-slate-200 bg-white p-8 shadow-sm">
          <p className="text-sm font-semibold uppercase tracking-[0.24em] text-blue-600">Sanchoris</p>
          <h1 className="mt-3 text-4xl font-bold tracking-tight">FRONTEND IS READY.</h1>
          <p className="mt-4 text-slate-600">
            Open <a className="font-semibold text-blue-600 underline" href="/admin/health">/admin/health</a> to check backend health.
          </p>
        </div>
      </main>
    );
  }

  return <AdminHealthPage />;
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
