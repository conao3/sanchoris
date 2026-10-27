import { useEffect, useRef, useState } from 'react';
import { Dim } from '../components/ui';
import { t } from '../i18n';
import { exchangeCodeForTokens, saveTokens } from '../lib/cognito';
import { AuthLayout } from './CreateTenantPage';

export function AuthCallbackPage() {
  // Guard so the one-shot code exchange runs once even under StrictMode double-mount.
  const started = useRef(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (started.current) return;
    started.current = true;

    const code = new URLSearchParams(window.location.search).get('code');
    if (!code) {
      window.location.replace('/login');
      return;
    }

    exchangeCodeForTokens(code)
      .then((tokens) => {
        saveTokens(tokens);
        window.location.replace('/');
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : t('login.failedUnknown'));
        window.setTimeout(() => window.location.replace('/login'), 2000);
      });
  }, []);

  return (
    <AuthLayout>
      <Dim>{error ? t('login.failed', { error }) : t('login.signingIn')}</Dim>
    </AuthLayout>
  );
}
