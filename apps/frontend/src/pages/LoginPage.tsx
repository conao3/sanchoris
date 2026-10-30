import { useSearch } from '@tanstack/react-router';
import { LogIn } from 'lucide-react';
import { Button } from '../components/Button.tsx';
import { Dim, Sub, Title } from '../components/ui.tsx';
import { t } from '../i18n/index.ts';
import { getLoginUrl } from '../lib/cognito.ts';
import { AuthLayout } from './CreateTenantPage.tsx';

export function LoginPage() {
  const { next } = useSearch({ from: '/login' });
  return (
    <AuthLayout>
      <Title>
        <span className="text-auth">{t('login.title')}</span>
      </Title>
      <Sub className="mt-1.5">{t('login.intro')}</Sub>
      <Button variant="primary" icon={LogIn} onPress={() => window.location.assign(getLoginUrl(next))}>
        {t('login.google')}
      </Button>
      <Dim className="mt-2.5">{t('login.after')}</Dim>
    </AuthLayout>
  );
}
