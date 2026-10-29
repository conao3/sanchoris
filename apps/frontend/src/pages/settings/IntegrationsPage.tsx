import { useNavigate, useSearch } from '@tanstack/react-router';
import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { Ban, CircleDashed, ExternalLink, GitBranch, Link2, MessageSquare, Plus, ShieldCheck, TriangleAlert, UserCheck } from 'lucide-react';
import { Button } from '../../components/Button';
import { SettingsPage } from '../../components/SettingsTabs';
import { ActionError, Chip, Dim, ErrorNote, Icon, Row, Skeleton, State, Sub, Th, Title, type Tone, toneClass } from '../../components/ui';
import {
  ConnectGitHubDocument,
  InstallGitHubAppDocument,
  IntegrationsDocument,
  UnlinkGitHubDocument,
} from '../../graphql/generated/graphql';
import { type MessageKey, t, tx } from '../../i18n';
import { ago } from '../../lib/format';
import { useAction } from '../../lib/mutation';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

type Outcome = { row: 'account' | 'installation' | 'either'; tone: Tone; text: MessageKey };

const outcomes: Record<string, Outcome> = {
  linked: { row: 'account', tone: 'ok', text: 'integrations.outcome.linked' },
  in_use: { row: 'account', tone: 'err', text: 'integrations.outcome.inUse' },
  installed: { row: 'installation', tone: 'ok', text: 'integrations.outcome.installed' },
  requested: { row: 'installation', tone: 'wait', text: 'integrations.outcome.requested' },
  not_member: { row: 'installation', tone: 'err', text: 'integrations.outcome.notMember' },
  not_admin: { row: 'installation', tone: 'err', text: 'integrations.outcome.notAdmin' },
  not_linked: { row: 'installation', tone: 'err', text: 'integrations.outcome.notLinked' },
  not_linked_account: { row: 'installation', tone: 'err', text: 'integrations.outcome.notLinkedAccount' },
  not_accessible: { row: 'installation', tone: 'err', text: 'integrations.outcome.notAccessible' },
  not_admin_of_account: { row: 'installation', tone: 'err', text: 'integrations.outcome.notAdminOfAccount' },
  installation_in_use: { row: 'installation', tone: 'err', text: 'integrations.outcome.installationInUse' },
  denied: { row: 'either', tone: 'err', text: 'integrations.outcome.denied' },
};

function OutcomeNote({ outcome }: { outcome: Outcome }) {
  return <div className={clsx('mt-0.5 text-xs', toneClass[outcome.tone])}>{t(outcome.text)}</div>;
}

export function IntegrationsPage() {
  const { slug } = useTenant();
  const navigate = useNavigate();
  const search = useSearch({ from: '/app/settings' });
  const [returned] = useState(search.github);
  const { data, error, refetch } = useData(IntegrationsDocument, { tenant: slug });

  useEffect(() => {
    if (search.github) {
      void navigate({ to: '/settings', search: {}, replace: true });
    }
  }, [search.github, navigate]);
  const [connectGitHub, connectState] = useAction(ConnectGitHubDocument);
  const [unlinkGitHub, unlinkState] = useAction(UnlinkGitHubDocument);
  const [installGitHubApp, installState] = useAction(InstallGitHubAppDocument);
  const tenant = data?.tenant;
  const login = data?.viewer.user.githubLogin;
  const github = tenant?.integrations.find((integration) => integration.kind === 'GITHUB_INSTALLATION');
  const slack = tenant?.integrations.find((integration) => integration.kind === 'SLACK');
  const cols = '44px 1fr 130px 170px';
  const known = returned ? outcomes[returned] : undefined;
  const outcome: Outcome | undefined = known?.row === 'either' ? { ...known, row: login ? 'installation' : 'account' } : known;

  return (
    <SettingsPage tab="integrations">
      <Title>{t('settings.integrations')}</Title>
      <Sub>{t('integrations.intro')}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={3} /> : null}
      {tenant ? (
        <>
          <Th cols={cols}>
            <span />
            <span>{t('col.integration')}</span>
            <span>{t('col.status')}</span>
            <span />
          </Th>
          <Row cols={cols} tall>
            <Icon icon={UserCheck} />
            <div>
              <span className="font-medium">{t('integrations.githubAccount')}</span>
              <Dim>{login ? tx('integrations.linkedAs', { login: <Chip>{login}</Chip> }) : t('integrations.notLinked')}</Dim>
              {outcome?.row === 'account' ? <OutcomeNote outcome={outcome} /> : null}
              <ActionError error={connectState.error ?? unlinkState.error} className="mt-0.5" />
            </div>
            {login ? (
              <State tone="ok" icon={ShieldCheck}>
                {t('integrations.linked')}
              </State>
            ) : (
              <State tone="wait" icon={CircleDashed}>
                {t('integrations.notLinkedState')}
              </State>
            )}
            <span className="justify-self-end">
              {login ? (
                <Button small icon={Ban} onPress={() => void unlinkGitHub({})} isDisabled={unlinkState.loading}>
                  {t('integrations.unlink')}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  icon={Link2}
                  isDisabled={connectState.loading}
                  onPress={() =>
                    void connectGitHub({}).then((result) => {
                      if (result) {
                        window.location.assign(result.connectGitHub.url);
                      }
                    })
                  }
                >
                  {t('integrations.linkGitHub')}
                </Button>
              )}
            </span>
          </Row>
          <Row cols={cols} tall>
            <Icon icon={GitBranch} />
            <div>
              <span className="font-medium">{t('integrations.githubInstallation')}</span>
              <Dim>
                {github && github.status !== 'NOT_CONNECTED'
                  ? tx('integrations.installedOn', {
                      account: <Chip>{github.account}</Chip>,
                      n: github.repositories.length,
                      repos: github.repositories.map((repository) => repository.split('/').pop()).join(', '),
                    })
                  : t('integrations.notInstalled')}
                {github?.error ? (
                  <span className="text-err">
                    {' '}
                    · {github.error}
                    {github.lastSyncedAt ? ` · ${t('integrations.lastSync', { at: ago(github.lastSyncedAt) })}` : null}
                  </span>
                ) : null}
              </Dim>
              {outcome?.row === 'installation' ? <OutcomeNote outcome={outcome} /> : null}
              <ActionError error={installState.error} className="mt-0.5" />
            </div>
            {github?.status === 'CONNECTED' ? (
              <State tone="ok" icon={ShieldCheck}>
                {t('integrations.connected')}
              </State>
            ) : github?.status === 'ERROR' ? (
              <State tone="err" icon={TriangleAlert}>
                {t('integrations.error')}
              </State>
            ) : (
              <State tone="wait" icon={CircleDashed}>
                {t('integrations.notConnected')}
              </State>
            )}
            <span className="justify-self-end">
              {github?.status === 'ERROR' ? (
                <Button small onPress={refetch}>
                  {t('common.retry')}
                </Button>
              ) : github?.manageUrl ? (
                <Button icon={ExternalLink} href={github.manageUrl}>
                  {t('integrations.manageOnGitHub')}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  icon={Plus}
                  isDisabled={installState.loading}
                  onPress={() =>
                    void installGitHubApp({ tenantId: tenant.id }).then((result) => {
                      if (result) {
                        window.location.assign(result.installGitHubApp.url);
                      }
                    })
                  }
                >
                  {t('integrations.installApp')}
                </Button>
              )}
            </span>
          </Row>
          <Row cols={cols} tall>
            <Icon icon={MessageSquare} />
            <div>
              <span className="font-medium">{t('integrations.slack')}</span>
              <Dim>{slack?.status === 'CONNECTED' ? tx('integrations.slackConnected', { workspace: <Chip>{slack.account}</Chip> }) : t('integrations.slackDetail')}</Dim>
              {slack?.status === 'CONNECTED' ? null : <Dim className="mt-0.5 text-xs">{t('integrations.slackLater')}</Dim>}
            </div>
            {slack?.status === 'CONNECTED' ? (
              <State tone="ok" icon={ShieldCheck}>
                {t('integrations.connected')}
              </State>
            ) : (
              <State tone="wait" icon={CircleDashed}>
                {t('integrations.notConnected')}
              </State>
            )}
            <span className="justify-self-end">
              {slack?.status === 'CONNECTED' ? (
                slack.manageUrl ? (
                  <Button icon={ExternalLink} href={slack.manageUrl}>
                    {t('integrations.manageInSlack')}
                  </Button>
                ) : null
              ) : (
                <Button icon={Plus} isDisabled>
                  {t('integrations.addSlack')}
                </Button>
              )}
            </span>
          </Row>
        </>
      ) : null}
    </SettingsPage>
  );
}
