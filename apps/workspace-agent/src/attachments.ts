import { randomUUID } from "node:crypto"
import { mkdir, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { basename, join } from "node:path"

import type { Attachment } from "./protocol.ts"
import type { WorkerImage, WorkerMessage } from "./worker.ts"

export async function downloadAttachments(
  attachments: readonly Attachment[],
): Promise<WorkerImage[]> {
  const images: WorkerImage[] = []
  for (const attachment of attachments) {
    if (basename(attachment.name) !== attachment.name || attachment.name.startsWith("."))
      throw new Error(`Attachment name ${JSON.stringify(attachment.name)} is not a plain file name`)
    const response = await fetch(attachment.url)
    if (!response.ok)
      throw new Error(`Downloading attachment ${attachment.name} failed with ${response.status}`)
    const bytes = Buffer.from(await response.arrayBuffer())
    const dir = join(tmpdir(), "sanchoris-attachments", randomUUID())
    await mkdir(dir, { recursive: true })
    const path = join(dir, attachment.name)
    await writeFile(path, bytes)
    images.push({
      name: attachment.name,
      mimeType: attachment.mimeType,
      path,
      data: bytes.toString("base64"),
    })
  }
  return images
}

export function workerMessage(
  text: string,
  steer: boolean,
  compact: boolean,
  images: WorkerImage[],
): WorkerMessage {
  const saved = images.map((image) => `[Attached image "${image.name}" is saved at: ${image.path}]`)
  return { text: [text, ...saved].join("\n"), steer, compact, images }
}
