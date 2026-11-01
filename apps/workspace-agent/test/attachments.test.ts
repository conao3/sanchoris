import { readFile } from "node:fs/promises"
import { createServer } from "node:http"
import type { AddressInfo } from "node:net"

import { describe, expect, it } from "vite-plus/test"

import { downloadAttachments, workerMessage } from "../src/attachments.ts"
import { codexInput } from "../src/workers/codex.ts"

describe("attachments", () => {
  it("downloads each image, saves it, and names the saved paths in the text", async () => {
    const server = createServer((request, response) => {
      if (request.url === "/missing") {
        response.writeHead(404).end()
        return
      }
      response.writeHead(200, { "content-type": "image/png" }).end(Buffer.from([1, 2, 3]))
    })
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve))
    const { port } = server.address() as AddressInfo
    try {
      const images = await downloadAttachments([
        { name: "shot.png", mimeType: "image/png", url: `http://127.0.0.1:${port}/shot` },
      ])
      expect(images).toHaveLength(1)
      expect(images[0]?.data).toBe("AQID")
      expect(await readFile(images[0]?.path ?? "")).toEqual(Buffer.from([1, 2, 3]))
      const message = workerMessage("Why does it fail?", false, false, images)
      expect(message.text).toBe(
        `Why does it fail?\n[Attached image "shot.png" is saved at: ${images[0]?.path}]`,
      )
      expect(codexInput(message)).toEqual([
        { type: "text", text: message.text, text_elements: [] },
        { type: "image", url: "data:image/png;base64,AQID" },
      ])
      await expect(
        downloadAttachments([
          { name: "gone.png", mimeType: "image/png", url: `http://127.0.0.1:${port}/missing` },
        ]),
      ).rejects.toThrow("Downloading attachment gone.png failed with 404")
    } finally {
      server.close()
    }
  })
})
