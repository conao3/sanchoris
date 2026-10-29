import { CircleHelp } from 'lucide-react';
import { Page, Pg } from '../components/Shell';
import { Empty, Lk } from '../components/ui';
import { t } from '../i18n';

export function NotFoundPage() {
  return (
    <Page crumbs={[{ label: t('notFound.title') }]} keys={[['g h', 'nav.home']]}>
      <Pg>
        <Empty icon={CircleHelp}>
          {t('notFound.text')} <Lk to="/">{t('notFound.home')}</Lk>
        </Empty>
      </Pg>
    </Page>
  );
}
