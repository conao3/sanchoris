import { LogIn } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button } from '../components/Button.tsx';
import { Dim } from '../components/ui.tsx';
import { t } from '../i18n/index.ts';
import { exchangeCodeForTokens, loginPath, samePath, saveTokens } from '../lib/cognito.ts';
import { AuthLayout } from './CreateTenantPage.tsx';

export function AuthCallbackPage() {
  // Guard so the one-shot code exchange runs once even under StrictMode double-mount.
  const started = useRef(false);
  const [params] = useState(() => new URLSearchParams(window.location.search));
  const next = samePath(params.get('state'));
  const denied = params.get('error');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (started.current || denied) return;
    started.current = true;

    const code = params.get('code');
    if (!code) {
      window.location.replace(loginPath(next));
      return;
    }

    exchangeCodeForTokens(code)
      .then((tokens) => {
        saveTokens(tokens);
        window.location.replace(next ?? '/');
      })
      .catch((err: unknown) => {
        setError(err instanceof Error ? err.message : t('login.failedUnknown'));
        window.setTimeout(() => window.location.replace(loginPath(next)), 2000);
      });
  }, [denied, next, params]);

  if (denied) {
    return (
      <AuthLayout>
        <div className="mb-3 text-err">{t('login.denied', { error: params.get('error_description') ?? denied })}</div>
        <Button variant="primary" icon={LogIn} onPress={() => window.location.replace(loginPath(next))}>
          {t('login.again')}
        </Button>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout>
      <Dim>{error ? t('login.failed', { error }) : t('login.signingIn')}</Dim>
    </AuthLayout>
  );
}
