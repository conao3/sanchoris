import { useMutation } from '@apollo/client/react';
import { CircleCheck, CircleX, ExternalLink, FolderGit2, MessageSquare, Plus, TriangleAlert, UserCheck, Zap } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { Button } from '../../components/Button';
import { SettingsPage } from '../../components/SettingsTabs';
import { Chip, Dim, ErrorNote, ExternalLk, Icon, Id, Row, Section, Skeleton, State, Sub, Th, Title } from '../../components/ui';
import { AddConfigDocument, ConfigsDocument, SyncConfigDocument } from '../../graphql/generated/graphql';
import { t } from '../../i18n';
import { ago } from '../../lib/format';
import { useListNav } from '../../lib/keys';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

type ValidationError = { path: string; line?: number | null; column?: number | null; message: string };

const fieldClass = 'h-[30px] rounded-ui border border-line2 bg-bg px-2.5 font-mono text-sm outline-none focus:border-ink';

function Errors({ errors }: { errors: ValidationError[] }) {
  return (
    <div className="mb-2 ml-1 mt-1 border-l-2 border-err pl-3">
      {errors.map((error, index) => (
        <div key={index} className="text-sm">
          <span className="font-mono text-err">
            {error.path}
            {error.line != null ? `:${error.line}` : ''}
            {error.column != null ? `:${error.column}` : ''}
          </span>{' '}
          {error.message}
        </div>
      ))}
    </div>
  );
}

function AddConfigForm({ tenantId, repositories }: { tenantId: string; repositories: string[] }) {
  const [repository, setRepository] = useState(repositories[0] ?? '');
  const [root, setRoot] = useState('/');
  const [branch, setBranch] = useState('main');
  const [addConfig, { data, loading, error }] = useMutation(AddConfigDocument, { refetchQueries: [ConfigsDocument] });

  function submit(event: FormEvent) {
    event.preventDefault();
    void addConfig({ variables: { input: { tenantId, repository, root, branch } } });
  }

  return (
    <>
      <form onSubmit={submit} className="mt-3 flex items-center gap-2">
        <span className="text-muted">{t('config.addConfig')}</span>
        <select aria-label={t('col.repository')} className={`${fieldClass} w-60`} value={repository} onChange={(event) => setRepository(event.target.value)}>
          {repositories.map((item) => (
            <option key={item}>{item}</option>
          ))}
        </select>
        <input aria-label={t('col.root')} className={`${fieldClass} w-[150px]`} value={root} onChange={(event) => setRoot(event.target.value)} />
        <input aria-label={t('config.branch')} className={`${fieldClass} w-[90px]`} value={branch} onChange={(event) => setBranch(event.target.value)} />
        <Button type="submit" icon={Plus} isDisabled={loading || !repository}>
          {t('config.addConfigButton')}
        </Button>
        <span className="text-muted">{t('config.checksFirst')}</span>
      </form>
      {error ? <Dim className="mt-2 text-err">{error.message}</Dim> : null}
      {data && data.addConfig.errors.length > 0 ? <Errors errors={data.addConfig.errors} /> : null}
    </>
  );
}

export function ConfigPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(ConfigsDocument, { tenant: slug });
  const [syncConfig, syncState] = useMutation(SyncConfigDocument);
  const tenant = data?.tenant;
  const configs = tenant?.configs ?? [];
  const nav = useListNav(configs.length, (index) => window.open(configs[index].url, '_blank', 'noreferrer'));
  const configCols = '1fr 150px 80px 90px 90px 80px 120px 110px';
  const projectCols = '120px 1fr 120px 280px';
  const changeCols = '44px 1fr 150px 110px 110px 150px';
  const repositories = tenant?.integrations.flatMap((integration) => (integration.kind === 'GITHUB_INSTALLATION' ? integration.repositories : [])) ?? [];

  return (
    <SettingsPage tab="config">
      <Title>{t('settings.config')}</Title>
      <Sub>{t('config.intro')}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={2} /> : null}
      {tenant ? (
        <>
          <Th cols={configCols}>
            <span>{t('col.repository')}</span>
            <span>{t('col.root')}</span>
            <span>{t('col.revision')}</span>
            <span>{t('col.synced')}</span>
            <span className="text-right">{t('nav.triggers')}</span>
            <span className="text-right">{t('nav.skills')}</span>
            <span>{t('col.changeRequests')}</span>
            <span>{t('col.validation')}</span>
          </Th>
          {configs.map((config, index) => {
            const { validation } = config.revision;
            return (
              <div key={config.id}>
                <Row cols={configCols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                  <span className="font-medium">
                    <Icon icon={FolderGit2} /> <ExternalLk href={config.url} className="no-underline">{config.repository}</ExternalLk>
                  </span>
                  <span className="font-mono text-sm">{config.root}</span>
                  <span className="font-mono text-sm">r{config.revision.number}</span>
                  <span className="text-muted">{config.syncedAt ? ago(config.syncedAt) : '—'}</span>
                  <span className="text-right font-mono text-sm">{config.triggerCount}</span>
                  <span className="text-right font-mono text-sm">{config.skillCount}</span>
                  {config.changeRequests.length > 0 ? (
                    <span className="underline decoration-line2 underline-offset-[3px]">{t('config.openCount', { n: config.changeRequests.length })}</span>
                  ) : (
                    <span className="text-muted">—</span>
                  )}
                  {validation.valid ? (
                    <State tone="ok" icon={CircleCheck}>
                      {t('config.valid')}
                    </State>
                  ) : (
                    <State tone="err" icon={CircleX}>
                      {t('config.invalid', { n: validation.errors.length })}
                    </State>
                  )}
                </Row>
                {config.syncError ? (
                  <div className="flex items-center gap-2 py-1.5 pl-1 text-sm">
                    <Icon icon={TriangleAlert} className="text-err" />
                    <span className="text-err">{config.syncError}</span>
                    <span className="text-muted">{config.syncedAt ? t('config.lastSynced', { at: ago(config.syncedAt) }) : null}</span>
                    <Button small isDisabled={syncState.loading} onPress={() => void syncConfig({ variables: { configId: config.id } })}>
                      {t('common.retry')}
                    </Button>
                  </div>
                ) : null}
                {validation.valid ? null : <Errors errors={validation.errors} />}
              </div>
            );
          })}
          <AddConfigForm tenantId={tenant.id} repositories={repositories} />

          <Section count={t('config.projectsCount', { n: tenant.projects.length })}>{t('config.projects')}</Section>
          <Dim className="mb-1.5">{t('config.projectsDetail')}</Dim>
          {tenant.projects.length > 0 ? (
            <>
              <Th cols={projectCols}>
                <span>{t('col.project')}</span>
                <span>{t('col.repositories')}</span>
                <span>{t('col.warmBuild')}</span>
                <span>{t('col.usedBySessions')}</span>
              </Th>
              {tenant.projects.map((project) => (
                <Row key={project.id} cols={projectCols}>
                  <span className="font-medium">{project.name}</span>
                  <span>
                    {project.repositories.map((repository) => (
                      <Chip key={repository.id}>{repository.name}</Chip>
                    ))}
                  </span>
                  {project.warmBuild ? (
                    <State tone={project.warmBuild.succeeded ? 'ok' : 'err'} icon={Zap}>
                      {ago(project.warmBuild.finishedAt)}
                    </State>
                  ) : (
                    <span className="text-muted">—</span>
                  )}
                  {project.sessions.length > 0 ? <span>{project.sessions.map((session) => session.name).join(', ')}</span> : <span className="text-muted">—</span>}
                </Row>
              ))}
            </>
          ) : null}

          <Section count={t('config.changeRequestsCount', { n: tenant.changeRequests.length })}>{t('col.changeRequests')}</Section>
          {tenant.changeRequests.length === 0 ? <Dim>{t('config.noChangeRequests')}</Dim> : null}
          {tenant.changeRequests.length > 0 ? (
            <>
              <Th cols={changeCols}>
                <span>#</span>
                <span>{t('col.title')}</span>
                <span>{t('col.changes')}</span>
                <span>{t('col.openedBy')}</span>
                <span>{t('col.validation')}</span>
                <span />
              </Th>
              {tenant.changeRequests.map((change) => (
                <Row key={change.id} cols={changeCols}>
                  <Id>#{change.number}</Id>
                  <span className="font-medium">{change.title}</span>
                  <span className="underline decoration-line2 underline-offset-[3px]">{change.changes.map((item) => item.name).filter((name, index, all) => all.indexOf(name) === index).join(', ')}</span>
                  <span className="text-muted">
                    {change.openedFromTask ? (
                      <>
                        <Icon icon={MessageSquare} /> {t('config.fromChat')}
                      </>
                    ) : (
                      <>
                        <Icon icon={UserCheck} /> {change.openedBy?.user.name}
                      </>
                    )}
                  </span>
                  {change.validation.valid ? (
                    <State tone="ok" icon={CircleCheck}>
                      {t('config.valid')}
                    </State>
                  ) : (
                    <State tone="err" icon={CircleX}>
                      {t('config.invalidShort')}
                    </State>
                  )}
                  <span className="justify-self-end">
                    <Button small icon={ExternalLink} href={change.url}>
                      {t('common.reviewOnGitHub')}
                    </Button>
                  </span>
                </Row>
              ))}
            </>
          ) : null}
        </>
      ) : null}
    </SettingsPage>
  );
}
