import { ApolloClient, ApolloLink, HttpLink, InMemoryCache } from '@apollo/client';
import { ApolloProvider } from '@apollo/client/react';
import { createRootRoute, createRoute, createRouter, Outlet, redirect, RouterProvider } from '@tanstack/react-router';
import React from 'react';
import { createRoot } from 'react-dom/client';
import { AppFrame } from './components/Shell';
import { checkoutSearch } from './components/billing';
import { authorizedFetch, isLoggedIn } from './lib/cognito';
import { mockApi } from './lib/env';
import { TenantProvider } from './lib/tenant';
import { AuthCallbackPage } from './pages/AuthCallbackPage';
import { ChatPage } from './pages/ChatPage';
import { CreateTenantPage } from './pages/CreateTenantPage';
import { HomePage } from './pages/HomePage';
import { LoginPage } from './pages/LoginPage';
import { OperatorTenantsPage } from './pages/OperatorTenantsPage';
import { RunPage } from './pages/RunPage';
import { SkillPage } from './pages/SkillPage';
import { SkillsPage } from './pages/SkillsPage';
import { TaskPage } from './pages/TaskPage';
import { TasksPage } from './pages/TasksPage';
import { TriggersPage } from './pages/TriggersPage';
import { AuditPage } from './pages/settings/AuditPage';
import { BillingPage } from './pages/settings/BillingPage';
import { ConfigPage } from './pages/settings/ConfigPage';
import { CredentialsPage } from './pages/settings/CredentialsPage';
import { IntegrationsPage } from './pages/settings/IntegrationsPage';
import { MembersPage } from './pages/settings/MembersPage';
import './style.css';

async function createLink(): Promise<ApolloLink> {
  if (mockApi) {
    const { createMockLink } = await import('./mock');
    return createMockLink();
  }
  return new HttpLink({ uri: '/api/graphql', fetch: authorizedFetch });
}

const apolloClient = new ApolloClient({
  link: await createLink(),
  cache: new InMemoryCache(),
});

function authGuard() {
  if (!mockApi && !isLoggedIn()) {
    throw redirect({ to: '/login' });
  }
}

const rootRoute = createRootRoute({ component: Outlet });

const loginRoute = createRoute({ getParentRoute: () => rootRoute, path: '/login', component: LoginPage });
const authCallbackRoute = createRoute({ getParentRoute: () => rootRoute, path: '/auth/callback', component: AuthCallbackPage });

const signedInRoute = createRoute({ getParentRoute: () => rootRoute, id: 'signed-in', beforeLoad: authGuard, component: Outlet });

const createTenantRoute = createRoute({ getParentRoute: () => signedInRoute, path: '/tenants/new', validateSearch: checkoutSearch, component: CreateTenantPage });

const appRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'app',
  beforeLoad: authGuard,
  component: () => (
    <TenantProvider>
      <AppFrame>
        <Outlet />
      </AppFrame>
    </TenantProvider>
  ),
});

const homeRoute = createRoute({ getParentRoute: () => appRoute, path: '/', validateSearch: checkoutSearch, component: HomePage });
const triggersRoute = createRoute({ getParentRoute: () => appRoute, path: '/triggers', component: TriggersPage });
const skillsRoute = createRoute({ getParentRoute: () => appRoute, path: '/skills', component: SkillsPage });
const skillRoute = createRoute({ getParentRoute: () => appRoute, path: '/skills/$skill', component: SkillPage });
const chatRoute = createRoute({ getParentRoute: () => appRoute, path: '/chat', component: ChatPage });
const chatTaskRoute = createRoute({ getParentRoute: () => appRoute, path: '/chat/$task', component: ChatPage });
const tasksRoute = createRoute({ getParentRoute: () => appRoute, path: '/tasks', component: TasksPage });
const taskRoute = createRoute({ getParentRoute: () => appRoute, path: '/tasks/$task', component: TaskPage });
const runRoute = createRoute({ getParentRoute: () => appRoute, path: '/tasks/$task/runs/$n', component: RunPage });
const operatorTenantsRoute = createRoute({ getParentRoute: () => appRoute, path: '/operator/tenants', component: OperatorTenantsPage });
const integrationsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/settings',
  validateSearch: (search: Record<string, unknown>): { github?: string } => (typeof search.github === 'string' ? { github: search.github } : {}),
  component: IntegrationsPage,
});
const configRoute = createRoute({ getParentRoute: () => appRoute, path: '/settings/config', component: ConfigPage });
const credentialsRoute = createRoute({ getParentRoute: () => appRoute, path: '/settings/credentials', component: CredentialsPage });
const membersRoute = createRoute({ getParentRoute: () => appRoute, path: '/settings/members', component: MembersPage });
const auditRoute = createRoute({ getParentRoute: () => appRoute, path: '/settings/audit', component: AuditPage });
const billingRoute = createRoute({ getParentRoute: () => appRoute, path: '/settings/billing', validateSearch: checkoutSearch, component: BillingPage });

const routeTree = rootRoute.addChildren([
  loginRoute,
  authCallbackRoute,
  signedInRoute.addChildren([createTenantRoute]),
  appRoute.addChildren([
    homeRoute,
    triggersRoute,
    skillsRoute,
    skillRoute,
    chatRoute,
    chatTaskRoute,
    tasksRoute,
    taskRoute,
    runRoute,
    operatorTenantsRoute,
    integrationsRoute,
    configRoute,
    credentialsRoute,
    membersRoute,
    auditRoute,
    billingRoute,
  ]),
]);

const router = createRouter({ routeTree });

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}

const root = document.getElementById('root');

if (!root) {
  throw new Error('Root element was not found.');
}

createRoot(root).render(
  <React.StrictMode>
    <ApolloProvider client={apolloClient}>
      <RouterProvider router={router} />
    </ApolloProvider>
  </React.StrictMode>,
);
