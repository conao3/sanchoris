import { Button as RACButton, type ButtonProps as RACButtonProps } from 'react-aria-components';

type ButtonVariant = 'primary' | 'secondary' | 'ghost';
type ButtonSize = 'sm' | 'md' | 'lg';

const variantClass: Record<ButtonVariant, string> = {
  primary:
    'bg-primary text-on-primary border border-primary hover:bg-primary-active pressed:bg-primary-active disabled:bg-primary-disabled disabled:border-primary-disabled disabled:text-muted-soft disabled:cursor-not-allowed',
  secondary:
    'bg-canvas text-ink border border-hairline hover:bg-surface-soft hover:border-cream-strong disabled:text-muted-soft disabled:cursor-not-allowed',
  ghost:
    'bg-transparent text-ink border border-transparent hover:bg-surface-soft disabled:text-muted-soft disabled:cursor-not-allowed',
};

const sizeClass: Record<ButtonSize, string> = {
  sm: 'h-8 px-3 text-sm',
  md: 'h-12 px-4 text-base',
  lg: 'h-14 px-6 text-md',
};

type ButtonProps = Omit<RACButtonProps, 'className'> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  className?: string;
};

export function Button({ variant = 'primary', size = 'md', className, ...props }: ButtonProps) {
  return (
    <RACButton
      {...props}
      className={`inline-flex items-center justify-center gap-2 rounded-input font-medium transition-colors outline-none focus-visible:shadow-focus-primary ${variantClass[variant]} ${sizeClass[size]}${className ? ` ${className}` : ''}`}
    />
  );
}
