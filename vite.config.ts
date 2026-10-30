import { resolve } from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig, type ViteUserConfig } from "vite-plus"
import type { PackUserConfig } from "vite-plus/pack"

const backendUrl = process.env.VITE_BACKEND_URL
const backendPaths = ["/api", "/auth/github", "/webhooks", "/records", "/runs"]

function nodeBundle(name: string, entry: Record<string, string>, outDir: string) {
  return {
    name,
    entry,
    outDir,
    platform: "node",
    target: "node26",
    format: "esm",
    dts: false,
    deps: { alwaysBundle: [/.*/] },
    outExtensions: () => ({ js: ".mjs" }),
  } satisfies PackUserConfig
}

export default defineConfig({
  root: resolve(import.meta.dirname, "apps/frontend"),
  fmt: {
    semi: false,
    singleQuote: false,
    trailingComma: "all",
    sortPackageJson: false,
    ignorePatterns: [
      "**/*.md",
      "**/generated/**",
      "apps/frontend/src/graphql/schema.graphql",
      "crates/*/tests/fixtures/**",
    ],
  },
  lint: {
    categories: { correctness: "error", suspicious: "warn" },
    ignorePatterns: ["node_modules", "dist", ".direnv", "**/generated/**"],
    options: { typeAware: true, typeCheck: true },
  },
  test: {
    root: import.meta.dirname,
    setupFiles: ["./test/msw.setup.ts"],
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["apps/*/test/**/*.test.ts", "packages/*/test/**/*.test.ts"],
          testTimeout: 20_000,
        },
      },
      {
        extends: true,
        test: {
          name: "smoke",
          include: ["apps/*/test/smoke/**/*.smoke.ts"],
          testTimeout: 300_000,
        },
      },
    ],
  },
  plugins: [react(), tailwindcss()],
  build: {
    cssTarget: ["chrome88", "safari14", "firefox78", "edge88"],
  },
  server: {
    host: "127.0.0.1",
    proxy: backendUrl
      ? Object.fromEntries(
          backendPaths.map((path) => [
            path,
            { target: backendUrl, changeOrigin: true, ws: path === "/runs" },
          ]),
        )
      : undefined,
  },
  pack: [
    nodeBundle(
      "workspace-agent",
      {
        "workspace-agent": "apps/workspace-agent/src/bin/workspace-agent.ts",
        sanchoris: "apps/workspace-agent/src/bin/sanchoris.ts",
      },
      "apps/workspace-agent/dist",
    ),
    nodeBundle(
      "egress-relay",
      { "egress-relay": "apps/egress-relay/src/bin/egress-relay.ts" },
      "apps/egress-relay/dist",
    ),
  ],
  run: {
    tasks: {
      check: {
        command: "vp run @sanchoris/frontend#codegen:check && vp check && vp test --project unit",
      },
      "build-frontend": {
        command: "vp build",
        cache: { input: [{ auto: true }, "!apps/frontend/dist/**"] },
      },
    },
  },
} satisfies ViteUserConfig)
