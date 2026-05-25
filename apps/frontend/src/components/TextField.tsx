import type { ReactNode } from 'react';
import {
  Input,
  Label,
  TextField as RACTextField,
  type TextFieldProps as RACTextFieldProps,
  Text,
} from 'react-aria-components';

type TextFieldProps = Omit<RACTextFieldProps, 'className' | 'children'> & {
  label: string;
  description?: ReactNode;
  placeholder?: string;
  autoComplete?: string;
  rightSlot?: ReactNode;
  className?: string;
};

export function TextField({
  label,
  description,
  placeholder,
  autoComplete,
  rightSlot,
  className,
  type = 'text',
  ...props
}: TextFieldProps) {
  return (
    <RACTextField
      {...props}
      type={type}
      className={`flex flex-col gap-1.5${className ? ` ${className}` : ''}`}
    >
      <div className="flex items-baseline justify-between gap-2">
        <Label className="text-sm font-medium text-body">{label}</Label>
        {rightSlot ? <span className="text-sm font-medium text-primary">{rightSlot}</span> : null}
      </div>
      <Input
        placeholder={placeholder}
        autoComplete={autoComplete}
        className="h-12 rounded-input border border-hairline bg-canvas px-3.5 text-base text-ink placeholder:text-muted-soft outline-none transition-colors focus:border-primary focus:bg-white focus:shadow-focus-primary"
      />
      {description ? (
        <Text slot="description" className="text-xs text-muted-soft">
          {description}
        </Text>
      ) : null}
    </RACTextField>
  );
}
