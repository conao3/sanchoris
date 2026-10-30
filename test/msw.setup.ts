import { HttpNetworkFrame } from "msw/experimental"
import { afterAll, afterEach, beforeAll, expect } from "vite-plus/test"
import { server } from "./msw.ts"

const loopbackHosts = new Set(["127.0.0.1", "localhost", "[::1]"])

let unhandledFrameCount = 0

beforeAll(() =>
  server.listen({
    onUnhandledFrame: async ({ frame }) => {
      if (
        frame instanceof HttpNetworkFrame &&
        loopbackHosts.has(new URL(frame.data.request.url).hostname)
      ) {
        return
      }
      unhandledFrameCount += 1
      throw new Error(await frame.getUnhandledMessage())
    },
  }),
)
afterEach(() => {
  const count = unhandledFrameCount
  unhandledFrameCount = 0
  server.resetHandlers()
  expect(count).toBe(0)
})
afterAll(() => server.close())
