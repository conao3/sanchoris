import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail, type RailItemDef } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Kbd, Tag } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Rail items ────────────────────────────────────────────────────────────────

const channelRailItems: RailItemDef[] = WORKSPACE_ITEMS.map((i) => ({
  ...i,
  active: i.key === 'channels',
}));

// ─── Local primitives ─────────────────────────────────────────────────────────

function Chip({
  active,
  count,
  dot,
  className,
  children,
}: {
  active?: boolean;
  count?: number;
  dot?: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <span
      className={`inline-flex h-7 cursor-pointer items-center gap-[6px] rounded-[6px] border px-[10px] text-[12px] ${
        active
          ? 'border-hairline bg-surface-cream-strong font-medium text-ink'
          : 'border-hairline bg-canvas text-body'
      } ${className ?? ''}`}
    >
      {dot && (
        <span className="h-[6px] w-[6px] rounded-full bg-accent-teal" />
      )}
      {children}
      {count != null && (
        <span className="text-[11px] text-muted-soft">{count}</span>
      )}
    </span>
  );
}

type DotColor = 'green' | 'coral' | 'muted';

function StatusDot({ color }: { color: DotColor }) {
  const cls: Record<DotColor, string> = {
    green: 'bg-success',
    coral: 'bg-primary',
    muted: 'bg-muted-soft',
  };
  return (
    <span className={`inline-block h-2 w-2 flex-shrink-0 rounded-full ${cls[color]}`} />
  );
}

function AuthStatus({ dot, children }: { dot: DotColor; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-[6px] text-[12px] text-body">
      <StatusDot color={dot} />
      {children}
    </span>
  );
}

function ChannelIcon({
  label,
  bg,
  color,
}: {
  label: string;
  bg: string;
  color?: string;
}) {
  return (
    <span
      className="inline-flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-[6px] font-mono text-[12px] font-bold leading-none"
      style={{ background: bg, color: color ?? 'inherit' }}
    >
      {label}
    </span>
  );
}

// ─── Channel data ─────────────────────────────────────────────────────────────

type ChannelRow = {
  icon: { label: string; bg: string; color?: string };
  name: string;
  subtitle: ReactNode;
  status: ReactNode;
  defaultProject: ReactNode;
  routingRules: ReactNode;
  lastEvent: string;
  action: { variant: 'primary' | 'secondary'; label: string };
};

const CHANNELS: ChannelRow[] = [
  {
    icon: { label: 'W', bg: 'var(--color-surface-card)' },
    name: 'Web UI',
    subtitle: 'app.sanchoris.dev',
    status: <AuthStatus dot="green">SAML · acme-sso</AuthStatus>,
    defaultProject: <span className="font-mono text-muted">—</span>,
    routingRules: (
      <span className="text-[12px] text-muted">No rules · interactive sessions</span>
    ),
    lastEvent: '12s ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: 'S', bg: 'rgba(93,184,166,0.22)', color: '#2c6e62' },
    name: 'Slack',
    subtitle: '#sanchoris-ops · acme.slack.com',
    status: <AuthStatus dot="green">OAuth · bot</AuthStatus>,
    defaultProject: <Tag tone="teal">acme-platform</Tag>,
    routingRules: (
      <span className="text-[12px]">
        6 rules · <span className="font-mono text-muted">@san +urgent → queue.p0</span>
      </span>
    ),
    lastEvent: '2m ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: 'T', bg: 'rgba(93,184,166,0.22)', color: '#2c6e62' },
    name: 'Telegram',
    subtitle: '@sanchoris_bot',
    status: <AuthStatus dot="green">Bot token</AuthStatus>,
    defaultProject: <Tag tone="teal">acme-platform</Tag>,
    routingRules: (
      <span className="text-[12px]">
        2 rules · <span className="font-mono text-muted">/task → queue.normal</span>
      </span>
    ),
    lastEvent: '8m ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: 'D', bg: 'rgba(232,165,90,0.25)', color: '#7a4d10' },
    name: 'Discord',
    subtitle: 'acme-community guild',
    status: (
      <span className="inline-flex flex-wrap items-center gap-[6px]">
        <Tag tone="coral">needs auth</Tag>
        <span className="text-[12px] text-muted">token expired 04/22</span>
      </span>
    ),
    defaultProject: <Tag tone="teal">growth-api</Tag>,
    routingRules: (
      <span className="text-[12px]">
        1 rule · <span className="font-mono text-muted">!bug → queue.normal</span>
      </span>
    ),
    lastEvent: '2d ago',
    action: { variant: 'primary', label: 'Reconnect' },
  },
  {
    icon: { label: '@', bg: 'var(--color-surface-card)', color: 'var(--color-ink)' },
    name: 'Email',
    subtitle: 'triage@sanchoris.dev',
    status: <AuthStatus dot="green">MX verified</AuthStatus>,
    defaultProject: <Tag tone="teal">queue-global</Tag>,
    routingRules: (
      <span className="text-[12px]">
        3 rules · <span className="font-mono text-muted">from:@acme.io → acme-platform</span>
      </span>
    ),
    lastEvent: '14m ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: 'G', bg: 'var(--color-surface-dark)', color: 'var(--color-on-dark)' },
    name: 'GitHub events',
    subtitle: 'acme-org · 14 repos · webhook v3',
    status: <AuthStatus dot="green">GitHub App</AuthStatus>,
    defaultProject: <Tag tone="teal">acme-platform</Tag>,
    routingRules: (
      <span className="text-[12px]">
        12 rules · <span className="font-mono text-muted">pr.opened → review.workflow</span>
      </span>
    ),
    lastEvent: 'just now',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: 'L', bg: 'rgba(93,184,166,0.22)', color: '#2c6e62' },
    name: 'Linear events',
    subtitle: 'acme · team CON',
    status: <AuthStatus dot="green">API key</AuthStatus>,
    defaultProject: <Tag tone="teal">acme-platform</Tag>,
    routingRules: (
      <span className="text-[12px]">
        5 rules · <span className="font-mono text-muted">priority:urgent → queue.p0</span>
      </span>
    ),
    lastEvent: '3m ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: '⏱', bg: 'rgba(232,165,90,0.25)', color: '#7a4d10' },
    name: 'Cron',
    subtitle: 'internal scheduler',
    status: <AuthStatus dot="green">n/a</AuthStatus>,
    defaultProject: <Tag tone="teal">nightly</Tag>,
    routingRules: (
      <span className="text-[12px]">
        8 jobs · <span className="font-mono text-muted">02:00 schema-drift, 03:00 dep-audit</span>
      </span>
    ),
    lastEvent: '11h ago',
    action: { variant: 'secondary', label: 'Edit' },
  },
  {
    icon: { label: '$', bg: 'rgba(204,120,92,0.18)', color: 'var(--color-primary-active)' },
    name: 'Direct command',
    subtitle: (
      <>
        CLI · <span className="font-mono">sanchoris</span> v0.3.2
      </>
    ),
    status: <AuthStatus dot="muted">disabled by policy</AuthStatus>,
    defaultProject: <span className="font-mono text-muted">—</span>,
    routingRules: (
      <span className="text-[12px] text-muted">
        CLI access restricted to <span className="font-mono">role:operator</span>
      </span>
    ),
    lastEvent: '—',
    action: { variant: 'secondary', label: 'Enable' },
  },
];

// ─── Page ──────────────────────────────────────────────────────────────────────

export function ChannelsPage() {
  return (
    <AppShell
      rail={<Rail items={channelRailItems} bottomItems={ACCOUNT_ITEMS} />}
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Settings', 'Channels']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Channels
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Inbound surfaces that produce tasks · routed into projects ·{' '}
                <strong className="font-semibold text-body-strong">9 configured</strong> ·{' '}
                <strong className="font-semibold text-body-strong">7 active</strong> · updated
                13:42 JST today by{' '}
                <strong className="font-semibold text-body-strong">kanagawa-ops</strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Test ingress</Btn>
              <Btn variant="primary">+ Connect channel</Btn>
            </div>
          </div>
        </ScreenHead>

        <ScreenBody>
          {/* Filter row */}
          <div className="mb-[14px] flex flex-wrap items-center gap-[8px]">
            <Chip active count={9}>All</Chip>
            <Chip count={7}>Connected</Chip>
            <Chip count={1}>Needs auth</Chip>
            <Chip count={1}>Disabled</Chip>
            <Chip dot count={37} className="ml-[6px]">Routing rules ·</Chip>
            <span className="ml-auto flex items-center gap-[8px] text-[12px] text-muted">
              <span>Density</span>
              <Kbd>comfortable</Kbd>
              <span>· Sort: last event</span>
            </span>
          </div>

          {/* Channels table */}
          <div className="overflow-hidden rounded-[10px] border border-hairline bg-canvas">
            <table className="w-full border-separate border-spacing-0 text-[13px]">
              <thead>
                <tr>
                  <th
                    style={{ width: 280 }}
                    className="border-b border-hairline bg-surface-soft px-[16px] py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                  >
                    Channel
                  </th>
                  <th
                    style={{ width: 160 }}
                    className="border-b border-hairline bg-surface-soft px-[16px] py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                  >
                    Status
                  </th>
                  <th
                    style={{ width: 160 }}
                    className="border-b border-hairline bg-surface-soft px-[16px] py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                  >
                    Default project
                  </th>
                  <th className="border-b border-hairline bg-surface-soft px-[16px] py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted">
                    Routing rules
                  </th>
                  <th
                    style={{ width: 120 }}
                    className="border-b border-hairline bg-surface-soft px-[16px] py-[10px] text-left text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted"
                  >
                    Last event
                  </th>
                  <th
                    style={{ width: 90 }}
                    className="border-b border-hairline bg-surface-soft px-[16px] py-[10px]"
                  />
                </tr>
              </thead>
              <tbody>
                {CHANNELS.map((ch, i) => {
                  const isLast = i === CHANNELS.length - 1;
                  const cellCls = `align-middle px-[16px] py-[13px] ${isLast ? '' : 'border-b border-hairline-soft'}`;
                  return (
                    <tr key={ch.name} className="hover:bg-surface-soft/60">
                      <td className={cellCls}>
                        <div className="flex items-center gap-[10px]">
                          <ChannelIcon
                            label={ch.icon.label}
                            bg={ch.icon.bg}
                            color={ch.icon.color}
                          />
                          <div>
                            <div className="text-[13px] font-medium text-ink">{ch.name}</div>
                            <div className="font-mono text-[11px] text-muted">{ch.subtitle}</div>
                          </div>
                        </div>
                      </td>
                      <td className={cellCls}>{ch.status}</td>
                      <td className={cellCls}>{ch.defaultProject}</td>
                      <td className={cellCls}>{ch.routingRules}</td>
                      <td className={`${cellCls} font-mono text-[12px] text-muted`}>
                        {ch.lastEvent}
                      </td>
                      <td className={`${cellCls} text-right`}>
                        <Btn variant={ch.action.variant} size="sm">
                          {ch.action.label}
                        </Btn>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
