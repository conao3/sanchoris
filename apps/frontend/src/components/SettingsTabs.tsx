import { Link } from '@tanstack/react-router';
import clsx from 'clsx';
import type { LucideIcon } from 'lucide-react';
import { CreditCard, FolderGit2, GitBranch, KeyRound, ListChecks, Users } from 'lucide-react';
import type { ReactNode } from 'react';
import { type MessageKey, t } from '../i18n';
import { Page, Pg } from './Shell';
import { Icon } from './ui';

export type SettingsTab = 'integrations' | 'config' | 'credentials' | 'members' | 'audit' | 'billing';

const tabs: { id: SettingsTab; to: string; label: MessageKey; icon: LucideIcon }[] = [
  { id: 'integrations', to: '/settings', label: 'settings.integrations', icon: GitBranch },
  { id: 'config', to: '/settings/config', label: 'settings.config', icon: FolderGit2 },
  { id: 'credentials', to: '/settings/credentials', label: 'settings.credentials', icon: KeyRound },
  { id: 'members', to: '/settings/members', label: 'settings.members', icon: Users },
  { id: 'audit', to: '/settings/audit', label: 'settings.audit', icon: ListChecks },
  { id: 'billing', to: '/settings/billing', label: 'settings.billing', icon: CreditCard },
];

export function SettingsPage({
  tab,
  actions,
  keys = [['g s', 'nav.settings']],
  children,
}: {
  tab: SettingsTab;
  actions?: ReactNode;
  keys?: [string, MessageKey][];
  children: ReactNode;
}) {
  const current = tabs.find((item) => item.id === tab) ?? tabs[0];
  return (
    <Page crumbs={[{ label: t('nav.settings'), to: '/settings' }, { label: t(current.label) }]} actions={actions} keys={keys}>
      <Pg>
        <div className="mb-4 flex h-[34px] items-end gap-[18px] border-b border-line">
          {tabs.map((item) => (
            <Link
              key={item.id}
              to={item.to}
              className={clsx(
                'flex items-center gap-1.5 pb-2',
                item.id === tab ? '-mb-px border-b-2 border-ink font-medium text-ink' : 'text-muted hover:text-ink',
              )}
            >
              <Icon icon={item.icon} />
              {t(item.label)}
            </Link>
          ))}
        </div>
        {children}
      </Pg>
    </Page>
  );
}
