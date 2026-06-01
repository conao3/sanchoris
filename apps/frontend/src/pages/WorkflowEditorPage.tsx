import { AppShell } from '../components/shell/AppShell';
import { Rail } from '../components/shell/Rail';
import { Screen, ScreenHead, Crumbs } from '../components/shell/Screen';
import { Btn, Pill } from '../components/shell/primitives';
import { WORKSPACE_ITEMS, ACCOUNT_ITEMS } from '../components/shell/railItems';
import { navigate } from '../lib/navigate';

// ─── Local graph components ───────────────────────────────────────────────────

type NodeState = 'default' | 'start' | 'sel' | 'end';

const nodeStateClass: Record<NodeState, string> = {
  default: 'border-[1.5px] border-hairline',
  start: 'border-[1.5px] border-accent-teal',
  sel: 'border-2 border-primary bg-[rgba(204,120,92,0.06)] shadow-[0_0_0_4px_rgba(204,120,92,0.12)]',
  end: 'border-[1.5px] border-surface-dark-elevated',
};

const nodeIcoClass: Record<NodeState, string> = {
  default: 'text-muted',
  start: 'text-[#2c6e62]',
  sel: 'text-primary',
  end: 'text-muted',
};

function WfNode({ num, label, state = 'default' }: { num: string; label: string; state?: NodeState }) {
  return (
    <div
      className={`relative min-w-[140px] rounded-[8px] bg-canvas p-[10px_14px] text-[13px] font-medium text-ink shadow-[0_2px_4px_rgba(20,20,19,0.04)] ${nodeStateClass[state]}`}
    >
      <span className={`mb-[2px] block font-mono text-[11px] tracking-[0.5px] ${nodeIcoClass[state]}`}>
        {num}
      </span>
      {label}
    </div>
  );
}

function WfEdge({ dir = 'right' }: { dir?: 'right' | 'left' }) {
  return (
    <span className="inline-flex items-center text-[16px] text-muted-soft">
      {dir === 'right' ? '→' : '←'}
    </span>
  );
}

function WfInspector() {
  return (
    <div className="flex flex-col gap-[10px] overflow-hidden border-l border-hairline bg-surface-soft px-[18px] py-[18px]">
      <h4 className="m-0 font-serif text-[20px] font-medium tracking-[-0.3px] text-ink">Gate</h4>
      <div className="text-[12px] text-muted">policy-driven approval point</div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">kind</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[7px] font-mono text-[12px] text-ink">
          merge-approval
        </div>
      </div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">required approvers</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[7px] font-mono text-[12px] text-ink">
          1 · from <span className="text-primary-active">prod-touching-files</span> match
        </div>
      </div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">policy</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[6px] font-mono text-[12px] text-ink">
          <div>high-risk-paths.yaml</div>
          <div className="mt-1 border-t border-hairline-soft pt-1 font-mono text-[11px] text-muted">
            matches <span className="font-mono">apps/auth/** apps/billing/**</span>
          </div>
        </div>
      </div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">on_approve</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[7px] font-mono text-[12px] text-ink">
          continue → node 10
        </div>
      </div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">on_reject</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[7px] font-mono text-[12px] text-ink">
          return to node 07 with note
        </div>
      </div>

      <div className="mt-2 text-[10.5px] font-semibold uppercase tracking-[1.3px] text-muted">timeout</div>
      <div>
        <div className="rounded-[6px] border border-hairline bg-canvas px-[10px] py-[7px] font-mono text-[12px] text-ink">
          12 h → escalate to oncall
        </div>
      </div>

      <div className="mt-auto flex gap-[6px] pt-[14px]">
        <Btn size="sm" variant="secondary">Test gate</Btn>
        <Btn size="sm" variant="primary">Save node</Btn>
      </div>
    </div>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export function WorkflowEditorPage() {
  return (
    <AppShell
      rail={
        <Rail
          items={WORKSPACE_ITEMS.map((i) => ({ ...i, active: i.key === 'workflows' }))}
          bottomItems={ACCOUNT_ITEMS}
        />
      }
    >
      <Screen>
        <ScreenHead>
          <Crumbs items={['acme-org', { label: 'Workflows', href: '/workflows' }, 'delivery/default']} />
          <div className="flex items-start justify-between gap-4">
            <div>
              <h1 className="m-0 font-serif text-[30px] font-medium leading-[1.15] tracking-[-0.6px] text-ink">
                Workflow editor · delivery/default{' '}
                <span className="ml-3 inline-flex items-center gap-2 align-middle">
                  <Pill kind="draft">draft v0.3.3</Pill>
                  <Pill kind="done">
                    <span className="inline-block h-[6px] w-[6px] rounded-full bg-muted-soft" />
                    published v0.3.2
                  </Pill>
                </span>
              </h1>
              <p className="mt-[6px] text-[13px] leading-[1.5] text-muted">
                Authoring the default delivery pipeline · 10 nodes ·{' '}
                <strong className="font-semibold text-body-strong">
                  last edited 11:08 JST today by kanagawa-ops
                </strong>
              </p>
            </div>
            <div className="flex flex-shrink-0 items-center gap-2 pt-1">
              <Btn variant="secondary" onClick={() => navigate('/workflows/delivery-default/history')}>History</Btn>
              <Btn variant="secondary">Diff vs v0.3.2</Btn>
              <Btn variant="secondary">Save draft</Btn>
              <Btn variant="primary" onClick={() => navigate('/workflows/delivery-default/publish')}>Publish v0.3.3</Btn>
            </div>
          </div>
        </ScreenHead>

        {/* wf-toolbar: Pan/Zoom hints, add chips, right-aligned status+zoom+fit */}
        <div className="flex flex-shrink-0 items-center gap-[14px] border-b border-hairline px-7 py-3">
          <span className="text-[12px] text-muted">
            Pan{' '}
            <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">
              Space
            </span>{' '}
            · Zoom{' '}
            <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">
              +
            </span>
            <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">
              -
            </span>
          </span>
          <div className="h-[18px] w-[1px] mx-1 bg-hairline" />
          <span className="inline-flex h-[22px] cursor-pointer items-center gap-[6px] rounded-[6px] border border-hairline bg-canvas px-[8px] py-[3px] text-[11px] text-body">
            + Node
          </span>
          <span className="inline-flex h-[22px] cursor-pointer items-center gap-[6px] rounded-[6px] border border-hairline bg-canvas px-[8px] py-[3px] text-[11px] text-body">
            + Branch
          </span>
          <span className="inline-flex h-[22px] cursor-pointer items-center gap-[6px] rounded-[6px] border border-hairline bg-canvas px-[8px] py-[3px] text-[11px] text-body">
            + Loop
          </span>
          <div className="ml-auto flex items-center gap-2">
            <Pill kind="success">
              <span className="inline-block h-[6px] w-[6px] rounded-full bg-[rgba(255,255,255,0.85)]" />
              validated
            </Pill>
            <span className="font-mono text-[12px] text-muted">zoom 100%</span>
            <Btn size="sm" variant="secondary">Fit</Btn>
          </div>
        </div>

        {/* wf-body: canvas + inspector 2-column grid */}
        <div className="flex-1 min-h-0 grid grid-cols-[1fr_360px]">
          {/* wf-canvas: dot-grid background, centered graph */}
          <div
            className="flex items-center justify-center overflow-hidden p-6"
            style={{
              background: '#faf9f5',
              backgroundImage: 'radial-gradient(circle, #e6dfd8 1px, transparent 1px)',
              backgroundSize: '20px 20px',
              backgroundPosition: '10px 10px',
            }}
          >
            {/* wf-graph: 5-row CSS grid (row, vline, row, vline, row) */}
            <div style={{ display: 'grid', gridTemplateRows: 'auto 24px auto 24px auto', gap: 0 }}>
              {/* Row 1: 01 → 02 → 03 */}
              <div className="flex items-center justify-start gap-[22px]">
                <WfNode num="01 · ingest" label="Source" state="start" />
                <WfEdge />
                <WfNode num="02" label="Normalize" />
                <WfEdge />
                <WfNode num="03" label="Prioritize" />
              </div>

              {/* Vline 1: aligned right to connect row1(03) → row2(04) */}
              <div
                className="w-[2px] h-6 bg-muted-soft opacity-[0.35]"
                style={{ marginLeft: 'auto', marginRight: '75px' }}
              />

              {/* Row 2 (reversed): 06 ← 05 ← 04, right-justified so 04 aligns with 03 */}
              <div className="flex items-center justify-end gap-[22px]">
                <WfNode num="06" label="Create workspace" />
                <WfEdge dir="left" />
                <WfNode num="05" label="Select worker" />
                <WfEdge dir="left" />
                <WfNode num="04" label="Select workflow" />
              </div>

              {/* Vline 2: aligned left to connect row2(06) → row3(07) */}
              <div
                className="w-[2px] h-6 bg-muted-soft opacity-[0.35]"
                style={{ marginLeft: '75px' }}
              />

              {/* Row 3: 07 → 08 → 09(selected) → 10(end) */}
              <div className="flex items-center justify-start gap-[22px]">
                <WfNode num="07" label="Execute" />
                <WfEdge />
                <WfNode num="08" label="Validate" />
                <WfEdge />
                <WfNode num="09 · selected" label="Gate" state="sel" />
                <WfEdge />
                <WfNode num="10 · sink" label="Update systems" state="end" />
              </div>
            </div>
          </div>

          {/* wf-inspector: Gate node properties */}
          <WfInspector />
        </div>
      </Screen>
    </AppShell>
  );
}
