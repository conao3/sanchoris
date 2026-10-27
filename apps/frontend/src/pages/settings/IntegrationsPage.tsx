import { useMutation } from '@apollo/client/react';
import { Ban, CircleDashed, ExternalLink, GitBranch, Link2, MessageSquare, Plus, ShieldCheck, TriangleAlert, UserCheck } from 'lucide-react';
import { Button } from '../../components/Button';
import { SettingsPage } from '../../components/SettingsTabs';
import { Chip, Dim, ErrorNote, Icon, Row, Skeleton, State, Sub, Th, Title } from '../../components/ui';
import {
  ConnectGitHubDocument,
  ConnectSlackDocument,
  IntegrationsDocument,
  UnlinkGitHubDocument,
} from '../../graphql/generated/graphql';
import { t, tx } from '../../i18n';
import { ago } from '../../lib/format';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

export function IntegrationsPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(IntegrationsDocument, { tenant: slug });
  const [connectGitHub] = useMutation(ConnectGitHubDocument);
  const [unlinkGitHub] = useMutation(UnlinkGitHubDocument);
  const [connectSlack] = useMutation(ConnectSlackDocument);
  const tenant = data?.tenant;
  const login = data?.viewer.user.githubLogin;
  const github = tenant?.integrations.find((integration) => integration.kind === 'GITHUB_INSTALLATION');
  const slack = tenant?.integrations.find((integration) => integration.kind === 'SLACK');
  const cols = '44px 1fr 130px 170px';

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
                <Button small icon={Ban} onPress={() => void unlinkGitHub()}>
                  {t('integrations.unlink')}
                </Button>
              ) : (
                <Button
                  variant="primary"
                  icon={Link2}
                  onPress={() =>
                    void connectGitHub().then((result) => {
                      const url = result.data?.connectGitHub.url;
                      if (url) {
                        window.location.assign(url);
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
                <Button variant="primary" icon={Plus} href={tenant.githubAppInstallUrl}>
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
                <Button
                  variant="primary"
                  icon={Plus}
                  onPress={() =>
                    void connectSlack({ variables: { tenantId: tenant.id } }).then((result) => {
                      const url = result.data?.connectSlack.url;
                      if (url) {
                        window.location.assign(url);
                      }
                    })
                  }
                >
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
