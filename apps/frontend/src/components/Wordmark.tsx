type WordmarkTone = 'on-light' | 'on-dark';
type WordmarkSize = 'sm' | 'md' | 'lg';

const sizeClass: Record<WordmarkSize, { text: string; glyph: string; gap: string }> = {
  sm: { text: 'text-base', glyph: 'size-3.5', gap: 'gap-2' },
  md: { text: 'text-h3', glyph: 'size-[18px]', gap: 'gap-2.5' },
  lg: { text: 'text-h2', glyph: 'size-6', gap: 'gap-3' },
};

type WordmarkProps = {
  tone?: WordmarkTone;
  size?: WordmarkSize;
  className?: string;
};

export function Wordmark({ tone = 'on-light', size = 'md', className }: WordmarkProps) {
  const text = tone === 'on-dark' ? 'text-on-dark' : 'text-ink';
  const glyphBorder = tone === 'on-dark' ? 'border-on-dark' : 'border-ink';
  const s = sizeClass[size];

  return (
    <span
      className={`inline-flex items-center ${s.gap} font-serif font-medium leading-tight tracking-tight ${s.text} ${text}${className ? ` ${className}` : ''}`}
    >
      <span
        aria-hidden="true"
        className={`relative inline-block ${s.glyph} rounded-xs border ${glyphBorder}`}
      >
        <span className="absolute left-0 top-0 size-[44%] bg-primary" />
      </span>
      <span>
        sanchoris<span className="text-primary not-italic">.</span>
      </span>
    </span>
  );
}
