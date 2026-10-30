import { useApolloClient } from '@apollo/client/react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { ExternalLink, ShieldCheck, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../../components/Button.tsx';
import { ConfirmDialog } from '../../components/ConfirmDialog.tsx';
import { SettingsPage } from '../../components/SettingsTabs.tsx';
import { CheckoutNote, TenantStateNote, useCheckoutOutcome } from '../../components/billing.tsx';
import { ActionError, Chip, Dim, ErrorNote, Kv, Meter, Row, Section, Skeleton, State, Sub, Table, Th, Title } from '../../components/ui.tsx';
import { BillingDocument, ChangePlanDocument, DeleteTenantDocument, OpenBillingPortalDocument } from '../../graphql/generated/graphql.ts';
import { t, tx } from '../../i18n/index.ts';
import { day, elapsedRatio, hours, percent } from '../../lib/format.ts';
import { useAction } from '../../lib/mutation.ts';
import { countOf, hoursOf, planLimit, planPrice, usedShare } from '../../lib/plan.ts';
import { useData } from '../../lib/query.ts';
import { useTenant } from '../../lib/tenant.tsx';

export function BillingPage() {
  const { slug, membership } = useTenant();
  const { data, error, refetch } = useData(BillingDocument, { tenant: slug });
  const [changePlan, changeState] = useAction(ChangePlanDocument);
  const [openPortal, portalState] = useAction(OpenBillingPortalDocument);
  const tenant = data?.tenant;
  const admin = tenant?.plan.isAdmin ?? false;
  const checkout = useCheckoutOutcome(useSearch({ from: '/app/settings/billing' }).checkout);
  const cols = '100px 140px 160px 160px 110px 100px 1fr';

  function portal() {
    if (tenant) {
      void openPortal({ tenantId: tenant.id }).then((result) => {
        const url = result?.openBillingPortal.url;
        if (url) {
          window.location.assign(url);
        }
      });
    }
  }

  function change(planId: string) {
    if (tenant) {
      void changePlan({ tenantId: tenant.id, planId }).then((result) => {
        const url = result?.changePlan.checkoutUrl;
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
        <div className="grid grid-cols-1 gap-x-10 gap-y-[22px] lg:grid-cols-2">
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
                [t('billing.concurrency'), t('billing.workspaces', { n: plan.concurrencyLimit })],
                [t('billing.inboundEvents'), t('billing.perMinute', { n: plan.inboundEventsPerMinute })],
              ]}
            />
          </div>
        </div>
        <div className="mt-1.5 grid grid-cols-1 gap-x-10 lg:grid-cols-2">
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
            <Table>
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
            </Table>
            <Dim className="mt-3">{t('billing.footer')}</Dim>
          </>
        )}
        {membership.role === 'OWNER' ? <DeleteTenant tenantId={tenant.id} name={tenant.name} free={!admin && plan.priceUsd === 0} /> : null}
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
      <CheckoutNote outcome={checkout} success="checkout.planPaid" canceled="checkout.planCanceled" />
      {tenant ? <TenantStateNote tenant={tenant} /> : null}
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      <ActionError error={portalState.error} className="my-3" />
      <ActionError error={changeState.error} className="my-3" />
      {!data && !error ? <Skeleton rows={6} /> : null}
      {content}
    </SettingsPage>
  );
}

function DeleteTenant({ tenantId, name, free }: { tenantId: string; name: string; free: boolean }) {
  const { viewer, slug, switchTenant } = useTenant();
  const client = useApolloClient();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState('');
  const [remove, removeState] = useAction(DeleteTenantDocument);

  function openChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setTyped('');
    }
  }

  function confirm() {
    void remove({ tenantId, slug: typed.trim() }).then((done) => {
      if (!done) {
        return;
      }
      setOpen(false);
      const next = viewer.memberships.find((item) => item.tenant.id !== tenantId);
      if (next) {
        switchTenant(next.tenant.slug);
        return;
      }
      void navigate({ to: '/tenants/new' });
      void client.resetStore();
    });
  }

  return (
    <>
      <Section>{t('billing.deleteTenant')}</Section>
      <div className="flex items-center justify-between gap-6 max-md:flex-col max-md:items-start max-md:gap-2.5">
        <Dim>{t('billing.deleteTenantNote')}</Dim>
        <Button variant="danger" icon={Trash2} onPress={() => setOpen(true)}>
          {t('billing.deleteTenant')}
        </Button>
      </div>
      <ConfirmDialog
        isOpen={open}
        onOpenChange={openChange}
        title={t('billing.deleteTenantTitle', { tenant: name })}
        confirmLabel={t('billing.deleteTenant')}
        confirmIcon={Trash2}
        onConfirm={confirm}
        isConfirmDisabled={typed.trim() !== slug || removeState.loading}
        focusConfirm={false}
        error={removeState.error}
      >
        <p className="m-0">{t('billing.deleteTenantBody')}</p>
        {free ? <p className="m-0 mt-2">{t('billing.deleteTenantFree')}</p> : null}
        <label htmlFor="delete-tenant-slug" className="mb-1 mt-3 block text-xs text-muted">
          {tx('billing.deleteTenantType', { slug: <Chip>{slug}</Chip> })}
        </label>
        <input
          id="delete-tenant-slug"
          autoFocus
          value={typed}
          onChange={(event) => setTyped(event.target.value)}
          autoComplete="off"
          spellCheck={false}
          className="block h-8 w-full rounded-ui border border-line2 bg-bg px-2.5 font-mono text-sm outline-none focus:border-ink"
        />
      </ConfirmDialog>
    </>
  );
}
