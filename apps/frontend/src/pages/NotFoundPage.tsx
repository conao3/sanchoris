import { CircleHelp } from 'lucide-react';
import { Page, Pg } from '../components/Shell.tsx';
import { Empty, Lk } from '../components/ui.tsx';
import { t } from '../i18n/index.ts';

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
