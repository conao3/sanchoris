import clsx from "clsx"
import type { LucideIcon } from "lucide-react"
import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react"
import { Button } from "./Button.tsx"

export type MenuOption<T extends string> = { value: T; label: ReactNode; detail?: ReactNode }

const VIEWPORT_MARGIN = 16

export function Menu<T extends string>({
  label,
  icon,
  title,
  options,
  value,
  onChange,
  align = "left",
  open: controlledOpen,
  onOpenChange,
  compact,
}: {
  label: ReactNode
  icon?: LucideIcon
  title?: ReactNode
  options: MenuOption<T>[]
  value?: T
  onChange: (value: T) => void
  align?: "left" | "right"
  open?: boolean
  onOpenChange?: (open: boolean) => void
  compact?: boolean
}) {
  const [localOpen, setLocalOpen] = useState(false)
  const open = controlledOpen ?? localOpen
  const setOpen = onOpenChange ?? setLocalOpen
  const ref = useRef<HTMLSpanElement>(null)
  const popup = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    const element = popup.current
    if (!element) {
      return
    }
    element.style.translate = ""
    const box = element.getBoundingClientRect()
    const overRight = box.right - (window.innerWidth - VIEWPORT_MARGIN)
    const overLeft = VIEWPORT_MARGIN - box.left
    element.style.translate =
      overRight > 0 ? `${-overRight}px 0` : overLeft > 0 ? `${overLeft}px 0` : ""
  }, [open])

  useEffect(() => {
    if (!open) {
      return
    }
    function onPointerDown(event: PointerEvent) {
      if (ref.current && !ref.current.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    window.addEventListener("pointerdown", onPointerDown)
    return () => window.removeEventListener("pointerdown", onPointerDown)
  }, [open, setOpen])

  return (
    <span ref={ref} className="relative inline-block">
      <Button icon={icon} compact={compact} onPress={() => setOpen(!open)}>
        {label}
      </Button>
      {open ? (
        <div
          ref={popup}
          className={clsx(
            "absolute top-9 z-20 w-[340px] max-w-[calc(100vw-2rem)] rounded-[8px] border border-line2 bg-bg py-1.5",
            align === "right" ? "right-0" : "left-0",
          )}
        >
          {title ? (
            <div className="px-3.5 pb-1 pt-1.5 text-xs font-semibold uppercase tracking-[.06em] text-faint">
              {title}
            </div>
          ) : null}
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              onClick={() => {
                setOpen(false)
                onChange(option.value)
              }}
              className={clsx(
                "block w-full px-3.5 py-[7px] text-left hover:bg-hover",
                option.value === value && "bg-hover shadow-[inset_2px_0_0_var(--color-ink)]",
              )}
            >
              <span className="font-medium">{option.label}</span>
              {option.detail ? <div className="text-muted">{option.detail}</div> : null}
            </button>
          ))}
        </div>
      ) : null}
    </span>
  )
}
