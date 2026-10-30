import { chmod, mkdir, writeFile } from "node:fs/promises"
import { resolve } from "node:path"

const projectRoot = resolve(import.meta.dirname, "..")
const hooksDirectory = resolve(projectRoot, ".vite-hooks")
const preCommitPath = resolve(hooksDirectory, "pre-commit")
const preCommit = `#!/usr/bin/env sh
set -eu

nix develop --command make check
`

await mkdir(hooksDirectory, { recursive: true })
await writeFile(preCommitPath, preCommit, { mode: 0o755 })
await chmod(preCommitPath, 0o755)
