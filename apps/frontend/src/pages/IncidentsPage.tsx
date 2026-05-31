import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn, Mono, Pill, Tag } from '../components/shell/primitives';
import { Avatar } from '../components/shell/Topbar';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';

// ─── Severity badge ───────────────────────────────────────────────────────────

type SevLevel = 1 | 2 | 3;

const sevClass: Record<SevLevel, string> = {
  1: 'bg-error text-white',
  2: 'bg-warning text-white',
  3: 'bg-surface-card text-ink',
};

function SevBadge({ level }: { level: SevLevel }) {
  return (
    <span
      className={`inline-block rounded-[4px] px-[6px] py-[2px] font-mono text-[10px] font-bold uppercase tracking-[0.8px] ${sevClass[level]}`}
    >
      sev-{level}
    </span>
  );
}

// ─── Avatar stack ─────────────────────────────────────────────────────────────

type AvatarDef = { initials: string; tone?: 'default' | 'coral' | 'teal' };

function AvStack({ avatars }: { avatars: AvatarDef[] }) {
  return (
    <div className="inline-flex items-center">
      {avatars.map((av, i) => (
        <div key={i} className={i > 0 ? '-ml-2' : ''} style={{ border: '2px solid var(--color-canvas)', borderRadius: '9999px' }}>
          <Avatar variant={av.tone === 'coral' ? 'coral' : av.tone === 'teal' ? 'teal' : 'sm'}>
            {av.initials}
          </Avatar>
        </div>
      ))}
    </div>
  );
}

// ─── IncidentCard ─────────────────────────────────────────────────────────────

type IncidentCardProps = {
  sev: SevLevel;
  id: string;
  selected?: boolean;
  title: string;
  startedAt: string;
  duration: string;
  services: string[];
  oncall: AvatarDef[];
  agents: AvatarDef[] | null;
};

function IncidentCard({
  sev,
  id,
  selected,
  title,
  startedAt,
  duration,
  services,
  oncall,
  agents,
}: IncidentCardProps) {
  const baseClass =
    'rounded-[10px] border cursor-pointer p-[14px_16px]';
  const selClass = selected
    ? 'border-primary bg-[rgba(204,120,92,0.04)] shadow-[0_0_0_3px_rgba(204,120,92,0.1)]'
    : 'border-hairline bg-canvas';

  return (
    <div className={`${baseClass} ${selClass}`}>
      <div className="mb-[6px] flex items-center gap-2">
        <SevBadge level={sev} />
        <span className="font-mono text-[11px] text-muted">
          {id}
          {selected ? ' · selected' : ''}
        </span>
      </div>
      <div className="mb-[6px] text-[13.5px] font-medium leading-[1.35] text-ink">{title}</div>
      <div className="mb-2 flex gap-2 font-mono text-[11px] text-muted">
        <span>started {startedAt}</span>
        <span>·</span>
        <span>{duration}</span>
      </div>
      <div className="mb-2 text-[11px] text-body">
        {services.map((svc) => (
          <Tag key={svc} className="mr-1">
            {svc}
          </Tag>
        ))}
      </div>
      <div className="flex items-center gap-2 text-[11px] text-muted">
        <span>on-call</span>
        <AvStack avatars={oncall} />
        <span>·</span>
        <span>agents</span>
        {agents ? <AvStack avatars={agents} /> : <span className="text-muted">none assigned</span>}
      </div>
    </div>
  );
}

// ─── Timeline row ─────────────────────────────────────────────────────────────

type TlRowProps = {
  timestamp: string;
  curr?: boolean;
  event: ReactNode;
  by: string;
};

function TlRow({ timestamp, curr, event, by }: TlRowProps) {
  return (
    <div
      className="grid items-baseline gap-[10px] border-b border-dashed border-hairline-soft py-[9px] text-[12px] last:border-b-0"
      style={{ gridTemplateColumns: '80px 14px 1fr 90px' }}
    >
      <span className="font-mono text-[11px] text-muted-soft">{timestamp}</span>
      <span
        className="mt-1 h-[9px] w-[9px] rounded-full"
        style={
          curr
            ? { background: 'var(--color-primary)', boxShadow: '0 0 0 3px rgba(204,120,92,0.2)' }
            : { background: 'var(--color-muted-soft)' }
        }
      />
      <span className="leading-[1.5] text-body">{event}</span>
      <span className="text-right font-mono text-[10.5px] text-muted-soft">{by}</span>
    </div>
  );
}

// ─── Timeline ─────────────────────────────────────────────────────────────────

function Timeline() {
  return (
    <div className="overflow-auto rounded-[10px] border border-hairline bg-canvas p-[16px_18px]">
      <h4 className="mb-3 font-serif text-[18px] font-medium leading-none tracking-[-0.2px] text-ink" style={{ margin: '0 0 12px' }}>
        INC-0492 · timeline
      </h4>
      <TlRow
        timestamp="+00:47:12"
        curr
        event={
          <>
            <strong className="font-semibold text-ink">codex</strong> proposes rollback of{' '}
            <Mono>edge-config-v18</Mono> · awaiting human ack
          </>
        }
        by="codex w-cx-1"
      />
      <TlRow
        timestamp="+00:38:04"
        event={
          <>
            <strong className="font-semibold text-ink">openhands</strong> correlated logs across 4
            PoPs · spike pattern matches TLS handshake retries
          </>
        }
        by="openhands"
      />
      <TlRow
        timestamp="+00:31:21"
        event={
          <>
            <strong className="font-semibold text-ink">kanagawa-ops</strong> joined · took comms
          </>
        }
        by="kanagawa-ops"
      />
      <TlRow
        timestamp="+00:24:55"
        event={
          <>
            <strong className="font-semibold text-ink">octocat</strong> joined · took IC
          </>
        }
        by="octocat"
      />
      <TlRow
        timestamp="+00:18:02"
        event={
          <>
            auto-paged on-call · gate <Mono>incident-comms</Mono> opened · status page draft
            prepared
          </>
        }
        by="sanchoris"
      />
      <TlRow
        timestamp="+00:14:18"
        event={
          <>
            workflow <Mono>incident v0.1.4</Mono> auto-started · assigned codex + openhands
          </>
        }
        by="sanchoris"
      />
      <TlRow
        timestamp="+00:07:42"
        event={
          <>
            downstream checkout error rate spiked to <Mono>2.4%</Mono> · SLO budget burning 12×
            baseline
          </>
        }
        by="obs"
      />
      <TlRow
        timestamp="+00:00:00"
        event={
          <>
            SLO alert <Mono>edge.error_rate &gt; 0.8%</Mono> fired · eu-west PoP fr3
          </>
        }
        by="obs"
      />
    </div>
  );
}

// ─── Runbook / PR link row ────────────────────────────────────────────────────

function RbRow({ icon, label, mono }: { icon: string; label: string; mono?: boolean }) {
  return (
    <div className="flex items-center gap-[7px] py-[5px] text-[12px] text-body">
      <span className={`text-[11px] text-muted-soft ${mono ? 'font-mono' : ''}`}>{icon}</span>
      <span className="flex-1 cursor-pointer text-primary">{label}</span>
    </div>
  );
}

// ─── IncidentRail ─────────────────────────────────────────────────────────────

function IncidentRail() {
  return (
    <div className="overflow-auto rounded-[10px] border border-hairline bg-surface-soft p-[16px]">
      <div>
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted" style={{ margin: '0 0 8px' }}>
          Runbook links
        </h5>
        <RbRow icon="→" label="Edge 502 triage" />
        <RbRow icon="→" label="Rollback edge-config" />
        <RbRow icon="→" label="EU PoP failover map" />
        <RbRow icon="→" label="Status page playbook" />
      </div>
      <div className="mt-4">
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted" style={{ margin: '0 0 8px' }}>
          Linked PRs
        </h5>
        <RbRow icon="#487" label="revert: edge-config-v18 → v17" mono />
        <RbRow icon="#488" label="chore: bump tls retry budget" mono />
      </div>
      <div className="mt-4">
        <h5 className="mb-2 text-[10.5px] font-semibold uppercase tracking-[1.4px] text-muted" style={{ margin: '0 0 8px' }}>
          Actions
        </h5>
        <Btn variant="primary" size="sm">
          <span className="w-full text-center">Page secondary on-call</span>
        </Btn>
        <div className="mt-[6px]">
          <Btn variant="secondary" size="sm">
            <span className="w-full text-center">Open war room (Slack)</span>
          </Btn>
        </div>
        <div className="mt-[6px]">
          <Btn variant="secondary" size="sm">
            <span className="w-full text-center">Update status page</span>
          </Btn>
        </div>
      </div>
    </div>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export function IncidentsPage() {
  const railItems = WORKSPACE_ITEMS.map((i) => ({ ...i, active: false }));

  return (
    <AppShell rail={<Rail items={railItems} bottomItems={ACCOUNT_ITEMS} />}>
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'Incidents']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Active incidents{' '}
                <span className="ml-3 inline-flex gap-2 align-middle">
                  <Pill kind="failed">1 sev-1</Pill>
                  <Pill kind="warn">1 sev-2</Pill>
                  <Pill kind="default">1 sev-3</Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Real-time runtime issues across the production gateway ·{' '}
                <strong className="font-semibold text-body-strong">auto-paged to on-call</strong> ·
                click an incident for its timeline
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">All incidents ⇗</Btn>
              <Btn variant="primary">Page on-call</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="flex min-h-0 flex-col gap-4 overflow-hidden">
            {/* Incident cards grid */}
            <div className="grid grid-cols-3 gap-3">
              <IncidentCard
                sev={1}
                id="INC-0492"
                selected
                title="Elevated 502 from EU edge — acme-platform"
                startedAt="12:55 JST today"
                duration="open 47 min"
                services={['acme-platform', 'edge / eu-west', 'checkout']}
                oncall={[{ initials: 'OC' }, { initials: 'EM' }]}
                agents={[{ initials: 'cx', tone: 'coral' }, { initials: 'oh', tone: 'teal' }]}
              />
              <IncidentCard
                sev={2}
                id="INC-0491"
                title="Webhook delivery lag p95 > 30s — stripe-bridge"
                startedAt="13:31 JST"
                duration="open 11 min"
                services={['stripe-bridge', 'webhooks']}
                oncall={[{ initials: 'HK' }]}
                agents={[{ initials: 'cx', tone: 'coral' }]}
              />
              <IncidentCard
                sev={3}
                id="INC-0490"
                title="Background job slow — nightly schema-drift sweep"
                startedAt="02:14 JST"
                duration="open 11 h 28 m"
                services={['cron', 'growth-api']}
                oncall={[{ initials: 'SH' }]}
                agents={null}
              />
            </div>

            {/* Timeline + rail */}
            <div
              className="min-h-0 flex-1 overflow-hidden"
              style={{ display: 'grid', gridTemplateColumns: '1fr 280px', gap: '16px' }}
            >
              <Timeline />
              <IncidentRail />
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
