import { useEffect, useRef, useState } from 'react';
import { Wordmark } from '../components/Wordmark';
import { exchangeCodeForTokens, saveTokens } from '../lib/cognito';

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
        setError(err instanceof Error ? err.message : 'sign-in failed');
        window.setTimeout(() => window.location.replace('/login'), 2000);
      });
  }, []);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-6 bg-surface-soft px-6 text-center">
      <Wordmark size="md" />
      {error ? (
        <p className="text-md text-body">Sign-in failed: {error}. Redirecting…</p>
      ) : (
        <p className="text-md text-body">Signing you in…</p>
      )}
    </main>
  );
}
