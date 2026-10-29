import { useNavigate } from '@tanstack/react-router';
import clsx from 'clsx';
import { Plus, Puzzle, Zap } from 'lucide-react';
import { useState } from 'react';
import { Button } from '../components/Button';
import { Menu } from '../components/Menu';
import { Page, Pg } from '../components/Shell';
import { TaskState } from '../components/status';
import { Empty, ErrorNote, Id, Row, Skeleton, Sub, Th, Title } from '../components/ui';
import { type TaskFilter, type TaskRowFragment, type TaskStatus, TasksDocument } from '../graphql/generated/graphql';
import { t } from '../i18n';
import { age, issueLabel, usd } from '../lib/format';
import { useListNav } from '../lib/keys';
import { IDLE_POLL, LIVE_POLL, useData } from '../lib/query';
import { isActive, skillProgress } from '../lib/taskText';
import { useTenant } from '../lib/tenant';

type StateFilter = 'all' | 'active' | 'needsYou' | 'finished';

const stateFilters: Record<StateFilter, TaskFilter> = {
  all: {},
  active: { active: true },
  needsYou: { statuses: ['NEEDS_YOU'] },
  finished: { statuses: ['SUCCEEDED', 'FAILED', 'ABORTED'] },
};

const stateLabels = {
  all: 'tasks.stateAll',
  active: 'tasks.stateActive',
  needsYou: 'tasks.stateNeedsYou',
  finished: 'tasks.stateFinished',
} as const;

function rank(status: TaskStatus): number {
  return status === 'NEEDS_YOU' ? 0 : 1;
}

function asTree(tasks: TaskRowFragment[]): { task: TaskRowFragment; child: boolean }[] {
  const ids = new Set(tasks.map((task) => task.id));
  const roots = tasks
    .filter((task) => !task.parent || !ids.has(task.parent.id))
    .sort((a, b) => rank(a.status) - rank(b.status) || b.updatedAt.localeCompare(a.updatedAt));
  return roots.flatMap((root) => [
    { task: root, child: false },
    ...tasks.filter((task) => task.parent?.id === root.id).map((task) => ({ task, child: true })),
  ]);
}

export function TasksPage() {
  const { slug } = useTenant();
  const navigate = useNavigate();
  const [triggerId, setTriggerId] = useState('');
  const [skillName, setSkillName] = useState('');
  const [state, setState] = useState<StateFilter>('all');
  const [filterOpen, setFilterOpen] = useState(false);
  const filter: TaskFilter = {
    ...stateFilters[state],
    ...(triggerId ? { triggerId } : {}),
    ...(skillName ? { skillName } : {}),
  };
  const { data, error, refetch } = useData(
    TasksDocument,
    { tenant: slug, filter },
    { poll: (result) => (result.tenant?.tasks.some((task) => isActive(task.status)) ? LIVE_POLL : IDLE_POLL) },
  );
  const tenant = data?.tenant;
  const rows = asTree(tenant?.tasks ?? []);

  function open(index: number) {
    void navigate({ to: '/tasks/$task', params: { task: rows[index].task.id } });
  }

  const nav = useListNav(rows.length, open, {
    c: () => void navigate({ to: '/chat' }),
    '/': () => setFilterOpen(true),
    Escape: () => setFilterOpen(false),
  });
  const cols = '170px minmax(220px,1fr) 110px 250px 64px 60px';
  const triggerName = tenant?.triggers.find((trigger) => trigger.id === triggerId)?.name;

  return (
    <Page
      crumbs={[{ label: t('nav.tasks') }]}
      actions={
        <Button variant="primary" icon={Plus} kbd="c" onPress={() => void navigate({ to: '/chat' })}>
          {t('common.newChat')}
        </Button>
      }
      keys={[
        ['j k', 'keys.move'],
        ['↵', 'keys.open'],
        ['/', 'keys.filter'],
        ['c', 'common.newChat'],
      ]}
    >
      <Pg>
        <Title>{t('nav.tasks')}</Title>
        <Sub>{t('tasks.intro', { n: rows.length })}</Sub>
        <div className="mb-3.5 flex gap-2">
          <Menu
            icon={Zap}
            open={filterOpen}
            onOpenChange={setFilterOpen}
            label={triggerName ?? t('tasks.allTriggers')}
            value={triggerId}
            onChange={setTriggerId}
            options={[{ value: '', label: t('tasks.allTriggers') }, ...(tenant?.triggers ?? []).map((trigger) => ({ value: trigger.id, label: trigger.name }))]}
          />
          <Menu
            icon={Puzzle}
            label={skillName || t('tasks.allSkills')}
            value={skillName}
            onChange={setSkillName}
            options={[
              { value: '', label: t('tasks.allSkills') },
              ...(tenant?.skills ?? []).filter((skill) => skill.kind === 'RELIABLE').map((skill) => ({ value: skill.name, label: skill.name })),
            ]}
          />
          <Menu
            label={t('tasks.stateLabel', { state: t(stateLabels[state]) })}
            value={state}
            onChange={setState}
            options={(Object.keys(stateLabels) as StateFilter[]).map((value) => ({ value, label: t(stateLabels[value]) }))}
          />
        </div>
        {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
        {!data && !error ? <Skeleton rows={8} /> : null}
        {data && rows.length === 0 ? <Empty>{t('tasks.empty')}</Empty> : null}
        {rows.length > 0 ? (
          <>
            <Th cols={cols}>
              <span>{t('col.state')}</span>
              <span>{t('col.task')}</span>
              <span>{t('col.trigger')}</span>
              <span>{t('col.skill')}</span>
              <span className="text-right">{t('col.cost')}</span>
              <span className="text-right">{t('col.updated')}</span>
            </Th>
            {rows.map(({ task, child }, index) => (
              <Row key={task.id} cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => open(index)}>
                <TaskState status={task.status} waitReason={task.waitReason} />
                <div className={clsx(child && "relative pl-5 before:absolute before:left-1 before:text-faint before:content-['└']")}>
                  <span className="font-medium">{task.title}</span>{' '}
                  <Id>
                    {task.issue ? issueLabel(task.issue.repository, task.issue.number) : t('tasks.chat')}
                    {task.blockedBy.length > 0
                      ? ` · ${t('tasks.blockedBy', { ids: task.blockedBy.map((blocker) => `#${blocker.issue?.number ?? ''}`).join(', ') })}`
                      : null}
                  </Id>
                </div>
                <span className="text-muted">{task.trigger.name}</span>
                <span className="text-muted">{skillProgress(task)}</span>
                <span className="text-right font-mono text-sm">{task.costUsd > 0 ? usd(task.costUsd) : '—'}</span>
                <span className="text-right text-muted">{age(task.updatedAt)}</span>
              </Row>
            ))}
          </>
        ) : null}
      </Pg>
    </Page>
  );
}
