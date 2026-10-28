import { useApolloClient, useMutation } from '@apollo/client/react';
import { useNavigate } from '@tanstack/react-router';
import { Check, ExternalLink, ShieldCheck, Users } from 'lucide-react';
import { type FormEvent, type ReactNode, useState } from 'react';
import { Button } from '../components/Button';
import { Dim, ErrorNote, Note, Row, Skeleton, State, Sub, Title } from '../components/ui';
import { AcceptInvitationDocument, CreateTenantDocument, CreateTenantPageDocument } from '../graphql/generated/graphql';
import { type MessageKey, t } from '../i18n';
import { until } from '../lib/format';
import { planPrice } from '../lib/plan';
import { useData } from '../lib/query';
import { rememberTenant } from '../lib/tenant';

const roleLabels: Record<string, MessageKey> = { OWNER: 'role.owner', ADMIN: 'role.admin', MEMBER: 'role.member' };

export function AuthLayout({ children }: { children: ReactNode }) {
  return (
    <div className="relative flex min-h-screen justify-center">
      <div className="absolute left-8 top-6 text-lg font-semibold">sanchoris</div>
      <div className="w-[460px] pb-16 pt-[150px]">{children}</div>
    </div>
  );
}

export function CreateTenantPage() {
  const navigate = useNavigate();
  const client = useApolloClient();
  const { data, error, refetch } = useData(CreateTenantPageDocument, {});
  const [name, setName] = useState('');
  const [planId, setPlanId] = useState<string>();
  const [create, createState] = useMutation(CreateTenantDocument);
  const [accept, acceptState] = useMutation(AcceptInvitationDocument);
  const plans = data?.viewer.newTenantPlans ?? [];
  const selected = plans.find((plan) => plan.id === planId) ?? plans.find((plan) => plan.name === 'Pro') ?? plans[0];

  function enter(slug: string) {
    rememberTenant(slug);
    void client.resetStore().then(() => navigate({ to: '/' }));
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!selected) {
      return;
    }
    void create({ variables: { input: { name, planId: selected.id } } }).then((result) => {
      const payload = result.data?.createTenant;
      if (payload?.checkoutUrl) {
        window.location.assign(payload.checkoutUrl);
      } else if (payload?.tenant) {
        enter(payload.tenant.slug);
      }
    });
  }

  return (
    <AuthLayout>
      <Title>
        <span className="text-auth">{t('tenantNew.title')}</span>
      </Title>
      <Sub className="mt-1.5">{t('tenantNew.intro')}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={5} /> : null}
      {data?.viewer.invitations.map((invitation) => (
        <Note key={invitation.id} icon={Users}>
          <span className="font-medium">
            {t('tenantNew.invited', { name: invitation.invitedBy.user.name, tenant: invitation.tenantName, role: t(roleLabels[invitation.role]) })}
          </span>
          <Dim>{t('tenantNew.invitedDetail', { expires: until(invitation.expiresAt), tenant: invitation.tenantName })}</Dim>
          <div className="mt-2">
            <Button
              variant="primary"
              icon={Check}
              isDisabled={acceptState.loading}
              onPress={() =>
                void accept({ variables: { invitationId: invitation.id } }).then((result) => {
                  const slug = result.data?.acceptInvitation.tenant.slug;
                  if (slug) {
                    enter(slug);
                  }
                })
              }
            >
              {t('tenantNew.join', { tenant: invitation.tenantName })}
            </Button>
          </div>
        </Note>
      ))}
      {data ? (
        <form onSubmit={submit}>
          {data.viewer.invitations.length > 0 ? <Dim className="mb-1">{t('tenantNew.orCreate')}</Dim> : null}
          <label htmlFor="tenant-name" className="mb-1 mt-3 block text-sm text-muted">
            {t('col.name')}
          </label>
          <input
            id="tenant-name"
            required
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="acme"
            className="h-[30px] w-full rounded-ui border border-line2 bg-bg px-2.5 outline-none focus:border-ink"
          />
          <div className="mb-1 mt-3 text-sm text-muted">{t('billing.plan')}</div>
          <div role="radiogroup" aria-label={t('billing.plan')}>
            {plans.map((plan) => {
              const isSelected = plan.id === selected?.id;
              return (
                <Row key={plan.id} cols="120px 1fr 100px" selected={isSelected} onClick={() => setPlanId(plan.id)}>
                  <span className="font-medium" role="radio" aria-checked={isSelected}>
                    {plan.name}
                  </span>
                  <span className={isSelected ? '' : 'text-muted'}>
                    {[
                      planPrice(plan),
                      ...(plan.taskLimit == null || plan.workspaceHourLimit == null || plan.memberLimit == null
                        ? [t('tenantNew.noLimits')]
                        : [
                            plan.limitPeriod === 'TOTAL'
                              ? t('tenantNew.limitsTotal', { tasks: plan.taskLimit, hours: plan.workspaceHourLimit })
                              : t('tenantNew.limitsMonth', { tasks: plan.taskLimit, hours: plan.workspaceHourLimit }),
                            t(plan.memberLimit === 1 ? 'tenantNew.memberOne' : 'tenantNew.members', { n: plan.memberLimit }),
                          ]),
                    ].join(' · ')}
                  </span>
                  {isSelected ? (
                    <State tone="ok" icon={ShieldCheck}>
                      {t('tenantNew.selected')}
                    </State>
                  ) : (
                    <span className="text-muted">{plan.requiresPayment ? '' : t('tenantNew.noCard')}</span>
                  )}
                </Row>
              );
            })}
          </div>
          {createState.error ? <Dim className="mt-2 text-err">{createState.error.message}</Dim> : null}
          <div className="mt-3.5">
            <Button type="submit" variant="primary" icon={selected?.requiresPayment ? ExternalLink : Check} isDisabled={createState.loading || !name.trim()}>
              {selected?.requiresPayment ? t('tenantNew.createAndPay') : t('tenantNew.create')}
            </Button>
          </div>
          <Dim className="mt-2.5">{t('tenantNew.after')}</Dim>
        </form>
      ) : null}
    </AuthLayout>
  );
}
