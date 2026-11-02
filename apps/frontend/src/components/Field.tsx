import clsx from "clsx"
import type { ComponentProps, ReactNode } from "react"

type FieldLook = { small?: boolean; mono?: boolean }

function fieldClass({ small, mono }: FieldLook, className?: string) {
  return clsx(
    "rounded-ui border border-line2 bg-bg px-2.5 text-sm text-ink outline-none placeholder:text-muted focus:border-ink disabled:cursor-not-allowed disabled:bg-sub disabled:text-muted",
    small ? "h-7" : "h-8",
    mono && "font-mono",
    className,
  )
}

/** A one-line text field, 32px tall or 28px when small, set in mono for names, paths, and keys. */
export function Input({ small, mono, className, ...props }: ComponentProps<"input"> & FieldLook) {
  return <input {...props} className={fieldClass({ small, mono }, className)} />
}

/** A native select with the same frame as `Input`. */
export function Select({ small, mono, className, ...props }: ComponentProps<"select"> & FieldLook) {
  return <select {...props} className={fieldClass({ small, mono }, className)} />
}

/** A multi-line text field that grows when dragged, with the frame of `Input`. */
export function TextArea({ className, ...props }: ComponentProps<"textarea">) {
  return (
    <textarea
      {...props}
      className={clsx(
        "block min-h-16 w-full resize-y rounded-ui border border-line2 bg-bg px-2.5 py-1.5 text-sm text-ink outline-none placeholder:text-muted focus:border-ink disabled:cursor-not-allowed disabled:bg-sub disabled:text-muted",
        className,
      )}
    />
  )
}

/** The label above a field, tied to it by `htmlFor`. */
export function Label({
  htmlFor,
  className,
  children,
}: {
  htmlFor: string
  className?: string
  children: ReactNode
}) {
  return (
    <label htmlFor={htmlFor} className={clsx("mb-1 mt-3 block text-xs text-muted", className)}>
      {children}
    </label>
  )
}
