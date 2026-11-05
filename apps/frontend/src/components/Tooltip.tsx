import clsx from "clsx"
import { type DOMAttributes, type ReactElement, type ReactNode, useContext, useRef } from "react"
import { useFocusable } from "react-aria"
import {
  Focusable,
  Tooltip as RACTooltip,
  TooltipTrigger,
  TooltipTriggerStateContext,
} from "react-aria-components"

type Align = "left" | "right" | "side"

/**
 * The one tooltip of the app, for hints and for the reasons a control is disabled: it opens on
 * hover and keyboard focus, closes on Escape, and flips above its trigger when there is no room
 * below. The trigger is its first child, a react-aria component or an element given its focus
 * props.
 */
export function TipTrigger({
  tip,
  align = "left",
  children,
}: {
  tip: ReactNode
  align?: Align
  children: ReactNode
}) {
  return (
    <TooltipTrigger delay={0} closeDelay={0}>
      {children}
      <RACTooltip
        placement={align === "side" ? "right" : align === "right" ? "bottom end" : "bottom start"}
        offset={4}
        containerPadding={8}
        className="z-40 w-max max-w-[min(260px,calc(100vw-16px))] whitespace-pre-line rounded-ui border border-ink bg-ink px-2.5 py-1.5 text-left text-xs font-normal text-white"
      >
        {tip}
      </RACTooltip>
    </TooltipTrigger>
  )
}

/** A tooltip on a focusable element that is not a react-aria component, such as a link. */
export function Tip({
  tip,
  align,
  children,
}: {
  tip?: ReactNode
  align?: Align
  children: ReactElement<DOMAttributes<HTMLElement>, string>
}) {
  if (!tip) {
    return children
  }
  return (
    <TipTrigger tip={tip} align={align}>
      <Focusable>{children}</Focusable>
    </TipTrigger>
  )
}

/** Why the wrapped control is disabled, shown on hover, focus, and tap. */
export function Reason({
  reason,
  align,
  className,
  children,
}: {
  reason?: string
  align?: Align
  className?: string
  children: ReactNode
}) {
  if (!reason) {
    return children
  }
  return (
    <TipTrigger tip={reason} align={align}>
      <ReasonAnchor className={className}>{children}</ReasonAnchor>
    </TipTrigger>
  )
}

function ReasonAnchor({ className, children }: { className?: string; children: ReactNode }) {
  const ref = useRef<HTMLSpanElement>(null)
  const { focusableProps } = useFocusable({}, ref)
  const state = useContext(TooltipTriggerStateContext)
  return (
    <span
      ref={ref}
      {...focusableProps}
      onPointerUp={(event) => {
        if (event.pointerType === "touch") {
          state?.open(true)
        }
      }}
      className={clsx(
        "inline-flex rounded-ui outline-none focus-visible:ring-2 focus-visible:ring-line2 [&>*]:pointer-events-none",
        className,
      )}
    >
      {children}
    </span>
  )
}
