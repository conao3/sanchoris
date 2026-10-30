import { useNavigate, useSearch } from '@tanstack/react-router';
import { ExternalLink, GitPullRequest, Plus, TriangleAlert } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../components/Button.tsx';
import { Cols, Page } from '../components/Shell.tsx';
import { CheckoutNote, TenantStateNote, useCheckoutOutcome } from '../components/billing.tsx';
import { TaskState } from '../components/status.tsx';
import { ActionError, Dim, ErrorNote, Id, Lk, Meter, Row, Section, Segments, Skeleton, State, Sub, Table, Th, Title, Toggle } from '../components/ui.tsx';
import {
  type HomeQuery,
  type Provider,
  HomeDocument,
  SetTriggerStateDocument,
  SnoozeTaskDocument,
} from '../graphql/generated/graphql.ts';
import { type MessageKey, t } from '../i18n/index.ts';
import { ago, day, elapsedRatio, hours, issueLabel, percent, usd } from '../lib/format.ts';
import { useListNav } from '../lib/keys.ts';
import { countOf, hoursOf, usedShare } from '../lib/plan.ts';
import { useAction } from '../lib/mutation.ts';
import { IDLE_POLL, LIVE_POLL, useData } from '../lib/query.ts';
import { limitedText, needsYouText, skillStep } from '../lib/taskText.ts';
import { useTenant } from '../lib/tenant.tsx';
import { HomeSetup } from './HomeSetup.tsx';

type Tenant = NonNullable<HomeQuery['tenant']>;
type TaskItem = Tenant['tasks'][number];
type CredentialItem = Tenant['credentials'][number];
type ChangeRequestItem = Tenant['changeRequests'][number];

type Attention =
  | { kind: 'task'; task: TaskItem }
  | { kind: 'credential'; credential: CredentialItem }
  | { kind: 'change'; change: ChangeRequestItem };

const providerKey: Record<Provider, MessageKey> = { CLAUDE: 'provider.claude', CODEX: 'provider.codex' };

function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(1, ...values);
  const step = 96 / Math.max(1, values.length - 1);
  const points = values.map((value, index) => `${Math.round(index * step)},${16 - Math.round((value / max) * 12)}`);
  const last = points[points.length - 1]?.split(',') ?? ['96', '6'];
  return (
    <svg width="96" height="18" viewBox="0 0 96 18" className="mx-1 inline align-[-4px]" aria-hidden>
      <polyline fill="none" stroke="var(--color-muted)" strokeWidth="1.5" points={points.join(' ')} />
      <circle cx={last[0]} cy={last[1]} r="2" fill="var(--color-ink)" />
    </svg>
  );
}

const stackedRow = 'max-md:grid-cols-[minmax(0,1fr)_auto]! max-md:gap-y-1 max-md:py-2 max-md:[&>:first-child]:col-span-2';

function SnoozeMenu({ taskId }: { taskId: string }) {
  const [open, setOpen] = useState(false);
  const [snooze, snoozeState] = useAction(SnoozeTaskDocument);
  const options: [MessageKey, number][] = [
    ['home.snooze1h', 1],
    ['home.snooze4h', 4],
    ['home.snooze1d', 24],
  ];
  return (
    <span className="relative">
      <button type="button" className="text-muted hover:text-ink" onClick={() => setOpen(!open)}>
        {t('home.snooze')}
      </button>
      {open ? (
        <span className="absolute right-0 top-6 z-10 flex w-40 flex-col rounded-[8px] border border-line2 bg-bg py-1.5">
          {options.map(([label, hoursAhead]) => (
            <button
              key={label}
              type="button"
              className="px-3.5 py-[7px] text-left hover:bg-hover"
              onClick={() => {
                setOpen(false);
                void snooze({ taskId, until: new Date(Date.now() + hoursAhead * 3_600_000).toISOString() });
              }}
            >
              {t(label)}
            </button>
          ))}
        </span>
      ) : null}
      <ActionError error={snoozeState.error} />
    </span>
  );
}

function UsageMeter({ title, value, pace, caption }: { title: string; value: number; pace?: number; caption: string }) {
  return (
    <>
      <Section>{title}</Section>
      <div className="mt-2.5">
        <Meter value={value} pace={pace} />
      </div>
      <Dim className="mt-2">{caption}</Dim>
    </>
  );
}

function Dashboard({ tenant }: { tenant: Tenant }) {
  const navigate = useNavigate();
  const [setTriggerState, triggerState] = useAction(SetTriggerStateDocument);
  const tasks = tenant.tasks;
  const viewerMemberId = tenant.viewerMember.id;
  const attention: Attention[] = [
    ...tasks.filter((task) => task.status === 'NEEDS_YOU' && !task.snoozedUntil).map((task) => ({ kind: 'task' as const, task })),
    ...tenant.credentials
      .filter((credential) => credential.status === 'RECONNECT')
      .map((credential) => ({ kind: 'credential' as const, credential })),
    ...tenant.changeRequests.map((change) => ({ kind: 'change' as const, change })),
  ];
  const running = tasks.filter((task) => task.status === 'RUNNING');
  const waiting = tasks.filter(
    (task) => task.status === 'PENDING' || (task.status === 'WAITING' && task.waitReason !== 'CHAT'),
  );

  function openAttention(item: Attention) {
    if (item.kind === 'task') {
      void navigate({ to: '/tasks/$task', params: { task: item.task.id } });
    } else if (item.kind === 'credential') {
      void navigate({ to: '/settings/credentials' });
    } else {
      window.open(item.change.url, '_blank', 'noreferrer');
    }
  }

  const nav = useListNav(attention.length, (index) => {
    const item = attention[index];
    if (item) {
      openAttention(item);
    }
  }, {
    c: () => void navigate({ to: '/chat' }),
  });

  const { plan, usage, activity } = tenant;
  const pace = usage.periodEnd ? elapsedRatio(usage.periodStart, usage.periodEnd) : undefined;
  const workspaceHours = hours(usage.workspaceSecondsThisPeriod);
  const providers = ['CLAUDE', 'CODEX'] as const;

  const main = (
    <>
      <Title>{t('nav.home')}</Title>
      <Sub>
        {t('home.summary', { running: activity.runningTasks, runs: activity.runsToday })}
        <Sparkline values={activity.runsLast24h} />
        <span className="text-muted">{t('home.last24h')}</span> · {usd(activity.costTodayUsd)} ·{' '}
        {t('home.prsOpened', { n: activity.pullRequestsOpenedToday })}
      </Sub>

      <Section count={attention.length} first>
        {t('status.needsYou')}
      </Section>
      {attention.length === 0 ? <Dim>{t('home.nothingNeedsYou')}</Dim> : null}
      {attention.map((item, index) => {
        const cols = '110px 1fr auto';
        const common = { cols, className: stackedRow, selected: nav.selected === index, rowRef: nav.rowRef(index), onClick: () => nav.select(index) };
        if (item.kind === 'task') {
          const { task } = item;
          return (
            <Row key={task.id} {...common}>
              <TaskState status={task.status} />
              <div>
                <span className="font-medium">{task.title}</span>{' '}
                {task.issue ? <Id>{issueLabel(task.issue.repository, task.issue.number)}</Id> : null}
                <Dim>{needsYouText(task)}</Dim>
              </div>
              <Button kbd="↵" onPress={() => openAttention(item)}>
                {t('home.openTask')}
              </Button>
            </Row>
          );
        }
        if (item.kind === 'credential') {
          const { credential } = item;
          const provider = t(providerKey[credential.provider]);
          return (
            <Row key={credential.id} {...common}>
              <State tone="err" icon={TriangleAlert}>
                {t('status.reconnect')}
              </State>
              <div>
                <span className="font-medium">{t('home.loginRejected', { provider })}</span>
                <Dim>
                  {[credential.owner?.user.name ?? tenant.name, credential.name, credential.rejectedAt ? ago(credential.rejectedAt) : null]
                    .filter(Boolean)
                    .join(' · ')}
                </Dim>
              </div>
              <Button onPress={() => openAttention(item)}>{t('home.reconnectProvider', { provider })}</Button>
            </Row>
          );
        }
        const { change } = item;
        return (
          <Row key={change.id} {...common}>
            <State tone="wait" icon={GitPullRequest}>
              {t('status.review')}
            </State>
            <div>
              <span className="font-medium">{change.title}</span>{' '}
              <Id>{issueLabel(change.config.repository, change.number)}</Id>
              <Dim>
                {[
                  t('home.changeRequest'),
                  change.validation.valid ? t('home.validationPassed') : t('home.validationFailed'),
                  change.openedFromTask ? t('home.openedFromChat') : null,
                ]
                  .filter(Boolean)
                  .join(' · ')}
              </Dim>
            </div>
            <Button icon={ExternalLink} href={change.url}>
              {t('common.reviewOnGitHub')}
            </Button>
          </Row>
        );
      })}

      <Section count={running.length}>{t('status.running')}</Section>
      {running.length === 0 ? (
        <Dim>{t('home.nothingRunning')}</Dim>
      ) : (
        <Table>
          <Th cols="110px 1fr 150px 120px 70px">
            <span>{t('col.state')}</span>
            <span>{t('col.task')}</span>
            <span>{t('col.turns')}</span>
            <span>{t('col.trigger')}</span>
            <span className="text-right">{t('col.cost')}</span>
          </Th>
          {running.map((task) => {
            const run = task.requiredSkills[0]?.latestRun;
            return (
              <Row key={task.id} cols="110px 1fr 150px 120px 70px" onClick={() => void navigate({ to: '/tasks/$task', params: { task: task.id } })}>
                <TaskState status={task.status} />
                <div>
                  <span className="font-medium">{task.title}</span>{' '}
                  {task.issue ? <Id>{issueLabel(task.issue.repository, task.issue.number)}</Id> : null}{' '}
                  {run ? <span className="text-muted">{skillStep(run)}</span> : null}
                </div>
                <Meter value={task.turns / task.limits.turns} />
                <span className="text-muted">{task.trigger.name}</span>
                <span className="text-right font-mono text-sm">{usd(task.costUsd)}</span>
              </Row>
            );
          })}
        </Table>
      )}

      <Section count={waiting.length}>{t('status.waiting')}</Section>
      {waiting.length === 0 ? <Dim>{t('home.nothingWaiting')}</Dim> : null}
      {waiting.map((task) => (
        <Row key={task.id} cols="110px 1fr auto" className={stackedRow} onClick={() => void navigate({ to: '/tasks/$task', params: { task: task.id } })}>
          <TaskState status={task.status} waitReason={task.waitReason} />
          <div>
            <span className="font-medium">{task.title}</span>{' '}
            {task.issue ? <Id>{issueLabel(task.issue.repository, task.issue.number)}</Id> : null}
            {task.waitReason === 'USAGE_LIMIT' ? <Dim>{limitedText(task)}</Dim> : null}
          </div>
          {task.waitReason === 'USAGE_LIMIT' ? (
            <span onClick={(event) => event.stopPropagation()}>
              <SnoozeMenu taskId={task.id} />
            </span>
          ) : task.blockedBy.length > 0 ? (
            <span className="text-muted">
              {t('home.blockedBy', { ids: task.blockedBy.map((blocker) => `#${blocker.issue?.number ?? ''}`).join(', ') })}
            </span>
          ) : (
            <span className="text-muted">{t(task.pendingReason === 'PLAN_LIMIT' ? 'home.planLimit' : 'home.concurrencyFull')}</span>
          )}
        </Row>
      ))}
    </>
  );

  const aside = (
    <>
      <Section first>{t('home.concurrency')}</Section>
      <Segments filled={usage.liveWorkspaces} total={plan.concurrencyLimit} />
      <Dim className="mt-1.5">{t('home.workspacesOf', { n: usage.liveWorkspaces, total: plan.concurrencyLimit })}</Dim>

      <UsageMeter
        title={t(plan.limitPeriod === 'MONTH' ? 'home.tasksThisMonth' : 'home.tasksInTotal')}
        value={usedShare(usage.tasksThisPeriod, plan.taskLimit)}
        pace={pace}
        caption={[
          countOf(usage.tasksThisPeriod, plan.taskLimit),
          plan.name,
          usage.periodEnd ? t('home.resets', { at: day(usage.periodEnd) }) : null,
        ]
          .filter(Boolean)
          .join(' · ')}
      />
      <UsageMeter
        title={t('home.workspaceHours')}
        value={usedShare(workspaceHours, plan.workspaceHourLimit)}
        pace={pace}
        caption={[
          hoursOf(workspaceHours, plan.workspaceHourLimit),
          pace !== undefined ? t('home.paceAt', { pace: percent(pace) }) : null,
        ]
          .filter(Boolean)
          .join(' · ')}
      />
      {providers.map((provider) => {
        const own = tenant.credentials.filter(
          (credential) => credential.provider === provider && (credential.owner?.id === viewerMemberId || credential.scope === 'ORGANIZATION'),
        );
        const credential = own.find((item) => item.owner?.id === viewerMemberId) ?? own[0];
        const title = t('home.weeklyUsage', { provider: t(providerKey[provider]) });
        if (!credential) {
          return null;
        }
        const week = credential.usage?.sevenDay;
        if (credential.status === 'RECONNECT' || !week) {
          return (
            <div key={provider}>
              <Section>{title}</Section>
              <Dim>{t('home.reconnectToSee', { provider: t(providerKey[provider]) })}</Dim>
            </div>
          );
        }
        const weekPace = elapsedRatio(new Date(new Date(week.resetsAt).getTime() - 7 * 86_400_000).toISOString(), week.resetsAt);
        return (
          <UsageMeter
            key={provider}
            title={title}
            value={week.utilization}
            pace={weekPace}
            caption={[
              t('home.used', { pct: percent(week.utilization) }),
              t('home.paceAt', { pace: percent(weekPace) }),
              t('home.resets', { at: day(week.resetsAt) }),
            ].join(' · ')}
          />
        );
      })}

      <Section>{t('nav.triggers')}</Section>
      {tenant.triggers.map((trigger) => (
        <Row key={trigger.id} cols="1fr auto">
          <span>
            <Lk to="/triggers">{trigger.name}</Lk>{' '}
            <span className="text-muted">
              {trigger.state === 'DISABLED'
                ? t('common.off').toLowerCase()
                : trigger.activeTasks > 0
                  ? t('home.active', { n: trigger.activeTasks })
                  : t('home.idle')}
            </span>
          </span>
          <Toggle
            on={trigger.state === 'ENABLED'}
            onToggle={() =>
              void setTriggerState({ triggerId: trigger.id, state: trigger.state === 'ENABLED' ? 'DISABLED' : 'ENABLED' })
            }
            isDisabled={triggerState.loading}
          />
        </Row>
      ))}
      <ActionError error={triggerState.error} className="mt-1.5" />
    </>
  );

  return <Cols main={main} aside={aside} />;
}

export function HomePage() {
  const { slug } = useTenant();
  const navigate = useNavigate();
  const { data, error, refetch } = useData(HomeDocument, { tenant: slug }, { poll: (result) => (result.tenant?.tasks.length ? LIVE_POLL : IDLE_POLL) });
  const tenant = data?.tenant;
  const setup = tenant && !tenant.setup.complete;
  const checkout = useCheckoutOutcome(useSearch({ from: '/app/' }).checkout);
  return (
    <Page
      crumbs={[{ label: t('nav.home') }]}
      actions={
        setup ? null : (
          <Button variant="primary" icon={Plus} kbd="c" onPress={() => void navigate({ to: '/chat' })}>
            {t('common.newChat')}
          </Button>
        )
      }
      keys={
        setup
          ? [['↵', 'keys.continue']]
          : [
              ['g h', 'nav.home'],
              ['g t', 'nav.tasks'],
              ['g r', 'nav.triggers'],
              ['c', 'common.newChat'],
            ]
      }
    >
      {error ? (
        <div className="px-4 py-6 md:px-6">
          <ErrorNote message={error.message} onRetry={refetch} />
        </div>
      ) : !tenant ? (
        <div className="px-4 py-6 md:px-6">
          <Skeleton rows={6} />
        </div>
      ) : (
        <>
          {checkout || tenant.state === 'SUSPENDED' ? (
            <div className="px-4 pt-5 md:px-6">
              <CheckoutNote outcome={checkout} success="checkout.tenantPaid" canceled="checkout.tenantCanceled" />
              <TenantStateNote tenant={tenant} />
            </div>
          ) : null}
          {setup ? <HomeSetup tenant={tenant} /> : <Dashboard tenant={tenant} />}
        </>
      )}
    </Page>
  );
}
