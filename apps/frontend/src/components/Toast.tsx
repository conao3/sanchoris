import { createContext, type ReactNode, useContext, useEffect, useState } from "react"

const TOAST_MS = 3000

const ToastContext = createContext<(message: string) => void>(() => {})

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<{ message: string; at: number }>()

  useEffect(() => {
    const timer = toast ? window.setTimeout(() => setToast(undefined), TOAST_MS) : undefined
    return () => window.clearTimeout(timer)
  }, [toast])

  return (
    <ToastContext.Provider value={(message) => setToast({ message, at: Date.now() })}>
      {children}
      <div
        role="status"
        aria-live="polite"
        className="pointer-events-none fixed inset-x-0 bottom-12 z-40 flex justify-center px-4"
      >
        {toast ? (
          <div
            key={toast.at}
            className="rounded-ui border border-ink bg-ink px-3.5 py-2 text-sm text-white"
          >
            {toast.message}
          </div>
        ) : null}
      </div>
    </ToastContext.Provider>
  )
}

export function useToast(): (message: string) => void {
  return useContext(ToastContext)
}
