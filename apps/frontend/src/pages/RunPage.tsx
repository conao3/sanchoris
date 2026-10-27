import { useMutation } from '@apollo/client/react';
import { useNavigate, useParams } from '@tanstack/react-router';
import clsx from 'clsx';
import { Square } from 'lucide-react';
import { type FormEvent, useEffect, useRef, useState } from 'react';
import { Button } from '../components/Button';
import { Cols, Page } from '../components/Shell';
import { RunState, SkillRunState } from '../components/status';
import { Dim, Empty, ErrorNote, ExternalLk, Kv, Lk, Row, Section, Skeleton, Sub, Title } from '../components/ui';
import { type LogLineKind, RunDocument, SendChatMessageDocument, StopRunDocument } from '../graphql/generated/graphql';
import { type MessageKey, t } from '../i18n';
import { clock } from '../lib/format';
import { useKeys } from '../lib/keys';
import { useData } from '../lib/query';
import { useTenant } from '../lib/tenant';

const lineClass: Record<LogLineKind, string> = {
  SYSTEM: '',
  INSTRUCTION: 'bg-term-instruction-bg text-term-instruction',
  AGENT: 'text-white',
  TOOL: '',
  EVENT: 'text-term-event',
  USER: 'text-term-user',
  ERROR: 'text-term-error',
};

const linePrefix: Partial<Record<LogLineKind, MessageKey>> = {
  INSTRUCTION: 'run.prefixSanchoris',
  AGENT: 'run.prefixAgent',
  TOOL: 'run.prefixTool',
  USER: 'run.prefixYou',
};

export function RunPage() {
  const { task: taskId, n } = useParams({ from: '/app/tasks/$task/runs/$n' });
  const { slug } = useTenant();
  const navigate = useNavigate();
  const [follow, setFollow] = useState(true);
  const { data, error, refetch } = useData(RunDocument, { tenant: slug, task: taskId, number: Number(n) }, { pollInterval: follow ? 5000 : 0 });
  const [stopRun, stopState] = useMutation(StopRunDocument);
  const [send, sendState] = useMutation(SendChatMessageDocument);
  const [text, setText] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const logEnd = useRef<HTMLDivElement>(null);
  const task = data?.tenant?.task;
  const run = task?.run;
  const lines = run?.log.length ?? 0;

  useEffect(() => {
    if (follow) {
      logEnd.current?.scrollIntoView({ block: 'nearest' });
    }
  }, [follow, lines]);

  const live = run && (run.status === 'RUNNING' || run.status === 'IDLE' || run.status === 'CREATING');

  function submit(event?: FormEvent) {
    event?.preventDefault();
    const tenantId = data?.tenant?.id;
    if (!task || !tenantId || !text.trim()) {
      return;
    }
    void send({ variables: { input: { tenantId, taskId: task.id, text: text.trim() } } }).then(() => {
      setText('');
      refetch();
    });
  }

  useKeys({
    Escape: () => void navigate({ to: '/tasks/$task', params: { task: taskId } }),
    f: () => setFollow(!follow),
    m: () => input.current?.focus(),
    s: () => {
      if (run && live) {
        void stopRun({ variables: { runId: run.id } });
      }
    },
  });

  let body;
  if (error) {
    body = (
      <div className="p-6">
        <ErrorNote message={error.message} onRetry={refetch} />
      </div>
    );
  } else if (!data) {
    body = (
      <div className="p-6">
        <Skeleton rows={8} />
      </div>
    );
  } else if (!task || !run) {
    body = (
      <div className="p-6">
        <Empty>{t('run.notFound')}</Empty>
      </div>
    );
  } else {
    const agent = t(run.agent === 'CODEX' ? 'provider.codex' : 'provider.claude');
    const main = (
      <div className="flex h-full min-h-[480px] flex-col">
        <Title>{t('run.title', { n: run.number })}</Title>
        <Sub>
          <RunState status={run.status} />{' '}
          <span className="ml-2">
            {t('run.sessionLine', { session: run.sessionName, agent, model: run.model, at: clock(run.startedAt) })}{' '}
            <Lk to="/tasks/$task" params={{ task: task.id }}>
              {task.issue ? `#${task.issue.number} ` : ''}
              {task.title}
            </Lk>
          </span>
        </Sub>
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-ui bg-term-bg py-3 font-mono text-sm leading-[1.75] text-term-fg">
          <div className="min-h-0 flex-1 overflow-auto">
            {run.log.map((line, index) => {
              const [first, ...rest] = line.text.split('\n');
              const prefix = linePrefix[line.kind];
              const current = index === run.log.length - 1;
              return (
                <div key={line.sequence} className={clsx(lineClass[line.kind], current && 'bg-term-line')}>
                  <div className="whitespace-pre-wrap px-4">
                    <span className="text-muted">{clock(line.at)}</span> {prefix ? `${t(prefix)} ` : ''}
                    {first}
                  </div>
                  {rest.map((continuation, offset) => (
                    <div key={offset} className="whitespace-pre-wrap px-4 pl-[calc(1rem+6ch)]">
                      {continuation}
                    </div>
                  ))}
                </div>
              );
            })}
            <div ref={logEnd} />
          </div>
          <form onSubmit={submit} className="mt-2.5 flex items-center justify-between gap-3 border-t border-term-line px-4 pt-2.5 text-muted">
            <input
              ref={input}
              value={text}
              onChange={(event) => setText(event.target.value)}
              placeholder={t('run.messagePlaceholder')}
              aria-label={t('run.messagePlaceholder')}
              disabled={!live || sendState.loading}
              className="min-w-0 flex-1 bg-transparent text-term-fg outline-none placeholder:text-muted"
            />
            <span>{t('run.enterSend')}</span>
          </form>
        </div>
        {sendState.error ? <Dim className="mt-2 text-err">{sendState.error.message}</Dim> : null}
      </div>
    );
    const prs = run.outputs.filter((output) => output.kind === 'PULL_REQUEST');
    const aside = (
      <>
        <Section first>{t('run.session')}</Section>
        <Kv
          items={[
            [t('run.session'), <span className="font-mono text-sm">{run.sessionName}</span>],
            [t('run.worker'), <>{agent} · <span className="font-mono text-sm">{run.model}</span></>],
            [t('run.credential'), <span className="font-mono text-sm">{run.credentialName}</span>],
            [t('run.repos'), <span className="font-mono text-sm">{run.workspace.repositories.join(', ')}</span>],
            [t('run.branch'), <span className="font-mono text-sm">{run.workspace.branch}</span>],
            [t('run.workspace'), t('run.workspaceSize', { platform: run.workspace.platform, vcpu: run.workspace.vcpu })],
          ]}
        />
        <Section count={run.skillRuns.length}>{t('run.skillsCalled')}</Section>
        {run.skillRuns.length === 0 ? <Dim>—</Dim> : null}
        {run.skillRuns.map((skillRun) => (
          <Row key={skillRun.id} cols="1fr auto">
            <span>
              <span className="font-mono font-medium">{skillRun.skill.name}</span>{' '}
              <span className="text-muted">{t('run.fixRounds', { n: skillRun.handoffs })}</span>
            </span>
            <SkillRunState status={skillRun.status} />
          </Row>
        ))}
        <Section>{t('run.sessionTotal')}</Section>
        <Kv
          items={[
            [t('col.turns'), <span className="font-mono text-sm">{run.turns}</span>],
            [t('col.cost'), <span className="font-mono text-sm">${run.costUsd.toFixed(2)}</span>],
          ]}
        />
        <Section>{t('run.output')}</Section>
        {prs.length === 0 ? <Dim>—</Dim> : null}
        {prs.map((output) => (
          <div key={output.id}>
            <ExternalLk href={output.url}>
              {output.repository}#{output.number}
            </ExternalLk>
            , {t(`output.${output.state.toLowerCase()}` as MessageKey)}
          </div>
        ))}
      </>
    );
    body = <Cols main={main} aside={aside} />;
  }

  return (
    <Page
      crumbs={[
        { label: t('nav.tasks'), to: '/tasks' },
        { label: task?.title ?? '…', to: '/tasks/$task', params: { task: taskId } },
        { label: t('run.title', { n }) },
      ]}
      actions={
        <>
          <Button variant="danger" icon={Square} kbd="s" isDisabled={!live || stopState.loading} onPress={() => run && void stopRun({ variables: { runId: run.id } })}>
            {t('run.stop')}
          </Button>
          <Button kbd="m" isDisabled={!live} onPress={() => input.current?.focus()}>
            {t('run.sendMessage')}
          </Button>
        </>
      }
      keys={[
        ['Esc', 'keys.back'],
        ['f', follow ? 'run.followOff' : 'run.follow'],
        ['m', 'run.sendMessage'],
      ]}
    >
      {body}
    </Page>
  );
}
