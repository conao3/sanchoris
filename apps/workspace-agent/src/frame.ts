export const MAX_FRAME_BYTES = 120 * 1024
const MARK = "…[truncated]"

function cut(value: unknown, limit: number): unknown {
  if (typeof value === "string") return value.length > limit ? value.slice(0, limit) + MARK : value
  if (Array.isArray(value)) return value.map((item) => cut(item, limit))
  if (value !== null && typeof value === "object")
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, cut(item, limit)]))
  return value
}

/** The message with its longest strings shortened until it fits in one channel frame. */
export function fitFrame<T>(message: T): T {
  if (Buffer.byteLength(JSON.stringify(message)) <= MAX_FRAME_BYTES) return message
  let limit = MAX_FRAME_BYTES / 2
  let fitted = cut(message, limit) as T
  while (Buffer.byteLength(JSON.stringify(fitted)) > MAX_FRAME_BYTES && limit > 64) {
    limit = Math.floor(limit / 2)
    fitted = cut(message, limit) as T
  }
  return fitted
}
