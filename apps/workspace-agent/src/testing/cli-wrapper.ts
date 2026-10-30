import { chmod, mkdir, writeFile } from "node:fs/promises"
import { join, resolve } from "node:path"

const cliEntry = resolve(import.meta.dirname, "../bin/sanchoris.ts")

export async function writeCliWrapper(binDir: string): Promise<void> {
  await mkdir(binDir, { recursive: true })
  const path = join(binDir, "sanchoris")
  await writeFile(path, `#!/bin/sh\nexec "${process.execPath}" --no-warnings "${cliEntry}" "$@"\n`)
  await chmod(path, 0o755)
}
