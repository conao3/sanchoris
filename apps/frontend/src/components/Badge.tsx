import type { HTMLAttributes } from 'react';

type BadgeTone = 'neutral' | 'primary' | 'success' | 'warning' | 'error' | 'on-dark';

const toneClass: Record<BadgeTone, string> = {
  neutral: 'bg-surface-soft text-muted border-hairline',
  primary: 'bg-primary text-on-primary border-primary',
  success: 'bg-canvas text-success border-success/40',
  warning: 'bg-canvas text-warning border-warning/40',
  error: 'bg-canvas text-error border-error/40',
  'on-dark': 'bg-surface-dark-elevated text-on-dark border-surface-dark-elevated',
};

type BadgeProps = HTMLAttributes<HTMLSpanElement> & {
  tone?: BadgeTone;
};

export function Badge({ tone = 'neutral', className, ...props }: BadgeProps) {
  return (
    <span
      {...props}
      className={`inline-flex items-center gap-1.5 rounded-pill border px-2 py-0.5 text-xs font-semibold ${toneClass[tone]}${className ? ` ${className}` : ''}`}
    />
  );
}
