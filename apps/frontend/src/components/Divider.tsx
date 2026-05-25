import type { HTMLAttributes } from 'react';

type DividerProps = Omit<HTMLAttributes<HTMLDivElement>, 'children'> & {
  label?: string;
  tone?: 'default' | 'on-dark';
};

export function Divider({ label, tone = 'default', className, ...props }: DividerProps) {
  const lineColor = tone === 'on-dark' ? 'bg-on-dark-soft/30' : 'bg-hairline';
  const textColor = tone === 'on-dark' ? 'text-on-dark-soft' : 'text-muted-soft';

  if (label) {
    return (
      <div
        {...props}
        role="separator"
        className={`flex items-center gap-3 text-xs font-semibold uppercase tracking-wide ${textColor}${className ? ` ${className}` : ''}`}
      >
        <span aria-hidden="true" className={`h-px flex-1 ${lineColor}`} />
        <span>{label}</span>
        <span aria-hidden="true" className={`h-px flex-1 ${lineColor}`} />
      </div>
    );
  }
  return (
    <div
      {...props}
      role="separator"
      className={`h-px w-full ${lineColor}${className ? ` ${className}` : ''}`}
    />
  );
}
