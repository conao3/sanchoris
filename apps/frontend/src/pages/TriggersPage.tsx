import type { LucideIcon } from 'lucide-react';
import { CircleDot, GitPullRequest, MessageSquare, Plus, Timer } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { Button } from '../components/Button';
import { Page, Pg } from '../components/Shell';
import { ActionError, Dim, Empty, ErrorNote, ExternalLk, Icon, Id, Lk, Row, Section, Skeleton, State, Sub, Th, Title, Toggle } from '../components/ui';
import {
  ProposeTriggerDocument,
  SetTriggerStateDocument,
  type TriggersQuery,
  TriggersDocument,
} from '../graphql/generated/graphql';
import { locale, t, tx } from '../i18n';
import { weekdayTime, when } from '../lib/format';
import { useListNav } from '../lib/keys';
import { useAction } from '../lib/mutation';
import { useData } from '../lib/query';
import { useTenant } from '../lib/tenant';

type Tenant = NonNullable<TriggersQuery['tenant']>;
type Trigger = Tenant['triggers'][number];
type TriggerEvent = Tenant['triggerEvents'][number];

const orList = new Intl.ListFormat(locale, { type: 'disjunction' });
const andList = new Intl.ListFormat(locale, { type: 'conjunction' });

function shortRepo(repository: string): string {
  return repository.split('/').pop() ?? repository;
}

function scopeLabel(scope: string): string {
  return scope === 'all' ? t('chat.allProjects') : t('chat.project', { name: scope });
}

function startsWhen(trigger: Trigger): { icon: LucideIcon; text: string } {
  const { match, limits } = trigger;
  const children = t('triggers.children', { depth: limits.childrenMaxDepth, count: limits.childrenMaxCount });
  switch (match.kind) {
    case 'ISSUE_LABELED':
      return {
        icon: CircleDot,
        text: `${t('triggers.issueLabeled', { labels: orList.format(match.labels), senders: orList.format(match.senders) })} ${children}`,
      };
    case 'ISSUE_COMMENT_CREATED':
      return { icon: CircleDot, text: `${t('triggers.issueComment', { senders: orList.format(match.senders) })} ${children}` };
    case 'CHECK_RUN_COMPLETED':
      return { icon: CircleDot, text: t('triggers.checkRun', { repos: andList.format(match.repositories.map(shortRepo)) }) };
    case 'MESSAGE':
      if (match.source === 'SLACK') {
        return { icon: MessageSquare, text: t('triggers.slackMessage', { channel: match.slackChannel ?? '' }) };
      }
      return { icon: MessageSquare, text: t('triggers.webMessage', { scope: scopeLabel(match.chatScope ?? 'all') }) };
    case 'SCHEDULE':
      return {
        icon: Timer,
        text: match.nextRunAt ? t('triggers.schedule', { next: weekdayTime(match.nextRunAt) }) : (match.scheduleExpression ?? ''),
      };
  }
}

function eventResult(event: TriggerEvent, triggers: Trigger[]) {
  const trigger = triggers.find((item) => item.id === event.trigger.id);
  if (event.result === 'STARTED' && event.task) {
    const { task } = event;
    const label = task.issue ? `#${task.issue.number} ${task.title}` : task.title;
    const details = [
      event.senderLogin && task.issue ? t('triggers.labeledBy', { login: event.senderLogin }) : null,
      task.parent?.issue ? t('triggers.childOf', { id: `#${task.parent.issue.number}` }) : null,
    ].filter(Boolean);
    return (
      <span>
        {tx('triggers.started', { task: <Lk to="/tasks/$task" params={{ task: task.id }}>{label}</Lk> })}
        {details.length > 0 ? <span className="text-muted"> · {details.join(', ')}</span> : null}
      </span>
    );
  }
  const limited = event.limitedTask?.issue ? `#${event.limitedTask.issue.number}` : '';
  const reason = (() => {
    switch (event.reason) {
      case 'CHILD_LIMIT':
        return t('triggers.childLimit', { id: limited, n: trigger?.limits.childrenMaxCount ?? 0 });
      case 'SENDER_NOT_ALLOWED':
        return trigger?.match.kind === 'ISSUE_LABELED'
          ? t('triggers.labelNotAllowed', { login: event.senderLogin ?? '' })
          : t('triggers.senderNotAllowed', { login: event.senderLogin ?? '' });
      case 'RECEIVE_LIMIT':
        return t('triggers.receiveLimit');
      case 'DUPLICATE':
        return t('triggers.duplicate');
      default:
        return '';
    }
  })();
  return (
    <span className="text-muted">
      {event.result === 'DISCARDED' ? t('triggers.discarded') : t('triggers.skipped')} · {reason}
    </span>
  );
}

function TemplatePanel({ tenant, templates, onClose }: { tenant: Tenant; templates: TriggersQuery['triggerTemplates']; onClose: () => void }) {
  const repositories = tenant.integrations.flatMap((integration) => (integration.kind === 'GITHUB_INSTALLATION' ? integration.repositories : []));
  const [templateId, setTemplateId] = useState(templates[0]?.id ?? '');
  const [configId, setConfigId] = useState(tenant.configs[0]?.id ?? '');
  const [repository, setRepository] = useState(repositories[0] ?? '');
  const [propose, { data, loading, error }] = useAction(ProposeTriggerDocument, { refetchQueries: [TriggersDocument] });
  const selectClass = 'h-[30px] w-full rounded-ui border border-line2 bg-bg px-2.5';

  function submit(event: FormEvent) {
    event.preventDefault();
    void propose({ input: { tenantId: tenant.id, templateId, configId, repository } });
  }

  return (
    <form onSubmit={submit} className="absolute right-6 top-[46px] z-10 w-[380px] rounded-[8px] border border-line2 bg-bg p-3.5">
      <div className="mb-1 text-xs font-semibold uppercase tracking-[.06em] text-faint">{t('triggers.newFromTemplate')}</div>
      <label className="mb-1 mt-2 block text-xs text-muted" htmlFor="template">
        {t('triggers.template')}
      </label>
      <select id="template" className={selectClass} value={templateId} onChange={(event) => setTemplateId(event.target.value)}>
        {templates.map((template) => (
          <option key={template.id} value={template.id}>
            {template.description}
          </option>
        ))}
      </select>
      <label className="mb-1 mt-2 block text-xs text-muted" htmlFor="repository">
        {t('triggers.repository')}
      </label>
      <select id="repository" className={selectClass} value={repository} onChange={(event) => setRepository(event.target.value)}>
        {repositories.map((item) => (
          <option key={item}>{item}</option>
        ))}
      </select>
      <label className="mb-1 mt-2 block text-xs text-muted" htmlFor="config">
        {t('triggers.config')}
      </label>
      <select id="config" className={selectClass} value={configId} onChange={(event) => setConfigId(event.target.value)}>
        {tenant.configs.map((config) => (
          <option key={config.id} value={config.id}>
            {config.repository}
          </option>
        ))}
      </select>
      <ActionError error={error} className="mt-2" />
      {data ? (
        <Dim className="mt-2">
          {tx('triggers.proposed', {
            link: <ExternalLk href={data.proposeTriggerFromTemplate.url}>#{data.proposeTriggerFromTemplate.number}</ExternalLk>,
          })}
        </Dim>
      ) : null}
      <div className="mt-3 flex gap-2">
        <Button type="submit" variant="primary" icon={GitPullRequest} isDisabled={loading || !templateId || !configId}>
          {t('triggers.openChangeRequest')}
        </Button>
        <Button onPress={onClose}>{t('common.close')}</Button>
      </div>
    </form>
  );
}

export function TriggersPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(TriggersDocument, { tenant: slug });
  const [setTriggerState, triggerState] = useAction(SetTriggerStateDocument);
  const [panel, setPanel] = useState(false);
  const tenant = data?.tenant;
  const triggers = tenant?.triggers ?? [];

  function toggle(trigger: Trigger) {
    void setTriggerState({ triggerId: trigger.id, state: trigger.state === 'ENABLED' ? 'DISABLED' : 'ENABLED' });
  }

  const nav = useListNav(triggers.length, (index) => window.open(triggers[index].editUrl, '_blank', 'noreferrer'), {
    n: () => setPanel(true),
    t: () => {
      const trigger = triggers[nav.selected];
      if (trigger) {
        toggle(trigger);
      }
    },
    Escape: () => setPanel(false),
  });

  const openChanges = new Set(triggers.flatMap((trigger) => (trigger.openChangeRequest ? [trigger.openChangeRequest.id] : []))).size;
  const cols = '180px minmax(240px,1fr) 190px 140px 60px 130px 60px';

  return (
    <Page
      crumbs={[{ label: t('nav.triggers') }]}
      actions={
        <Button variant="primary" icon={Plus} kbd="n" onPress={() => setPanel(!panel)}>
          {t('triggers.newFromTemplate')}
        </Button>
      }
      keys={[
        ['j k', 'keys.move'],
        ['↵', 'keys.openOnGitHub'],
        ['n', 'triggers.newFromTemplate'],
        ['t', 'keys.onOff'],
      ]}
    >
      <div className="relative min-h-0 flex-1">
        {panel && tenant && data ? <TemplatePanel tenant={tenant} templates={data.triggerTemplates} onClose={() => setPanel(false)} /> : null}
        <Pg className="h-full">
          <Title>{t('nav.triggers')}</Title>
          <Sub>{t('triggers.intro', { n: triggers.length, changes: openChanges })}</Sub>
          {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
          <ActionError error={triggerState.error} className="mb-2" />
          {!tenant && !error ? <Skeleton rows={5} /> : null}
          {tenant && triggers.length === 0 ? <Empty>{t('triggers.empty')}</Empty> : null}
          {triggers.length > 0 ? (
            <>
              <Th cols={cols}>
                <span>{t('col.trigger')}</span>
                <span>{t('col.startsWhen')}</span>
                <span>{t('col.session')}</span>
                <span>{t('col.requiredSkill')}</span>
                <span className="text-right">{t('col.tasks7d')}</span>
                <span>{t('col.config')}</span>
                <span className="text-right">{t('col.on')}</span>
              </Th>
              {triggers.map((trigger, index) => {
                const starts = startsWhen(trigger);
                const showRepos = trigger.match.kind === 'MESSAGE' && trigger.session.repositories.length > 0;
                return (
                  <Row key={trigger.id} cols={cols} tall selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                    <div className="flex flex-col items-start gap-0.5">
                      <ExternalLk href={trigger.editUrl} className="font-medium no-underline">
                        {trigger.name}
                      </ExternalLk>
                      {trigger.openChangeRequest ? (
                        <State tone="wait" icon={GitPullRequest}>
                          {t('triggers.changeRequestOpen', { n: trigger.openChangeRequest.number })}
                        </State>
                      ) : null}
                    </div>
                    <span>
                      <Icon icon={starts.icon} /> {starts.text}
                    </span>
                    <span>
                      <span className="font-mono text-sm">{trigger.session.name}</span> ·{' '}
                      {t(trigger.session.agent === 'CODEX' ? 'provider.codex' : 'provider.claude')}
                      {showRepos ? ` · ${trigger.session.repositories.map(shortRepo).join(', ')}` : null}
                    </span>
                    {trigger.requiredSkills.length > 0 ? (
                      <span className="font-mono text-sm">{trigger.requiredSkills.map((skill) => skill.name).join(', ')}</span>
                    ) : (
                      <span className="text-muted">—</span>
                    )}
                    <span className="text-right font-mono text-sm">{trigger.tasksLast7d}</span>
                    <span className="font-mono text-sm text-muted">{shortRepo(trigger.config.repository)}</span>
                    <span className="text-right">
                      <Toggle on={trigger.state === 'ENABLED'} onToggle={() => toggle(trigger)} isDisabled={triggerState.loading} />
                    </span>
                  </Row>
                );
              })}
            </>
          ) : null}

          <Section count={t('triggers.allTriggers')}>{t('triggers.recentEvents')}</Section>
          {tenant && tenant.triggerEvents.length === 0 ? <Dim>{t('triggers.noEvents')}</Dim> : null}
          {tenant && tenant.triggerEvents.length > 0 ? (
            <>
              <Th cols="64px 150px 1fr">
                <span>{t('col.time')}</span>
                <span>{t('col.trigger')}</span>
                <span>{t('col.result')}</span>
              </Th>
              {tenant.triggerEvents.map((event) => (
                <Row key={event.id} cols="64px 150px 1fr">
                  <Id>{when(event.receivedAt)}</Id>
                  <span className="font-mono text-sm">{event.trigger.name}</span>
                  {eventResult(event, triggers)}
                </Row>
              ))}
            </>
          ) : null}
          <Dim className="mt-3.5">{t('triggers.footer')}</Dim>
        </Pg>
      </div>
    </Page>
  );
}
