import { Ban, Bot, LogOut, Plus, RefreshCw, ShieldCheck, Terminal, TriangleAlert } from 'lucide-react';
import { useState } from 'react';
import { ApiKeyForm, SubscriptionCommands } from '../../components/ApiKeyForm.tsx';
import { Button } from '../../components/Button.tsx';
import { SettingsPage } from '../../components/SettingsTabs.tsx';
import { ActionError, Dim, Empty, ErrorNote, Icon, Meter, Row, Section, Skeleton, State, Sub, Table, Th, Title } from '../../components/ui.tsx';
import { type CredentialKind, CredentialsDocument, RemoveCredentialDocument } from '../../graphql/generated/graphql.ts';
import { type MessageKey, t, tx } from '../../i18n/index.ts';
import { ago } from '../../lib/format.ts';
import { useListNav } from '../../lib/keys.ts';
import { useAction } from '../../lib/mutation.ts';
import { useData } from '../../lib/query.ts';
import { useTenant } from '../../lib/tenant.tsx';

const kindLabels: Record<CredentialKind, MessageKey> = {
  SUBSCRIPTION: 'credentials.subscription',
  API_KEY: 'credentials.apiKey',
  BEDROCK: 'credentials.bedrock',
};

export function CredentialsPage() {
  const { slug } = useTenant();
  const { data, error, refetch } = useData(CredentialsDocument, { tenant: slug });
  const [remove, removeState] = useAction(RemoveCredentialDocument, { refetchQueries: [CredentialsDocument] });
  const [connect, setConnect] = useState(false);
  const tenant = data?.tenant;
  const credentials = tenant?.credentials ?? [];
  const nav = useListNav(credentials.length, () => undefined);
  const cols = '130px 80px 140px 140px 96px 64px 110px 90px minmax(90px,1fr) 110px';

  return (
    <SettingsPage
      tab="credentials"
      keys={[
        ['g s', 'nav.settings'],
        ['j k', 'keys.move'],
      ]}
      actions={
        <Button variant="primary" icon={Plus} onPress={() => setConnect(!connect)}>
          {t('credentials.connect')}
        </Button>
      }
    >
      <Title>{t('settings.credentials')}</Title>
      <Sub>{t('credentials.intro')}</Sub>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={4} /> : null}
      {tenant && connect ? (
        <div className="mb-4 grid grid-cols-1 gap-10 border-b border-line pb-4 lg:grid-cols-[1fr_340px]">
          <div>
            <Section first>{t('setup.connectSubscription')}</Section>
            <Dim className="mb-2">{t('setup.connectSubscriptionDetail')}</Dim>
            <SubscriptionCommands tenantName={tenant.name} />
          </div>
          <div>
            <Section first>{t('setup.orApiKey')}</Section>
            <Dim>{t('setup.apiKeyProviders')}</Dim>
            <ApiKeyForm tenantId={tenant.id} />
          </div>
        </div>
      ) : null}
      {tenant && credentials.length === 0 ? <Empty>{t('credentials.empty')}</Empty> : null}
      {credentials.length > 0 ? (
        <>
          <Table>
            <Th cols={cols}>
              <span>{t('col.name')}</span>
              <span>{t('col.provider')}</span>
              <span>{t('col.account')}</span>
              <span>{t('col.kind')}</span>
              <span>{t('col.scope')}</span>
              <span>{t('col.owner')}</span>
              <span>{t('col.status')}</span>
              <span>{t('col.usage')}</span>
              <span>{t('col.lastUsed')}</span>
              <span />
            </Th>
            {credentials.map((credential, index) => {
              const reconnect = credential.status === 'RECONNECT';
              const week = credential.usage?.sevenDay;
              const fiveHour = credential.usage?.fiveHour;
              return (
                <Row key={credential.id} cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                  <span className="font-mono text-sm">{credential.name}</span>
                  <span className="font-medium">
                    <Icon icon={credential.provider === 'CODEX' ? Terminal : Bot} /> {t(credential.provider === 'CODEX' ? 'provider.codex' : 'provider.claude')}
                  </span>
                  <span className={credential.kind === 'SUBSCRIPTION' ? '' : 'font-mono text-sm'}>{credential.account}</span>
                  <span>
                    {t(kindLabels[credential.kind])}
                    {credential.subscriptionPlan ? ` · ${credential.subscriptionPlan}` : ''}
                  </span>
                  <span>{t(credential.scope === 'PERSONAL' ? 'credentials.personal' : 'credentials.organization')}</span>
                  <span>{credential.owner?.user.name ?? tenant?.name}</span>
                  {reconnect ? (
                    <State tone="err" icon={TriangleAlert}>
                      {t('status.reconnect')}
                    </State>
                  ) : (
                    <State tone="ok" icon={ShieldCheck}>
                      {t('integrations.connected')}
                    </State>
                  )}
                  {week ? (
                    <div title={t('credentials.usageTitle', { week: Math.round(week.utilization * 100), fiveHour: Math.round((fiveHour?.utilization ?? 0) * 100) })}>
                      <Meter value={week.utilization} />
                    </div>
                  ) : (
                    <span className="text-muted">—</span>
                  )}
                  <span className="text-muted">
                    {reconnect && credential.rejectedAt
                      ? t('credentials.rejected', { at: ago(credential.rejectedAt) })
                      : credential.lastUsedAt
                        ? ago(credential.lastUsedAt)
                        : t('credentials.never')}
                  </span>
                  {reconnect ? (
                    <Button variant="primary" small icon={RefreshCw} onPress={() => setConnect(true)}>
                      {t('credentials.reconnect')}
                    </Button>
                  ) : (
                    <Button small icon={credential.kind === 'SUBSCRIPTION' ? LogOut : Ban} onPress={() => void remove({ credentialId: credential.id })} isDisabled={removeState.loading}>
                      {credential.kind === 'SUBSCRIPTION' ? t('credentials.disconnect') : t('credentials.remove')}
                    </Button>
                  )}
                </Row>
              );
            })}
          </Table>
          <ActionError error={removeState.error} className="mt-1.5" />
        </>
      ) : null}
      <Section>{t('credentials.howPicked')}</Section>
      <Dim className="leading-[1.8]">
        {tx('credentials.howPickedDetail', {
          example: <span className="font-mono text-sm">{credentials.find((credential) => credential.scope === 'ORGANIZATION')?.name ?? 'acme-claude-team'}</span>,
          requester: <span className="font-mono text-sm">requester</span>,
        })}
      </Dim>
    </SettingsPage>
  );
}
