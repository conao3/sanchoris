import clsx from "clsx"
import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"
import { Button as RACButton } from "react-aria-components"
import { Reason, TipTrigger } from "./Tooltip.tsx"
import { Kbd } from "./ui.tsx"

type ButtonVariant = "default" | "primary" | "danger"

type ButtonProps = {
  children?: ReactNode
  variant?: ButtonVariant
  small?: boolean
  icon?: LucideIcon
  kbd?: string
  compact?: boolean
  href?: string
  isDisabled?: boolean
  reason?: string
  reasonAlign?: "left" | "right"
  tip?: string
  autoFocus?: boolean
  type?: "button" | "submit"
  onPress?: () => void
  className?: string
}

const variantClass: Record<ButtonVariant, string> = {
  default: "border-line2 bg-bg text-ink hover:bg-hover",
  primary: "border-ink bg-ink text-white hover:bg-body",
  danger: "border-line2 bg-bg text-err hover:bg-hover",
}

export function Button({
  children,
  variant = "default",
  small,
  icon: Icon,
  kbd,
  compact,
  href,
  isDisabled,
  reason,
  reasonAlign,
  tip,
  autoFocus,
  type = "button",
  onPress,
  className,
}: ButtonProps) {
  const classes = clsx(
    "inline-flex items-center gap-1.5 whitespace-nowrap rounded-ui border font-medium outline-none focus-visible:ring-2 focus-visible:ring-line2 disabled:cursor-not-allowed disabled:opacity-50",
    small ? "h-7 px-2.5 text-sm" : "h-8 px-3 text-sm",
    compact && "max-md:w-8 max-md:justify-center max-md:px-0",
    variantClass[variant],
    className,
  )
  const content = (
    <>
      {Icon ? <Icon className="size-[15px] shrink-0" aria-hidden /> : null}
      {compact ? <span className="max-md:sr-only">{children}</span> : children}
      {kbd ? <Kbd tone={variant === "primary" ? "inverse" : "button"}>{kbd}</Kbd> : null}
    </>
  )
  if (href && !reason) {
    return (
      <a href={href} target="_blank" rel="noreferrer" className={classes}>
        {content}
      </a>
    )
  }
  const button = (
    <RACButton
      type={type}
      isDisabled={isDisabled || !!reason}
      autoFocus={autoFocus}
      onPress={onPress}
      className={classes}
    >
      {content}
    </RACButton>
  )
  if (reason) {
    return (
      <Reason reason={reason} align={reasonAlign}>
        {button}
      </Reason>
    )
  }
  return tip ? (
    <TipTrigger tip={tip} align={reasonAlign}>
      {button}
    </TipTrigger>
  ) : (
    button
  )
}

type IconButtonTone = "plain" | "overlay"

type IconButtonProps = {
  icon: LucideIcon
  label: string
  tone?: IconButtonTone
  reason?: string
  align?: "left" | "right"
  onPress?: () => void
  className?: string
}

const iconButtonClass: Record<IconButtonTone, string> = {
  plain:
    "-m-0.5 rounded-kbd p-1 text-faint hover:text-ink disabled:cursor-not-allowed disabled:text-line2",
  overlay: "rounded-full bg-ink/70 p-0.5 text-white hover:bg-ink",
}

/**
 * A button that shows only its icon, named by its label for screen readers and in its tooltip.
 * `plain` sits in a line of text with a hit area larger than the icon, `overlay` sits on an image.
 */
export function IconButton({
  icon: Icon,
  label,
  tone = "plain",
  reason,
  align,
  onPress,
  className,
}: IconButtonProps) {
  const button = (
    <RACButton
      aria-label={label}
      isDisabled={!!reason}
      onPress={onPress}
      className={clsx(
        "outline-none focus-visible:ring-2 focus-visible:ring-line2",
        iconButtonClass[tone],
        className,
      )}
    >
      <Icon className={tone === "overlay" ? "size-3" : "size-3.5"} aria-hidden />
    </RACButton>
  )
  return reason ? (
    <Reason reason={reason} align={align}>
      {button}
    </Reason>
  ) : (
    <TipTrigger tip={label} align={align}>
      {button}
    </TipTrigger>
  )
}
