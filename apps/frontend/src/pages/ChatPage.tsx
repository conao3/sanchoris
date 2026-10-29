import { useMutation } from '@apollo/client/react';
import { Link, useNavigate, useParams } from '@tanstack/react-router';
import clsx from 'clsx';
import { ChevronDown, CircleDot, Plus } from 'lucide-react';
import { type KeyboardEvent, useState } from 'react';
import { Button } from '../components/Button';
import { Menu } from '../components/Menu';
import { Page } from '../components/Shell';
import { TaskState } from '../components/status';
import { Chip, Dim, ErrorNote, Icon, Id, Kbd, Lk, Row, Section, Skeleton, Sub, Th, Title } from '../components/ui';
import { type ChatQuery, ChatDocument, SendChatMessageDocument } from '../graphql/generated/graphql';
import { locale, t, tx } from '../i18n';
import { clock } from '../lib/format';
import { useKeys } from '../lib/keys';
import { IDLE_POLL, LIVE_POLL, useData } from '../lib/query';
import { isActive, skillStep } from '../lib/taskText';
import { useTenant } from '../lib/tenant';

type Tenant = NonNullable<ChatQuery['tenant']>;

const andList = new Intl.ListFormat(locale, { type: 'conjunction' });
type ChatTask = NonNullable<Tenant['task']>;

function shortRepo(repository: string): string {
  return repository.split('/').pop() ?? repository;
}

function scopeLabel(scope: string): string {
  return scope === 'all' ? t('chat.allProjects') : t('chat.project', { name: scope });
}

function scopeShort(scope: string): string {
  return scope === 'all' ? t('chat.allProjects') : scope;
}

function scopes(tenant: Tenant) {
  return tenant.triggers
    .filter((trigger) => trigger.state === 'ENABLED' && trigger.match.source === 'WEB' && trigger.match.chatScope)
    .map((trigger) => {
      const scope = trigger.match.chatScope ?? '';
      const repos = trigger.session.repositories;
      return {
        value: scope,
        label: scopeLabel(scope),
        detail:
          scope === 'all'
            ? t('chat.scopeAllDetail', { repos: repos.join(', ') })
            : t('chat.scopeProjectDetail', { repos: andList.format(repos.map(shortRepo)) }),
      };
    });
}

function Composer({ disabled, onSend }: { disabled: boolean; onSend: (text: string) => Promise<unknown> }) {
  const [text, setText] = useState('');

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      if (text.trim() && !disabled) {
        void onSend(text.trim()).then(() => setText(''));
      }
    }
  }

  return (
    <div className="mt-3.5 flex items-start justify-between gap-3 rounded-ui border border-line2 px-2.5 py-2">
      <textarea
        rows={1}
        value={text}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder={t('chat.placeholder')}
        aria-label={t('chat.placeholder')}
        className="min-h-5 flex-1 resize-none bg-transparent outline-none placeholder:text-faint"
      />
      <Kbd>{t('chat.enterSend')}</Kbd>
    </div>
  );
}

function Thread({ task, tenant }: { task: ChatTask; tenant: Tenant }) {
  const [send, { error }] = useMutation(SendChatMessageDocument, { refetchQueries: [ChatDocument] });
  const repos = task.session.repositories.join(', ');
  const run = task.runs[task.runs.length - 1];
  const requester = task.requester?.member?.user.name;
  return (
    <>
      <Title>{task.title}</Title>
      <Sub>
        <TaskState status={task.status} waitReason={task.waitReason} />{' '}
        <span className="ml-2">
          {tx('chat.threadLine', {
            trigger: <Lk to="/triggers">{task.trigger.name}</Lk>,
            scope: task.channel ? scopeLabel(task.channel.scope).toLowerCase() : '',
            session: <span className="font-mono text-sm">{task.session.name}</span>,
            repos: <span className="font-mono text-sm">{repos}</span>,
            run: run ? t('run.title', { n: run.number }) : '—',
          })}
          {requester ? ` · ${t(task.channel?.kind === 'SLACK' ? 'chat.startedBySlack' : 'chat.startedByWeb', { name: requester })}` : null}
        </span>
      </Sub>
      {task.messages.map((message) => (
        <div key={message.id} className="border-b border-line py-2.5">
          <div className="mb-0.5 text-sm font-semibold">
            {message.authorKind === 'AGENT' ? 'sanchoris' : (message.member?.user.name ?? '')}
            <span className="ml-1.5 font-normal text-faint">{clock(message.at)}</span>
          </div>
          <div className="whitespace-pre-wrap">{message.text}</div>
          {message.issues.map((issue) => {
            const blockers = issue.task?.blockedBy ?? [];
            return (
              <div key={`${issue.repository}#${issue.number}`} className="mt-1.5 grid grid-cols-[18px_auto_1fr_auto] items-center gap-2 border-t border-line py-1.5">
                <Icon icon={CircleDot} />
                <span>
                  <Chip>{shortRepo(issue.repository)}</Chip>
                </span>
                {issue.task ? (
                  <Link to="/tasks/$task" params={{ task: issue.task.id }} className="font-medium">
                    {issue.title}
                  </Link>
                ) : (
                  <span className="font-medium">{issue.title}</span>
                )}
                <Id>
                  #{issue.number}
                  {blockers.length > 0 ? ` · ${t('tasks.blockedBy', { ids: blockers.map((blocker) => `#${blocker.issue?.number ?? ''}`).join(', ') })}` : null}
                </Id>
              </div>
            );
          })}
        </div>
      ))}
      <Composer
        disabled={task.status === 'SUCCEEDED' && task.channel?.kind === 'SLACK'}
        onSend={(text) => send({ variables: { input: { tenantId: tenant.id, taskId: task.id, text } } })}
      />
      {error ? <Dim className="mt-2 text-err">{error.message}</Dim> : null}
    </>
  );
}

function NewChat({ tenant, scope, onPickScope }: { tenant: Tenant; scope?: string; onPickScope: () => void }) {
  const navigate = useNavigate();
  const [send, { error }] = useMutation(SendChatMessageDocument, { refetchQueries: [ChatDocument] });
  const options = scopes(tenant);
  const option = options.find((item) => item.value === scope);
  return (
    <>
      <Title>{t('chat.newChat')}</Title>
      <Sub>
        {option ? (
          <>
            {t('chat.scopeIs', { scope: option.label })} · {option.detail}
          </>
        ) : options.length === 0 ? (
          <>
            {t('chat.noScopes')} <Lk to="/triggers">{t('nav.triggers')}</Lk>
          </>
        ) : (
          <button type="button" className="underline decoration-line2 underline-offset-[3px]" onClick={onPickScope}>
            {t('chat.pickScope')}
          </button>
        )}
      </Sub>
      <Composer
        disabled={!scope}
        onSend={(text) =>
          send({ variables: { input: { tenantId: tenant.id, scope, text } } }).then((result) => {
            const task = result.data?.sendChatMessage.task;
            if (task) {
              void navigate({ to: '/chat/$task', params: { task: task.id } });
            }
          })
        }
      />
      {error ? <Dim className="mt-2 text-err">{error.message}</Dim> : null}
    </>
  );
}

function FiledTasks({ task }: { task: ChatTask }) {
  return (
    <>
      <Section count={task.filedTasks.length} first>
        {t('chat.issuesFiled')}
      </Section>
      <Dim className="mb-1.5">{t('chat.issuesFiledDetail')}</Dim>
      {task.filedTasks.length === 0 ? <Dim>{t('chat.noIssues')}</Dim> : null}
      {task.filedTasks.length > 0 ? (
        <>
          <Th cols="96px 1fr">
            <span>{t('col.state')}</span>
            <span>{t('col.task')}</span>
          </Th>
          {task.filedTasks.map((filed) => {
            const run = filed.requiredSkills[0]?.latestRun;
            const details = [
              filed.parent?.issue ? t('triggers.childOf', { id: `#${filed.parent.issue.number}` }) : filed.trigger.name,
              filed.blockedBy.length > 0
                ? t('tasks.blockedBy', { ids: filed.blockedBy.map((blocker) => `#${blocker.issue?.number ?? ''}`).join(', ') })
                : run
                  ? skillStep(run)
                  : null,
            ].filter(Boolean);
            return (
              <Row key={filed.id} cols="96px 1fr" tall>
                <TaskState status={filed.status} waitReason={filed.waitReason} />
                <div>
                  <Lk to="/tasks/$task" params={{ task: filed.id }} className="font-medium no-underline">
                    {filed.title}
                  </Lk>{' '}
                  {filed.issue ? <Id>#{filed.issue.number}</Id> : null}
                  <Dim>{details.join(' · ')}</Dim>
                </div>
              </Row>
            );
          })}
        </>
      ) : null}
    </>
  );
}

export function ChatPage() {
  const params = useParams({ strict: false });
  const taskId = params.task;
  const { slug } = useTenant();
  const navigate = useNavigate();
  const { data, error, refetch } = useData(
    ChatDocument,
    { tenant: slug, task: taskId ?? '', withTask: !!taskId },
    { poll: (result) => (!taskId ? IDLE_POLL : result.tenant?.task && isActive(result.tenant.task.status) ? LIVE_POLL : 0) },
  );
  const [scope, setScope] = useState<string>();
  const [scopeOpen, setScopeOpen] = useState(!taskId);
  const tenant = data?.tenant;
  const task = tenant?.task;

  function newChat() {
    setScope(undefined);
    setScopeOpen(true);
    void navigate({ to: '/chat' });
  }

  useKeys({ c: newChat });

  return (
    <Page
      crumbs={[{ label: t('nav.chat'), to: '/chat' }, { label: taskId ? (task?.title ?? '…') : t('chat.newChat') }]}
      actions={
        <>
          <Button variant="primary" icon={Plus} kbd="c" onPress={newChat}>
            {t('common.newChat')}
          </Button>
          {tenant ? (
            <Menu
              icon={ChevronDown}
              label={t('chat.scope')}
              title={t('chat.startIn')}
              align="right"
              open={scopeOpen && scopes(tenant).length > 0}
              onOpenChange={setScopeOpen}
              value={scope}
              options={scopes(tenant)}
              onChange={(value) => {
                setScope(value);
                if (taskId) {
                  void navigate({ to: '/chat' });
                }
              }}
            />
          ) : null}
        </>
      }
      keys={[
        ['↵', 'chat.send'],
        ['⇧↵', 'chat.newLine'],
      ]}
    >
      {error ? (
        <div className="p-6">
          <ErrorNote message={error.message} onRetry={refetch} />
        </div>
      ) : !tenant ? (
        <div className="p-6">
          <Skeleton rows={6} />
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-[260px_1fr_340px]">
          <div className="min-h-0 overflow-auto px-5 py-[18px]">
            <Section count={tenant.chats.length} first>
              {t('chat.threads')}
            </Section>
            {tenant.chats.length === 0 ? <Dim>{t('chat.noThreads')}</Dim> : null}
            {tenant.chats.map((chat) => (
              <Link
                key={chat.id}
                to="/chat/$task"
                params={{ task: chat.id }}
                className={clsx('block rounded-ui px-2 py-[7px] hover:bg-hover', chat.id === taskId && 'bg-hover')}
              >
                <span className="font-medium">{chat.title}</span>
                <small className="block text-xs text-muted">
                  {[
                    chat.channel ? scopeShort(chat.channel.scope) : null,
                    chat.channel?.kind === 'SLACK' ? t('chat.slack') : t('chat.web'),
                    t(chat.filedTasks.length === 1 ? 'chat.issueCountOne' : 'chat.issuesCount', { n: chat.filedTasks.length }),
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </small>
              </Link>
            ))}
          </div>
          <div className="min-h-0 overflow-auto border-l border-line px-5 py-[18px]">
            {taskId ? (
              task ? (
                <Thread task={task} tenant={tenant} />
              ) : (
                <Dim>{t('chat.notFound')}</Dim>
              )
            ) : (
              <NewChat tenant={tenant} scope={scope} onPickScope={() => setScopeOpen(true)} />
            )}
          </div>
          <div className="min-h-0 overflow-auto border-l border-line px-5 py-[18px]">
            {task ? <FiledTasks task={task} /> : <Dim>{t('chat.filedHint')}</Dim>}
          </div>
        </div>
      )}
    </Page>
  );
}
