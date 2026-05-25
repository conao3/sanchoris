import type { HTMLAttributes } from 'react';

type CardSurface = 'canvas' | 'soft' | 'card' | 'cream-strong' | 'dark';
type CardElevation = 'flat' | 'soft' | 'elevated' | 'dark';

const surfaceClass: Record<CardSurface, string> = {
  canvas: 'bg-canvas border-hairline text-ink',
  soft: 'bg-surface-soft border-hairline text-ink',
  card: 'bg-surface-card border-hairline-soft text-ink',
  'cream-strong': 'bg-cream-strong border-hairline text-ink',
  dark: 'bg-surface-dark border-surface-dark-elevated text-on-dark',
};

const elevationClass: Record<CardElevation, string> = {
  flat: '',
  soft: 'shadow-card-soft',
  elevated: 'shadow-card-elevated',
  dark: 'shadow-card-dark',
};

type CardProps = HTMLAttributes<HTMLDivElement> & {
  surface?: CardSurface;
  elevation?: CardElevation;
  padded?: boolean;
};

export function Card({
  surface = 'card',
  elevation = 'flat',
  padded = true,
  className,
  ...props
}: CardProps) {
  const padding = padded ? 'p-4' : '';
  return (
    <div
      {...props}
      className={`rounded-card border ${surfaceClass[surface]} ${elevationClass[elevation]} ${padding}${className ? ` ${className}` : ''}`}
    />
  );
}
