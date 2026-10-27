import { useMutation } from '@apollo/client/react';
import { Ban, CircleCheck } from 'lucide-react';
import { Button } from '../components/Button';
import { Page, Pg } from '../components/Shell';
import { Empty, ErrorNote, Row, Skeleton, State, Sub, Th, Title } from '../components/ui';
import { OperatorTenantsDocument, SuspendTenantDocument, UnsuspendTenantDocument } from '../graphql/generated/graphql';
import { t } from '../i18n';
import { hours } from '../lib/format';
import { useListNav } from '../lib/keys';
import { useData } from '../lib/query';

function ratio(value: number, limit: number, total: boolean): string {
  return total ? t('operator.ratioTotal', { n: value, total: limit }) : `${value} / ${limit}`;
}

export function OperatorTenantsPage() {
  const { data, error, refetch } = useData(OperatorTenantsDocument, {});
  const [suspend] = useMutation(SuspendTenantDocument);
  const [unsuspend] = useMutation(UnsuspendTenantDocument);
  const operator = data?.operator;
  const tenants = operator?.tenants ?? [];
  const nav = useListNav(tenants.length, () => undefined);
  const cols = '1fr 160px 70px 120px 80px 90px 130px 240px';

  return (
    <Page
      crumbs={[{ label: t('operator.operator') }, { label: t('nav.tenants') }]}
      badge={<State tone="err" icon={Ban} className="ml-1.5">{t('operator.only')}</State>}
      keys={[
        ['j k', 'keys.move'],
        ['↵', 'keys.open'],
      ]}
    >
      <Pg>
        <Title>{t('nav.tenants')}</Title>
        <Sub>
          {operator
            ? t('operator.intro', {
                n: tenants.length,
                runs: operator.runningRuns,
                hours: Math.round(operator.runHoursThisMonth),
                revenue: `$${operator.revenueThisMonthUsd}`,
              })
            : ' '}
        </Sub>
        {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
        {!data && !error ? <Skeleton rows={4} /> : null}
        {data && !operator ? <Empty>{t('operator.notOperator')}</Empty> : null}
        {tenants.length > 0 ? (
          <>
            <Th cols={cols}>
              <span>{t('col.tenant')}</span>
              <span>{t('col.github')}</span>
              <span>{t('billing.plan')}</span>
              <span className="text-right">{t('col.tasksThisPeriod')}</span>
              <span className="text-right">{t('settings.members')}</span>
              <span className="text-right">{t('col.concurrency')}</span>
              <span className="text-right">{t('col.workspaceHours')}</span>
              <span>{t('col.state')}</span>
            </Th>
            {tenants.map((tenant, index) => {
              const total = tenant.plan.limitPeriod === 'TOTAL';
              return (
                <Row key={tenant.id} cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                  <span className="font-medium underline decoration-line2 underline-offset-[3px]">{tenant.name}</span>
                  <span className="font-mono text-sm">
                    {tenant.githubOwner
                      ? `${tenant.githubOwner.kind === 'ORGANIZATION' ? t('operator.org') : t('operator.user')}: ${tenant.githubOwner.login}`
                      : '—'}
                  </span>
                  <span>{tenant.plan.name}</span>
                  <span className="text-right font-mono text-sm">{ratio(tenant.usage.tasksThisPeriod, tenant.plan.taskLimit, total)}</span>
                  <span className="text-right font-mono text-sm">{`${tenant.usage.seatsUsed} / ${tenant.plan.memberLimit}`}</span>
                  <span className="text-right font-mono text-sm">{tenant.plan.concurrencyLimit}</span>
                  <span className="text-right font-mono text-sm">
                    {ratio(hours(tenant.usage.workspaceSecondsThisPeriod), tenant.plan.workspaceHourLimit, total)}
                  </span>
                  <span className="flex items-center gap-2">
                    {tenant.state === 'ACTIVE' ? (
                      <State tone="ok" icon={CircleCheck}>
                        {t('operator.active')}
                      </State>
                    ) : (
                      <State tone="err" icon={Ban}>
                        {t('operator.suspended', { reason: tenant.suspendedReason ?? '' })}
                      </State>
                    )}
                    {nav.selected === index ? (
                      tenant.state === 'ACTIVE' ? (
                        <Button
                          small
                          variant="danger"
                          onPress={() => {
                            const reason = window.prompt(t('operator.suspendReason'));
                            if (reason) {
                              void suspend({ variables: { tenantId: tenant.id, reason } });
                            }
                          }}
                        >
                          {t('operator.suspend')}
                        </Button>
                      ) : (
                        <Button small onPress={() => void unsuspend({ variables: { tenantId: tenant.id } })}>
                          {t('operator.unsuspend')}
                        </Button>
                      )
                    ) : null}
                  </span>
                </Row>
              );
            })}
          </>
        ) : null}
      </Pg>
    </Page>
  );
}
