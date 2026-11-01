export const IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"]
export const MAX_ATTACHMENT_BYTES = 4 * 1024 * 1024
export const MAX_ATTACHMENTS = 10
const MAX_SIDE = 2048
const JPEG_QUALITY = 0.85

export type Draft = { key: string; name: string; preview: string; file?: File; id?: string }

function encode(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result
      if (typeof result !== "string") {
        reject(new Error(`could not read ${blob.type}`))
        return
      }
      resolve(result.replace(/^data:[^,]*,/, ""))
    }
    reader.onerror = () => reject(reader.error ?? new Error(`could not read ${blob.type}`))
    reader.readAsDataURL(blob)
  })
}

async function shrink(file: File): Promise<File> {
  const bitmap = await createImageBitmap(file)
  const scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height))
  const canvas = new OffscreenCanvas(
    Math.round(bitmap.width * scale),
    Math.round(bitmap.height * scale),
  )
  const context = canvas.getContext("2d")
  if (!context) {
    throw new Error(`${file.name} could not be read as an image.`)
  }
  context.drawImage(bitmap, 0, 0, canvas.width, canvas.height)
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: JPEG_QUALITY })
  if (blob.size > MAX_ATTACHMENT_BYTES) {
    throw new Error(`${file.name} is too large to attach, even after compression.`)
  }
  return new File([blob], file.name.replace(/\.[^.]+$/, "") + ".jpg", { type: "image/jpeg" })
}

/** The file as base64, shrunk to a JPEG when it is over the limit. */
export async function attachmentData(file: File): Promise<{ file: File; data: string }> {
  const fitted = file.size > MAX_ATTACHMENT_BYTES ? await shrink(file) : file
  return { file: fitted, data: await encode(fitted) }
}

export function imageFiles(files: Iterable<File>): File[] {
  return [...files].filter((file) => IMAGE_TYPES.includes(file.type))
}
