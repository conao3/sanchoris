import { LogIn } from 'lucide-react';
import { Button } from '../components/Button';
import { Dim, Sub, Title } from '../components/ui';
import { t } from '../i18n';
import { getLoginUrl } from '../lib/cognito';
import { AuthLayout } from './CreateTenantPage';

export function LoginPage() {
  return (
    <AuthLayout>
      <Title>
        <span className="text-auth">{t('login.title')}</span>
      </Title>
      <Sub className="mt-1.5">{t('login.intro')}</Sub>
      <Button variant="primary" icon={LogIn} onPress={() => window.location.assign(getLoginUrl())}>
        {t('login.google')}
      </Button>
      <Dim className="mt-2.5">{t('login.after')}</Dim>
    </AuthLayout>
  );
}
