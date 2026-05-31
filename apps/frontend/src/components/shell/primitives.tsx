import type { ReactNode } from 'react';

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-hairline border-b-2 bg-canvas px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-muted">
      {children}
    </span>
  );
}

export function KbdInverse({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-[2px] rounded-[4px] border border-[rgba(255,255,255,0.3)] border-b-2 bg-[rgba(255,255,255,0.2)] px-[6px] py-[1.5px] font-mono text-[11px] leading-none text-white">
      {children}
    </span>
  );
}

export type PillKind =
  | 'default'
  | 'running'
  | 'validating'
  | 'gate'
  | 'failed'
  | 'done'
  | 'queued'
  | 'draft'
  | 'success'
  | 'warn'
  | 'info-soft';

const pillKindClass: Record<PillKind, string> = {
  default: 'bg-surface-card text-ink',
  running: 'bg-primary text-white',
  validating: 'bg-surface-dark text-on-dark',
  gate: 'bg-accent-amber text-[#4a3306]',
  failed: 'bg-error text-white',
  done: 'bg-surface-card text-muted',
  queued: 'border border-hairline bg-canvas text-body',
  draft: 'border border-hairline bg-surface-soft text-muted',
  success: 'bg-[rgba(93,184,114,0.18)] text-[#386b46]',
  warn: 'bg-[rgba(212,160,23,0.16)] text-[#856200]',
  'info-soft': 'bg-[rgba(93,184,166,0.18)] text-[#2c6e62]',
};

export function Pill({ kind = 'default', children }: { kind?: PillKind; children: ReactNode }) {
  return (
    <span
      className={`inline-flex items-center gap-[6px] rounded-full px-[10px] py-[3px] text-[11.5px] font-medium leading-[1.4] whitespace-nowrap ${pillKindClass[kind]}`}
    >
      {children}
    </span>
  );
}

export type TagTone = 'default' | 'coral' | 'coral-soft' | 'amber' | 'navy' | 'teal' | 'err';

const tagToneClass: Record<TagTone, string> = {
  default: 'bg-surface-card text-ink',
  coral: 'bg-primary text-white',
  'coral-soft': 'bg-[rgba(204,120,92,0.16)] text-primary-active',
  amber: 'bg-[rgba(232,165,90,0.22)] text-[#7a4d10]',
  navy: 'bg-surface-dark text-on-dark',
  teal: 'bg-[rgba(93,184,166,0.18)] text-[#2c6e62]',
  err: 'bg-[rgba(198,69,69,0.16)] text-[#862e2e]',
};

export function Tag({
  tone = 'default',
  className,
  children,
}: {
  tone?: TagTone;
  className?: string;
  children: ReactNode;
}) {
  return (
    <span
      className={`inline-flex rounded-[4px] px-[7px] py-[2px] font-mono text-[11px] leading-[1.4] ${tagToneClass[tone]} ${className ?? ''}`}
    >
      {children}
    </span>
  );
}

export type BtnVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
export type BtnSize = 'default' | 'sm';

const btnVariantClass: Record<BtnVariant, string> = {
  primary: 'bg-primary text-white border-primary',
  secondary: 'bg-canvas text-ink border-hairline',
  ghost: 'bg-transparent text-body border-transparent',
  danger: 'bg-transparent text-error border-hairline',
};

const btnSizeClass: Record<BtnSize, string> = {
  default: 'h-[34px] px-[14px] text-[13px] rounded-[7px]',
  sm: 'h-7 px-[10px] text-[12px] rounded-[6px]',
};

export function Btn({
  variant = 'secondary',
  size = 'default',
  children,
  onClick,
  type = 'button',
}: {
  variant?: BtnVariant;
  size?: BtnSize;
  children: ReactNode;
  onClick?: () => void;
  type?: 'button' | 'submit' | 'reset';
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      className={`inline-flex cursor-pointer items-center gap-[7px] border font-medium ${btnVariantClass[variant]} ${btnSizeClass[size]}`}
    >
      {children}
    </button>
  );
}

export function Mono({ children }: { children: ReactNode }) {
  return <span className="font-mono">{children}</span>;
}
