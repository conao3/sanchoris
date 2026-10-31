import clsx from "clsx"
import { type CSSProperties, type ReactNode, useId, useRef, useState } from "react"

const GAP = 4
const MARGIN = 8

export function Reason({
  reason,
  align = "left",
  className,
  children,
}: {
  reason?: string
  align?: "left" | "right"
  className?: string
  children: ReactNode
}) {
  const id = useId()
  const anchor = useRef<HTMLSpanElement>(null)
  const [place, setPlace] = useState<CSSProperties>()
  if (!reason) {
    return children
  }

  function show() {
    const box = anchor.current?.getBoundingClientRect()
    if (!box) {
      return
    }
    setPlace(
      align === "right"
        ? { top: box.bottom + GAP, right: Math.max(MARGIN, window.innerWidth - box.right) }
        : { top: box.bottom + GAP, left: Math.max(MARGIN, box.left) },
    )
  }

  return (
    <span
      ref={anchor}
      tabIndex={0}
      aria-describedby={id}
      onMouseEnter={show}
      onMouseLeave={() => setPlace(undefined)}
      onFocus={show}
      onBlur={() => setPlace(undefined)}
      className={clsx(
        "inline-flex rounded-ui outline-none focus-visible:ring-2 focus-visible:ring-line2 [&>*]:pointer-events-none",
        className,
      )}
    >
      {children}
      <span
        id={id}
        role="tooltip"
        style={place}
        className={clsx(
          "fixed z-40 w-max max-w-[min(260px,calc(100vw-16px))] whitespace-normal rounded-ui border border-ink bg-ink px-2.5 py-1.5 text-left text-xs font-normal text-white",
          place ? "block" : "hidden",
        )}
      >
        {reason}
      </span>
    </span>
  )
}
