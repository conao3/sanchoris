import { describe, expect, it } from "vite-plus/test"

import { MAX_FRAME_BYTES, fitFrame } from "../src/frame.ts"

describe("fitFrame", () => {
  it("leaves small messages alone and shortens long strings until the frame fits", () => {
    const small = { type: "event", data: { text: "hello" } }
    expect(fitFrame(small)).toBe(small)
    const big = {
      type: "event",
      turn: 1,
      data: { name: "Write", input: { content: "x".repeat(300_000), path: "a.txt" } },
    }
    const fitted = fitFrame(big)
    expect(Buffer.byteLength(JSON.stringify(fitted))).toBeLessThanOrEqual(MAX_FRAME_BYTES)
    expect(fitted.data.input.path).toBe("a.txt")
    expect(fitted.data.input.content.endsWith("…[truncated]")).toBe(true)
    expect(fitted.turn).toBe(1)
  })
})
