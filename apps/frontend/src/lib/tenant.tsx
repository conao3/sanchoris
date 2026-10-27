import { useApolloClient } from '@apollo/client/react';
import { Navigate } from '@tanstack/react-router';
import { type ReactNode, createContext, useContext, useState } from 'react';
import { ErrorNote, Skeleton } from '../components/ui';
import { type ShellQuery, ShellDocument } from '../graphql/generated/graphql';
import { useData } from './query';

const TENANT_KEY = 'sanchoris_tenant';

type Membership = ShellQuery['viewer']['memberships'][number];

type TenantContextValue = {
  viewer: ShellQuery['viewer'];
  membership: Membership;
  slug: string;
  switchTenant: (slug: string) => void;
};

const TenantContext = createContext<TenantContextValue | null>(null);

export function rememberTenant(slug: string) {
  localStorage.setItem(TENANT_KEY, slug);
}

export function TenantProvider({ children }: { children: ReactNode }) {
  const client = useApolloClient();
  const { data, error, refetch } = useData(ShellDocument, {});
  const [stored, setStored] = useState(() => localStorage.getItem(TENANT_KEY));

  if (error) {
    return (
      <div className="p-6">
        <ErrorNote message={error.message} onRetry={refetch} />
      </div>
    );
  }
  if (!data) {
    return (
      <div className="p-6">
        <Skeleton />
      </div>
    );
  }
  const memberships = data.viewer.memberships;
  if (memberships.length === 0) {
    return <Navigate to="/tenants/new" />;
  }
  const membership = memberships.find((item) => item.tenant.slug === stored) ?? memberships[0];

  function switchTenant(slug: string) {
    rememberTenant(slug);
    setStored(slug);
    void client.resetStore();
  }

  return (
    <TenantContext.Provider value={{ viewer: data.viewer, membership, slug: membership.tenant.slug, switchTenant }}>
      {children}
    </TenantContext.Provider>
  );
}

export function useTenant(): TenantContextValue {
  const value = useContext(TenantContext);
  if (!value) {
    throw new Error('useTenant must be used inside TenantProvider');
  }
  return value;
}
