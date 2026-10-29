import clsx from 'clsx';
import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';
import { Button as RACButton } from 'react-aria-components';
import { Kbd } from './ui';

type ButtonVariant = 'default' | 'primary' | 'danger';

type ButtonProps = {
  children?: ReactNode;
  variant?: ButtonVariant;
  small?: boolean;
  icon?: LucideIcon;
  kbd?: string;
  href?: string;
  sameTab?: boolean;
  isDisabled?: boolean;
  type?: 'button' | 'submit';
  onPress?: () => void;
  className?: string;
};

const variantClass: Record<ButtonVariant, string> = {
  default: 'border-line2 bg-bg text-ink hover:bg-hover',
  primary: 'border-ink bg-ink text-white hover:bg-body',
  danger: 'border-line2 bg-bg text-err hover:bg-hover',
};

export function Button({
  children,
  variant = 'default',
  small,
  icon: Icon,
  kbd,
  href,
  sameTab,
  isDisabled,
  type = 'button',
  onPress,
  className,
}: ButtonProps) {
  const classes = clsx(
    'inline-flex items-center gap-1.5 whitespace-nowrap rounded-ui border font-medium outline-none focus-visible:ring-2 focus-visible:ring-line2 disabled:cursor-not-allowed disabled:opacity-50',
    small ? 'h-7 px-2.5 text-sm' : 'h-8 px-3 text-sm',
    variantClass[variant],
    className,
  );
  const content = (
    <>
      {Icon ? <Icon className="size-[15px] shrink-0" aria-hidden /> : null}
      {children}
      {kbd ? <Kbd tone={variant === 'primary' ? 'inverse' : 'button'}>{kbd}</Kbd> : null}
    </>
  );
  if (href) {
    return (
      <a href={href} target={sameTab ? undefined : '_blank'} rel={sameTab ? undefined : 'noreferrer'} className={classes}>
        {content}
      </a>
    );
  }
  return (
    <RACButton type={type} isDisabled={isDisabled} onPress={onPress} className={classes}>
      {content}
    </RACButton>
  );
}
