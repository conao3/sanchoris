import { Check } from 'lucide-react';
import { type FormEvent, useState } from 'react';
import { type ApiKeyProvider, AddCredentialDocument } from '../graphql/generated/graphql.ts';
import { t } from '../i18n/index.ts';
import { useAction } from '../lib/mutation.ts';
import { Button } from './Button.tsx';
import { ActionError, Code } from './ui.tsx';

export function SubscriptionCommands({ tenantName }: { tenantName: string }) {
  return (
    <Code>
      <span className="text-faint"># {t('setup.claudeSubscription')}</span>
      {'\n$ sanchoris claude-auth\n'}
      <span className="text-faint"># {t('setup.codexSubscription')}</span>
      {'\n$ sanchoris codex-auth\n'}
      <span className="text-faint"># {t('setup.shareWithTenant', { tenant: tenantName })}</span>
      {'\n$ sanchoris claude-auth --org'}
    </Code>
  );
}

const fieldClass = 'h-[30px] w-full rounded-ui border border-line2 bg-bg px-2.5 outline-none focus:border-ink';

export function ApiKeyForm({ tenantId }: { tenantId: string }) {
  const [provider, setProvider] = useState<ApiKeyProvider>('ANTHROPIC');
  const [name, setName] = useState('');
  const [key, setKey] = useState('');
  const [addCredential, { loading, error }] = useAction(AddCredentialDocument, { refetchQueries: 'active' });

  function submit(event: FormEvent) {
    event.preventDefault();
    void addCredential({ input: { tenantId, name, provider, key, scope: 'ORGANIZATION' } }).then((done) => {
      if (!done) {
        return;
      }
      setName('');
      setKey('');
    });
  }

  return (
    <form onSubmit={submit}>
      <label className="mb-1 mt-3 block text-xs text-muted" htmlFor="provider">
        {t('setup.provider')}
      </label>
      <select id="provider" className={fieldClass} value={provider} onChange={(event) => setProvider(event.target.value as ApiKeyProvider)}>
        <option value="ANTHROPIC">{t('setup.anthropicKey')}</option>
        <option value="OPENAI">{t('setup.openaiKey')}</option>
        <option value="BEDROCK">{t('setup.bedrock')}</option>
      </select>
      <label className="mb-1 mt-3 block text-xs text-muted" htmlFor="credential-name">
        {t('setup.name')}
      </label>
      <input
        id="credential-name"
        className={`${fieldClass} font-mono text-sm`}
        placeholder="acme-claude-key"
        value={name}
        onChange={(event) => setName(event.target.value)}
        required
      />
      <label className="mb-1 mt-3 block text-xs text-muted" htmlFor="credential-key">
        {t('setup.key')}
      </label>
      <input
        id="credential-key"
        type="password"
        className={`${fieldClass} font-mono text-sm`}
        placeholder="sk-ant-••••••••••••••••"
        value={key}
        onChange={(event) => setKey(event.target.value)}
        required
      />
      <ActionError error={error} className="mt-2" />
      <div className="mt-3.5">
        <Button type="submit" variant="primary" icon={Check} isDisabled={loading}>
          {t('setup.saveKey')}
        </Button>
      </div>
    </form>
  );
}
