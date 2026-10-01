import { defineConfig } from "@playwright/test"

const smokePort = Number.parseInt(process.env.SPARK_UI_SMOKE_PORT ?? "4173", 10)
if (!Number.isInteger(smokePort) || smokePort <= 0 || smokePort > 65535) {
  throw new Error(`Invalid SPARK_UI_SMOKE_PORT: ${process.env.SPARK_UI_SMOKE_PORT}`)
}

const baseURL = `http://127.0.0.1:${smokePort}`

export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  fullyParallel: false,
  // Every test shares one Spark home, and some change workspace-wide settings
  // (such as the default model provider) that other tests' chats inherit.
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL,
    headless: true,
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
  },
  webServer: {
    command: `node ./scripts/run-rust-smoke-server.mjs --port ${smokePort}`,
    url: baseURL,
    reuseExistingServer: false,
    timeout: 180_000,
  },
})
