import type { ReactNode } from 'react';
import { Button as RACButton, type ButtonProps as RACButtonProps } from 'react-aria-components';

type SSOButtonVariant = 'default' | 'primary';
type SSOIconTone = 'dark' | 'primary' | 'teal' | 'amber';

type SSOButtonProps = Omit<RACButtonProps, 'className' | 'children'> & {
  variant?: SSOButtonVariant;
  iconLabel: string;
  iconTone?: SSOIconTone;
  meta?: string;
  badge?: string;
  label: ReactNode;
  className?: string;
};

const variantClass: Record<SSOButtonVariant, string> = {
  default:
    'bg-canvas text-ink border-hairline hover:bg-surface-soft hover:border-cream-strong',
  primary:
    'bg-surface-dark text-on-dark border-surface-dark hover:bg-surface-dark-elevated hover:border-surface-dark-elevated',
};

const iconToneClass: Record<SSOIconTone, string> = {
  dark: 'bg-surface-dark text-on-dark',
  primary: 'bg-primary/20 text-primary-active',
  teal: 'bg-accent-teal/20 text-surface-dark',
  amber: 'bg-accent-amber/20 text-surface-dark',
};

export function SSOButton({
  variant = 'default',
  iconLabel,
  iconTone = 'dark',
  meta,
  badge,
  label,
  className,
  ...props
}: SSOButtonProps) {
  const metaColor = variant === 'primary' ? 'text-on-dark-soft' : 'text-muted';

  return (
    <RACButton
      {...props}
      className={`flex h-12 w-full items-center gap-3 rounded-input border px-4 text-base font-medium transition-colors outline-none focus-visible:shadow-focus-primary ${variantClass[variant]}${className ? ` ${className}` : ''}`}
    >
      <span
        aria-hidden="true"
        className={`inline-flex size-[22px] shrink-0 items-center justify-center rounded-sm font-mono text-xs font-bold ${iconToneClass[iconTone]}`}
      >
        {iconLabel}
      </span>
      <span className="flex-1 text-left">{label}</span>
      {meta ? <span className={`font-mono text-xs ${metaColor}`}>{meta}</span> : null}
      {badge ? (
        <span className="rounded-xs bg-primary px-1.5 py-0.5 text-[9.5px] font-bold uppercase tracking-wide text-on-primary">
          {badge}
        </span>
      ) : null}
    </RACButton>
  );
}
