import { Ban, CircleCheck, Trash2 } from 'lucide-react';
import { type FormEvent, Fragment, useState } from 'react';
import { Button } from '../components/Button.tsx';
import { Page, Pg } from '../components/Shell.tsx';
import { ActionError, Empty, ErrorNote, Row, Skeleton, State, Sub, Table, Th, Title } from '../components/ui.tsx';
import { OperatorTenantsDocument, SuspendTenantDocument, UnsuspendTenantDocument } from '../graphql/generated/graphql.ts';
import { t } from '../i18n/index.ts';
import { hours } from '../lib/format.ts';
import { useListNav } from '../lib/keys.ts';
import { useAction } from '../lib/mutation.ts';
import { useData } from '../lib/query.ts';

function ratio(value: number, limit: number | null | undefined, total: boolean): string {
  if (limit == null) {
    return `${value} / ${t('plan.noLimit')}`;
  }
  return total ? t('operator.ratioTotal', { n: value, total: limit }) : `${value} / ${limit}`;
}

export function OperatorTenantsPage() {
  const { data, error, refetch } = useData(OperatorTenantsDocument, {});
  const [suspend, suspendState] = useAction(SuspendTenantDocument);
  const [unsuspend, unsuspendState] = useAction(UnsuspendTenantDocument);
  const operator = data?.operator;
  const tenants = operator?.tenants ?? [];
  const nav = useListNav(tenants.length, () => undefined);
  const [suspending, setSuspending] = useState<string | null>(null);
  const [reason, setReason] = useState('');

  function submitSuspend(event: FormEvent, tenantId: string) {
    event.preventDefault();
    void suspend({ tenantId, reason: reason.trim() }).then((done) => {
      if (done) {
        setSuspending(null);
        setReason('');
      }
    });
  }
  const cols = '1fr 160px 70px 120px 80px 90px 130px 240px';

  return (
    <Page
      crumbs={[{ label: t('operator.operator') }, { label: t('nav.tenants') }]}
      badge={<State tone="err" icon={Ban} className="ml-1.5">{t('operator.only')}</State>}
      keys={[['j k', 'keys.move']]}
    >
      <Pg>
        <Title>{t('nav.tenants')}</Title>
        <Sub>
          {operator
            ? t('operator.intro', {
                n: tenants.length,
                workspaces: operator.liveWorkspaces,
                hours: Math.round(operator.runHoursThisMonth),
                revenue: `$${operator.revenueThisMonthUsd}`,
              })
            : ' '}
        </Sub>
        {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
        <ActionError error={suspendState.error ?? unsuspendState.error} className="mb-2" />
        {!data && !error ? <Skeleton rows={4} /> : null}
        {data && !operator ? <Empty>{t('operator.notOperator')}</Empty> : null}
        {tenants.length > 0 ? (
          <Table>
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
                <Fragment key={tenant.id}>
                  <Row cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                    <span className="font-medium underline decoration-line2 underline-offset-[3px]">{tenant.name}</span>
                    <span className="font-mono text-sm">
                      {tenant.githubOwner
                        ? `${tenant.githubOwner.kind === 'ORGANIZATION' ? t('operator.org') : t('operator.user')}: ${tenant.githubOwner.login}`
                        : '—'}
                    </span>
                    <span>{tenant.plan.name}</span>
                    <span className="text-right font-mono text-sm">{ratio(tenant.usage.tasksThisPeriod, tenant.plan.taskLimit, total)}</span>
                    <span className="text-right font-mono text-sm">{ratio(tenant.usage.seatsUsed, tenant.plan.memberLimit, false)}</span>
                    <span className="text-right font-mono text-sm">{tenant.plan.concurrencyLimit}</span>
                    <span className="text-right font-mono text-sm">
                      {ratio(hours(tenant.usage.workspaceSecondsThisPeriod), tenant.plan.workspaceHourLimit, total)}
                    </span>
                    <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                      {tenant.state === 'ACTIVE' ? (
                        <State tone="ok" icon={CircleCheck}>
                          {t('operator.active')}
                        </State>
                      ) : tenant.state === 'DELETING' ? (
                        <State tone="warn" icon={Trash2}>
                          {t('operator.deleting')}
                        </State>
                      ) : (
                        <State tone="err" icon={Ban} wrap>
                          {t('operator.suspended', { reason: tenant.suspendedReason ?? '' })}
                        </State>
                      )}
                      {nav.selected === index && tenant.state !== 'DELETING' ? (
                        tenant.state === 'ACTIVE' ? (
                          <Button small variant="danger" onPress={() => setSuspending(tenant.id)} isDisabled={suspending === tenant.id}>
                            {t('operator.suspend')}
                          </Button>
                        ) : (
                          <Button small onPress={() => void unsuspend({ tenantId: tenant.id })} isDisabled={unsuspendState.loading}>
                            {t('operator.unsuspend')}
                          </Button>
                        )
                      ) : null}
                    </span>
                  </Row>
                  {suspending === tenant.id && tenant.state === 'ACTIVE' ? (
                    <form
                      onSubmit={(event) => submitSuspend(event, tenant.id)}
                      className="flex items-center gap-2 border-b border-line py-2.5 pl-1"
                    >
                      <label htmlFor="suspend-reason" className="text-muted">
                        {t('operator.suspendReason')}
                      </label>
                      <input
                        id="suspend-reason"
                        name="reason"
                        required
                        autoFocus
                        className="h-[30px] w-80 rounded-ui border border-line2 bg-bg px-2.5 outline-none focus:border-ink"
                        value={reason}
                        onChange={(event) => setReason(event.target.value)}
                      />
                      <Button type="submit" variant="danger" isDisabled={suspendState.loading || !reason.trim()}>
                        {t('operator.suspend')}
                      </Button>
                      <Button onPress={() => setSuspending(null)}>{t('common.close')}</Button>
                    </form>
                  ) : null}
                </Fragment>
              );
            })}
          </Table>
        ) : null}
      </Pg>
    </Page>
  );
}
