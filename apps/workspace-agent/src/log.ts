export type Logger = (
  level: "info" | "warn" | "error",
  message: string,
  fields?: Record<string, unknown>,
) => void

export const stderrLogger: Logger = (level, message, fields) => {
  process.stderr.write(
    `${JSON.stringify({ at: new Date().toISOString(), level, message, ...fields })}\n`,
  )
}
