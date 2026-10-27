import { useMutation } from '@apollo/client/react';
import type { LucideIcon } from 'lucide-react';
import { Bot, ExternalLink, ShieldCheck, UserCheck, Users, Zap } from 'lucide-react';
import { useMemo, useState } from 'react';
import { Button } from '../../components/Button';
import { Menu } from '../../components/Menu';
import { SettingsPage } from '../../components/SettingsTabs';
import { Dim, Empty, ErrorNote, Icon, Lk, Row, Skeleton, Sub, Th, Title } from '../../components/ui';
import { type ActorKind, type AuditFilter, AuditLogDocument, type AuditOrigin, ExportAuditLogDocument } from '../../graphql/generated/graphql';
import { type MessageKey, t } from '../../i18n';
import { dayTime } from '../../lib/format';
import { useListNav } from '../../lib/keys';
import { useData } from '../../lib/query';
import { useTenant } from '../../lib/tenant';

const actorIcons: Record<ActorKind, LucideIcon> = {
  MEMBER: UserCheck,
  AGENT: Bot,
  TRIGGER: Zap,
  SYSTEM: ShieldCheck,
  OPERATOR: ShieldCheck,
};

const originLabels: Record<AuditOrigin, MessageKey | null> = {
  WEB: null,
  GITHUB: 'audit.github',
  SLACK: 'audit.slack',
  SYSTEM: null,
};

const periods: { value: string; label: MessageKey; days: number }[] = [
  { value: '7', label: 'audit.last7', days: 7 },
  { value: '30', label: 'audit.last30', days: 30 },
  { value: '90', label: 'audit.last90', days: 90 },
];

export function AuditPage() {
  const { slug } = useTenant();
  const [actor, setActor] = useState('');
  const [action, setAction] = useState('');
  const [period, setPeriod] = useState('7');
  const [filterOpen, setFilterOpen] = useState(false);
  const since = useMemo(() => new Date(Date.now() - Number(period) * 86_400_000).toISOString(), [period]);
  const filter: AuditFilter = {
    since,
    ...(actor ? { memberId: actor } : {}),
    ...(action ? { action } : {}),
  };
  const { data, error, refetch } = useData(AuditLogDocument, { tenant: slug, filter });
  const [exportLog, exportState] = useMutation(ExportAuditLogDocument);
  const tenant = data?.tenant;
  const events = tenant?.auditEvents ?? [];
  const actions = [...new Set(events.map((event) => event.action))].sort();

  function doExport() {
    if (tenant) {
      void exportLog({ variables: { tenantId: tenant.id, filter } }).then((result) => {
        const url = result.data?.exportAuditLog.url;
        if (url) {
          window.open(url, '_blank', 'noreferrer');
        }
      });
    }
  }

  const nav = useListNav(events.length, () => undefined, {
    '/': () => setFilterOpen(true),
    e: doExport,
  });
  const cols = '130px 160px 190px 1fr 120px';
  const actorName = tenant?.members.find((member) => member.id === actor)?.user.name;

  return (
    <SettingsPage
      tab="audit"
      actions={
        <Button icon={ExternalLink} onPress={doExport} isDisabled={exportState.loading || !tenant}>
          {t('audit.export')}
        </Button>
      }
      keys={[
        ['j k', 'keys.move'],
        ['/', 'keys.filter'],
        ['e', 'audit.exportKey'],
      ]}
    >
      <Title>{t('settings.audit')}</Title>
      <Sub>{t('audit.intro', { tenant: tenant?.name ?? '' })}</Sub>
      <div className="mb-3 flex gap-2">
        <Menu
          icon={Users}
          label={actorName ?? t('audit.allActors')}
          value={actor}
          open={filterOpen}
          onOpenChange={setFilterOpen}
          onChange={setActor}
          options={[{ value: '', label: t('audit.allActors') }, ...(tenant?.members ?? []).map((member) => ({ value: member.id, label: member.user.name }))]}
        />
        <Menu
          label={action || t('audit.allActions')}
          value={action}
          onChange={setAction}
          options={[{ value: '', label: t('audit.allActions') }, ...actions.map((item) => ({ value: item, label: <span className="font-mono text-sm">{item}</span> }))]}
        />
        <Menu
          label={t(periods.find((item) => item.value === period)?.label ?? 'audit.last7')}
          value={period}
          onChange={setPeriod}
          options={periods.map((item) => ({ value: item.value, label: t(item.label) }))}
        />
      </div>
      {error ? <ErrorNote message={error.message} onRetry={refetch} /> : null}
      {!data && !error ? <Skeleton rows={8} /> : null}
      {tenant && events.length === 0 ? <Empty>{t('audit.empty')}</Empty> : null}
      {events.length > 0 ? (
        <>
          <Th cols={cols}>
            <span>{t('col.time')}</span>
            <span>{t('col.actor')}</span>
            <span>{t('col.action')}</span>
            <span>{t('col.targetSummary')}</span>
            <span>{t('col.from')}</span>
          </Th>
          {events.map((event, index) => {
            const origin = originLabels[event.origin];
            return (
              <Row key={event.id} cols={cols} selected={nav.selected === index} rowRef={nav.rowRef(index)} onClick={() => nav.select(index)}>
                <span className="font-mono text-sm">{dayTime(event.at)}</span>
                <span>
                  <Icon icon={actorIcons[event.actorKind]} /> {event.actorName}
                </span>
                <span className="font-mono text-sm">{event.action}</span>
                <span>
                  {event.targetTask ? <Lk to="/tasks/$task" params={{ task: event.targetTask.id }}>{event.target}</Lk> : event.target}
                  {event.summary ? <span className="text-muted"> · {event.summary}</span> : null}
                </span>
                {event.ip ? (
                  <span className="font-mono text-sm text-muted">{event.ip}</span>
                ) : (
                  <Dim>{origin ? t(origin) : '—'}</Dim>
                )}
              </Row>
            );
          })}
        </>
      ) : null}
    </SettingsPage>
  );
}
