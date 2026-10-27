import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["test/smoke/**/*.smoke.ts"],
    testTimeout: 300_000,
  },
});
