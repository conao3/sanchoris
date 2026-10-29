import { useMutation } from '@apollo/client/react';
import { ExternalLink, ShieldCheck } from 'lucide-react';
import { Button } from '../../components/Button';
import { SettingsPage } from '../../components/SettingsTabs';
import { Dim, ErrorNote, Kv, Meter, Row, Section, Skeleton, State, Sub, Th, Title } from '../../components/ui';
import { BillingDocument, ChangePlanDocument, OpenBillingPortalDocument } from '../../graphql/generated/graphql';
import { t } from '../../i18n';
import { day, elapsedRatio, hours, percent } from '../../lib/format';
import { countOf, hoursOf, planLimit, planPrice, usedShare } from '../../lib/plan';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

export function BillingPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(BillingDocument, { tenant: slug });
  const [changePlan, changeState] = useMutation(ChangePlanDocument);
  const [openPortal, portalState] = useMutation(OpenBillingPortalDocument);
  const tenant = data?.tenant;
  const admin = tenant?.plan.isAdmin ?? false;
  const cols = '100px 140px 160px 160px 110px 100px 1fr';

  function portal() {
    if (tenant) {
      void openPortal({ variables: { tenantId: tenant.id } }).then((result) => {
        const url = result.data?.openBillingPortal.url;
        if (url) {
          window.open(url, '_blank', 'noreferrer');
        }
      });
    }
  }

  function change(planId: string) {
    if (tenant) {
      void changePlan({ variables: { tenantId: tenant.id, planId } }).then((result) => {
        const url = result.data?.changePlan.checkoutUrl;
        if (url) {
          window.location.assign(url);
        }
      });
    }
  }

  let content = null;
  if (tenant && data) {
    const { plan, usage } = tenant;
    const period = usage.periodEnd ? `${day(usage.periodStart)} – ${day(new Date(new Date(usage.periodEnd).getTime() - 86_400_000).toISOString())}` : t('billing.inTotal');
    const pace = usage.periodEnd ? elapsedRatio(usage.periodStart, usage.periodEnd) : undefined;
    const workspaceHours = hours(usage.workspaceSecondsThisPeriod);
    content = (
      <>
        <div className="grid grid-cols-2 gap-10">
          <div>
            <Section count={period} first>
              {t('billing.tasksThisPeriod')}
            </Section>
            <div className="mt-2.5">
              <Meter value={usedShare(usage.tasksThisPeriod, plan.taskLimit)} pace={pace} />
            </div>
            <Dim className="mt-2">
              {[
                countOf(usage.tasksThisPeriod, plan.taskLimit),
                pace !== undefined ? t('home.paceAt', { pace: percent(pace) }) : null,
                usage.periodEnd ? t('billing.countResets', { at: day(usage.periodEnd) }) : null,
              ]
                .filter(Boolean)
                .join(' · ')}
            </Dim>
          </div>
          <div>
            <Section first>{t('billing.currentPlan')}</Section>
            <Kv
              items={[
                [t('billing.plan'), <><b>{plan.name}</b> · {planPrice(plan)}</>],
                [t('settings.members'), countOf(usage.seatsUsed, plan.memberLimit)],
                [t('billing.nextBilling'), usage.periodEnd ? day(usage.periodEnd) : '—'],
                [t('billing.concurrentRuns'), plan.concurrencyLimit],
                [t('billing.inboundEvents'), t('billing.perMinute', { n: plan.inboundEventsPerMinute })],
              ]}
            />
          </div>
        </div>
        <div className="mt-1.5 grid grid-cols-2 gap-10">
          <div>
            <Section count={period}>{t('billing.workspaceHours')}</Section>
            <div className="mt-2.5">
              <Meter value={usedShare(workspaceHours, plan.workspaceHourLimit)} pace={pace} />
            </div>
            <Dim className="mt-2">
              {admin ? hoursOf(workspaceHours, plan.workspaceHourLimit) : `${hoursOf(workspaceHours, plan.workspaceHourLimit)} · ${t('billing.thresholds')}`}
            </Dim>
          </div>
          <div />
        </div>
        {admin ? null : (
          <>
            <Section>{t('billing.plans')}</Section>
            <Th cols={cols}>
              <span>{t('billing.plan')}</span>
              <span>{t('col.price')}</span>
              <span>{t('nav.tasks')}</span>
              <span>{t('col.workspaceHours')}</span>
              <span>{t('col.concurrent')}</span>
              <span>{t('settings.members')}</span>
              <span />
            </Th>
            {data.plans.map((item) => (
              <Row key={item.id} cols={cols} selected={item.id === plan.id}>
                <span className="font-medium">{item.name}</span>
                <span>{planPrice(item)}</span>
                <span>{planLimit(item.taskLimit, item.limitPeriod)}</span>
                <span>{planLimit(item.workspaceHourLimit, item.limitPeriod)}</span>
                <span>{item.concurrencyLimit}</span>
                <span>{item.memberLimit}</span>
                {item.id === plan.id ? (
                  <State tone="ok" icon={ShieldCheck}>
                    {t('billing.current')}
                  </State>
                ) : (
                  <span className="justify-self-end">
                    <Button small isDisabled={changeState.loading} onPress={() => change(item.id)}>
                      {t('billing.change')}
                    </Button>
                  </span>
                )}
              </Row>
            ))}
            <Dim className="mt-3">{t('billing.footer')}</Dim>
          </>
        )}
      </>
    );
  }

  return (
    <SettingsPage
      tab="billing"
      actions={
        admin ? undefined : (
          <Button icon={ExternalLink} onPress={portal} isDisabled={!tenant}>
            {t('billing.manageInStripe')}
          </Button>
        )
      }
    >
      <Title>{t('settings.billing')}</Title>
      <Sub>{t(admin ? 'billing.adminIntro' : 'billing.intro', { tenant: tenant?.name ?? '' })}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {portalState.error ? <Dim className="my-3 text-err">{portalState.error.message}</Dim> : null}
      {changeState.error ? <Dim className="my-3 text-err">{changeState.error.message}</Dim> : null}
      {!data && !error ? <Skeleton rows={6} /> : null}
      {content}
    </SettingsPage>
  );
}
