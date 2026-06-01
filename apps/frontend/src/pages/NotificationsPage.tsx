import type { ReactNode } from 'react';
import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, ScreenBody, Crumbs } from '../components/shell/Screen';
import { Btn } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Shared page-local helpers ────────────────────────────────────────────────

function Quad({
  borderRight,
  borderBottom,
  children,
}: {
  borderRight?: boolean;
  borderBottom?: boolean;
  children: ReactNode;
}) {
  const cls = [
    'flex flex-col gap-[10px] overflow-hidden p-[14px_18px] min-h-0 min-w-0',
    borderRight ? 'border-r border-hairline' : '',
    borderBottom ? 'border-b border-hairline' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return <div className={cls}>{children}</div>;
}

function QuadHdr({
  iconClass,
  icon,
  name,
  channel,
}: {
  iconClass: string;
  icon: string;
  name: string;
  channel: string;
}) {
  return (
    <div className="flex flex-shrink-0 items-center gap-2">
      <span
        className={`inline-flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center rounded-[5px] font-mono text-[10.5px] font-bold ${iconClass}`}
      >
        {icon}
      </span>
      <span className="text-[13px] font-semibold text-ink">{name}</span>
      <span className="ml-auto font-mono text-[10.5px] text-muted">{channel}</span>
    </div>
  );
}

function GateTag({ children }: { children: ReactNode }) {
  return (
    <span className="mr-[6px] rounded-[3px] bg-[rgba(232,165,90,0.25)] px-[5px] py-[1px] font-mono text-[9.5px] font-bold tracking-[0.5px] text-[#7a4d10]">
      {children}
    </span>
  );
}

function InlineCode({ children }: { children: ReactNode }) {
  return (
    <code className="rounded-[3px] bg-surface-soft px-[5px] py-[1px] font-mono text-[10.5px] text-ink">
      {children}
    </code>
  );
}

// ─── Slack quad (top-left) ────────────────────────────────────────────────────

function SlackQuad() {
  return (
    <Quad borderRight borderBottom>
      <QuadHdr
        iconClass="bg-[rgba(93,184,166,0.22)] text-[#2c6e62]"
        icon="S"
        name="Slack"
        channel="#eng-review · 24 members"
      />
      <div className="flex min-h-0 flex-1 flex-col gap-[9px] overflow-hidden rounded-[8px] border border-hairline bg-canvas p-[11px_13px]">
        <div className="flex items-start gap-[10px]">
          <span className="inline-flex h-[30px] w-[30px] flex-shrink-0 items-center justify-center rounded-[6px] bg-surface-dark font-mono text-[10px] font-bold text-on-dark">
            san
          </span>
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline gap-[6px]">
              <span className="text-[13px] font-bold text-ink">Sanchoris</span>
              <span className="rounded-[3px] bg-surface-card px-[5px] py-[1px] text-[9px] font-bold tracking-[0.5px] text-muted">
                APP
              </span>
              <span className="font-mono text-[10px] text-muted-soft">13:42 JST</span>
            </div>
            <div className="mt-[3px] text-[12.5px] font-medium leading-[1.35] text-ink">
              <GateTag>Gate #1</GateTag>peer-review ·{' '}
              <span className="font-mono text-[11px]">CON-1247</span> Add OAuth login for vendor
              portal
            </div>
          </div>
        </div>
        <div className="text-[11.5px] leading-[1.5] text-body">
          Codeowner approval needed before merge to <InlineCode>main</InlineCode>. Touched files:{' '}
          <InlineCode>apps/auth/**</InlineCode> · PR <InlineCode>#482</InlineCode> · all checks
          green.
        </div>
        <div className="overflow-hidden rounded-[6px] bg-surface-dark px-[10px] py-[8px] font-mono text-[10.5px] leading-[1.55] text-on-dark">
          <div>
            <span className="text-[#e58176]">
              {'- if (Date.now() > exp) return redirect("/login");'}
            </span>
          </div>
          <div>
            <span className="text-[#7fcc8f]">
              {'+ const skewMs = opts?.allowSkewMs ?? 90_000;'}
            </span>
          </div>
          <div>
            <span className="text-[#7fcc8f]">
              {'+ if (Date.now() > exp + skewMs) return redirect("/login");'}
            </span>
          </div>
          <div>
            <span className="text-[#e58176]">
              {'- export function loginWithSession(jwt: string) {'}
            </span>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-[6px]">
          <button
            type="button"
            className="cursor-pointer rounded-[5px] border border-primary bg-primary px-[11px] py-[6px] font-[inherit] text-[12px] font-medium leading-[1] text-white"
            onClick={() => navigate('/runs/r-9143/gate')}
          >
            Approve
          </button>
          <button
            type="button"
            className="cursor-pointer rounded-[5px] border border-hairline bg-canvas px-[11px] py-[6px] font-[inherit] text-[12px] font-medium leading-[1] text-ink"
          >
            Reject with note
          </button>
          <span className="text-[11px] italic text-muted-soft">
            ↳ Reply in thread to leave a review note
          </span>
        </div>
      </div>
    </Quad>
  );
}

// ─── Telegram quad (top-right) ────────────────────────────────────────────────

function TelegramQuad() {
  return (
    <Quad borderBottom>
      <QuadHdr
        iconClass="bg-[rgba(74,144,226,0.22)] text-[#2a5f96]"
        icon="Tg"
        name="Telegram"
        channel="@sanchoris_oncall_bot"
      />
      <div className="flex min-h-0 flex-1 flex-col gap-[9px] overflow-hidden rounded-[8px] border border-hairline bg-canvas p-[11px_13px]">
        <div className="max-w-[96%] rounded-[9px_9px_9px_2px] bg-surface-soft p-[9px_11px]">
          <div className="mb-[3px] text-[11px] font-semibold text-[#2a5f96]">
            @sanchoris_oncall_bot · just now
          </div>
          <div className="mb-[4px] text-[12.5px] font-semibold leading-[1.35] text-ink">
            <GateTag>Gate #3</GateTag>prod-deploy · CON-1247
          </div>
          <div className="text-[11px] leading-[1.5] text-body">
            <strong className="font-semibold text-ink">2 of 2 required</strong> · AND rule ·{' '}
            <strong className="font-semibold text-ink">@oncall</strong> and{' '}
            <strong className="font-semibold text-ink">@pm</strong> must both approve. Staged at
            14:02 JST · smoke 10/10 green.
          </div>
        </div>
        <div className="flex flex-col gap-[4px]">
          <div className="px-[4px] font-mono text-[10px] text-muted">— @oncall (KO · you)</div>
          <div className="flex gap-[5px]">
            <button
              type="button"
              className="flex-1 cursor-pointer rounded-[6px] border border-[rgba(93,184,114,0.45)] bg-[rgba(93,184,114,0.06)] px-[10px] py-[7px] text-center font-[inherit] text-[11.5px] font-medium leading-[1] text-[#386b46]"
              onClick={() => navigate('/runs/r-9143/gate')}
            >
              ✓ Approve
            </button>
            <button
              type="button"
              className="flex-1 cursor-pointer rounded-[6px] border border-[rgba(198,69,69,0.35)] bg-canvas px-[10px] py-[7px] text-center font-[inherit] text-[11.5px] font-medium leading-[1] text-[#862e2e]"
            >
              × Reject
            </button>
          </div>
          <div className="px-[4px] font-mono text-[10px] text-muted">— @pm (HK · pending)</div>
          <div className="flex gap-[5px]">
            <button
              type="button"
              className="flex-1 cursor-pointer rounded-[6px] border border-[rgba(93,184,114,0.45)] bg-[rgba(93,184,114,0.06)] px-[10px] py-[7px] text-center font-[inherit] text-[11.5px] font-medium leading-[1] text-[#386b46]"
              onClick={() => navigate('/runs/r-9143/gate')}
            >
              ✓ Approve
            </button>
            <button
              type="button"
              className="flex-1 cursor-pointer rounded-[6px] border border-[rgba(198,69,69,0.35)] bg-canvas px-[10px] py-[7px] text-center font-[inherit] text-[11.5px] font-medium leading-[1] text-[#862e2e]"
            >
              × Reject
            </button>
          </div>
        </div>
        <div className="text-[10.5px] text-muted-soft">
          Each approver taps independently ·{' '}
          <strong className="font-semibold text-body">AND rule</strong> · 0 / 2 so far · timeout
          3h 38m
        </div>
      </div>
    </Quad>
  );
}

// ─── Email quad (bottom-left) ─────────────────────────────────────────────────

function EmailQuad() {
  return (
    <Quad borderRight>
      <QuadHdr
        iconClass="bg-surface-card text-ink"
        icon="@"
        name="Email"
        channel="human-review escape hatch"
      />
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[8px] border border-hairline bg-canvas">
        <div className="flex-shrink-0 border-b border-hairline-soft bg-surface-soft px-[13px] py-[9px] text-[11.5px]">
          <div className="font-semibold text-ink">
            Sanchoris{' '}
            <span className="font-mono text-[10px] font-normal text-muted">
              &lt;runs@sanchoris.dev&gt;
            </span>
          </div>
          <div className="mt-[1px] text-[10.5px] text-muted">
            to <span className="font-mono text-[10px] text-body">yuto.k@acme.io</span>
          </div>
        </div>
        <div className="flex-shrink-0 border-b border-hairline-soft px-[13px] py-[9px] text-[12.5px] font-semibold leading-[1.4] text-ink">
          <span className="font-mono text-[10px] font-medium text-muted">[Sanchoris]</span> Run
          blocked · <span className="font-mono text-[12px]">CON-1248</span> · Linear OAuth scope
          unclear
          <span className="mt-[3px] block font-mono text-[9.5px] font-normal italic text-muted-soft">
            no inbox involved — reply, or click an action below
          </span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-[9px] overflow-hidden px-[13px] py-[11px] text-[11.5px] leading-[1.55] text-body">
          <p className="m-0">
            The agent paused on <strong className="font-semibold text-ink">CON-1248</strong> at{' '}
            <InlineCode>Normalize</InlineCode>. The Linear ticket lists{' '}
            <strong className="font-semibold text-ink">two conflicting OAuth scopes</strong> in its
            description (<InlineCode>vendor:read</InlineCode> vs{' '}
            <InlineCode>vendor:admin</InlineCode>).
          </p>
          <div className="rounded-[6px] border border-hairline-soft bg-surface-soft px-[10px] py-[7px] font-mono text-[11px] leading-[1.5]">
            <strong className="font-semibold text-ink">Proposed:</strong> adopt{' '}
            <code className="font-mono text-[10.5px]">vendor:read</code> · matches the existing
            audit-log policy; reversible if PM disagrees.
          </div>
          <div className="mt-auto flex flex-wrap gap-[6px]">
            <button
              type="button"
              className="cursor-pointer rounded-[5px] border border-primary bg-primary px-[11px] py-[7px] font-[inherit] text-[11.5px] font-medium leading-[1] text-white"
              onClick={() => navigate('/runs/r-9143/gate')}
            >
              Resolve in 1-click · accept proposed
            </button>
            <button
              type="button"
              className="cursor-pointer rounded-[5px] border border-hairline bg-canvas px-[11px] py-[7px] font-[inherit] text-[11.5px] font-medium leading-[1] text-ink"
            >
              Reassign worker
            </button>
            <button
              type="button"
              className="cursor-pointer rounded-[5px] border border-hairline bg-canvas px-[11px] py-[7px] font-[inherit] text-[11.5px] font-medium leading-[1] text-ink"
            >
              Reply with instruction
            </button>
          </div>
        </div>
        <div className="flex-shrink-0 border-t border-hairline-soft bg-surface-soft px-[13px] py-[7px] text-[10.5px] text-muted-soft">
          <a className="cursor-pointer text-muted no-underline">Open Run timeline ↗</a> · you can
          always open the run, but no inbox lives here.
        </div>
      </div>
    </Quad>
  );
}

// ─── Web push + mobile quad (bottom-right) ────────────────────────────────────

function PushQuad() {
  return (
    <Quad>
      <QuadHdr
        iconClass="bg-[rgba(204,120,92,0.2)] text-primary-active"
        icon="!"
        name="Web push · mobile"
        channel="single-decision page"
      />
      <div className="flex min-h-0 flex-1 items-stretch gap-[12px] overflow-hidden">
        <div className="w-[175px] flex-shrink-0 self-start rounded-[12px] bg-[rgba(20,20,19,0.94)] p-[11px_13px] shadow-[0_8px_22px_rgba(20,20,19,0.18)]">
          <div className="mb-[5px] flex items-center gap-[7px]">
            <span className="inline-flex h-[14px] w-[14px] flex-shrink-0 items-center justify-center rounded-[4px] bg-primary font-mono text-[8.5px] font-bold text-white">
              S
            </span>
            <span className="text-[11px] font-semibold text-on-dark">Sanchoris</span>
            <span className="ml-auto font-mono text-[9.5px] text-on-dark-soft">now</span>
          </div>
          <div className="text-[12px] font-medium leading-[1.35] text-on-dark">
            Gate #3 prod-deploy needs your tap
          </div>
          <div className="mt-[4px] text-[10.5px] leading-[1.5] text-on-dark-soft">
            CON-1247 · 0 of 2 approved · you&apos;re @oncall.
          </div>
        </div>
        <div className="flex min-h-0 max-w-[220px] flex-1 flex-col self-stretch rounded-[18px] border border-hairline bg-surface-soft p-[7px] shadow-[0_4px_12px_rgba(20,20,19,0.06)]">
          <div className="flex flex-shrink-0 justify-between px-[9px] pb-[5px] pt-[3px] font-mono text-[9px] text-muted">
            <span>14:33</span>
            <span>●●●</span>
          </div>
          <div className="flex min-h-0 flex-1 flex-col gap-[8px] overflow-hidden rounded-[12px] bg-canvas p-[10px_11px]">
            <div className="text-[9px] font-semibold uppercase tracking-[1px] text-muted-soft">
              sanchoris · gate #3
            </div>
            <h6 className="m-0 font-serif text-[16px] font-medium leading-[1.2] tracking-[-0.2px] text-ink">
              Approve prod deploy?
            </h6>
            <div className="font-mono text-[10px] text-muted">CON-1247 · v0.3.3 → prod</div>
            <div className="flex items-center gap-[5px] rounded-[6px] border border-hairline-soft bg-surface-soft p-[6px_7px]">
              <div className="min-w-0 flex-1">
                <div className="text-[10.5px] font-semibold leading-[1.2] text-ink">
                  @oncall (you)
                </div>
                <div className="font-mono text-[9px] text-muted">pending</div>
              </div>
              <span className="rounded-full bg-[rgba(204,120,92,0.2)] px-[5px] py-[1px] font-mono text-[8px] font-bold tracking-[0.8px] text-primary-active">
                AND
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-[10.5px] font-semibold leading-[1.2] text-ink">@pm</div>
                <div className="font-mono text-[9px] text-muted">pending</div>
              </div>
            </div>
            <div
              className="overflow-hidden rounded-[5px] bg-surface-dark px-[8px] py-[6px] font-mono text-[9px] leading-[1.5] text-on-dark"
              style={{ maxHeight: '50px' }}
            >
              <div>
                <span className="text-[#e58176]">{'- if (Date.now() > exp)'}</span>
              </div>
              <div>
                <span className="text-[#7fcc8f]">{'+ if (Date.now() > exp + skew)'}</span>
              </div>
            </div>
            <div className="mt-auto flex flex-col gap-[4px]">
              <button
                type="button"
                className="inline-flex h-[30px] w-full cursor-pointer items-center justify-center rounded-[6px] border border-primary bg-primary font-[inherit] text-[11.5px] font-semibold leading-[1] text-white"
                onClick={() => navigate('/runs/r-9143/gate')}
              >
                Approve
              </button>
              <button
                type="button"
                className="inline-flex h-[30px] w-full cursor-pointer items-center justify-center rounded-[6px] border border-hairline bg-canvas font-[inherit] text-[11.5px] font-semibold leading-[1] text-ink"
              >
                Defer 1h
              </button>
              <button
                type="button"
                className="inline-flex h-[30px] w-full cursor-pointer items-center justify-center rounded-[6px] border border-hairline bg-transparent font-[inherit] text-[11.5px] font-semibold leading-[1] text-error"
              >
                Reject
              </button>
            </div>
            <div className="text-center font-mono text-[8.5px] text-muted-soft">
              no list · no other items · one decision
            </div>
          </div>
        </div>
      </div>
    </Quad>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export function NotificationsPage() {
  return (
    <AppShell
      rail={
        <Rail
          items={WORKSPACE_ITEMS.map((i) => ({ ...i, active: false }))}
          bottomItems={ACCOUNT_ITEMS}
        />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', 'acme-platform', 'Notifications · runtime']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Notifications &amp; 1-tap decisions
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Where Gate-approval requests reach humans · not an inbox · not a queue · in the
                channel they&apos;re already in
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary">Channel settings</Btn>
              <Btn variant="secondary">Decision log</Btn>
            </div>
          </div>
        </ScreenHead>
        <ScreenBody>
          <div className="-mx-7 -mb-6 -mt-[18px] flex min-h-0 flex-1 flex-col">
            <div className="flex flex-shrink-0 items-center gap-3 border-b border-hairline bg-surface-soft px-6 py-3 text-[13px] leading-[1.5] text-body">
              <span className="inline-flex h-[22px] w-[22px] flex-shrink-0 items-center justify-center rounded-full bg-[rgba(204,120,92,0.16)] text-[13px] font-bold text-primary-active">
                !
              </span>
              <span>
                <strong className="font-semibold text-ink">Humans never open an inbox.</strong>{' '}
                Decisions are pushed to the channel they&apos;re already in, and they answer in
                place — with one tap, on the device they&apos;re holding.
              </span>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-2 grid-rows-2">
              <SlackQuad />
              <TelegramQuad />
              <EmailQuad />
              <PushQuad />
            </div>
          </div>
        </ScreenBody>
      </Screen>
    </AppShell>
  );
}
