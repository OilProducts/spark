import { rmSync } from "node:fs"
import { createServer } from "node:net"
import { defineConfig } from "@playwright/test"

// ponytail: the port is released before the server binds it, so a stranger can
// take it in between; the server then fails to bind and the run fails loudly.
const freePort = () =>
  new Promise<number>((resolve, reject) => {
    const server = createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      server.close(() => (address && typeof address === "object" ? resolve(address.port) : reject(new Error("no port"))))
    })
  })

// Workers load this config again and inherit the port through the environment.
process.env.SPARK_UI_SMOKE_PORT ||= String(await freePort())
const smokePort = Number.parseInt(process.env.SPARK_UI_SMOKE_PORT, 10)
if (!Number.isInteger(smokePort) || smokePort <= 0 || smokePort > 65535) {
  throw new Error(`Invalid SPARK_UI_SMOKE_PORT: ${process.env.SPARK_UI_SMOKE_PORT}`)
}

const baseURL = `http://127.0.0.1:${smokePort}`
const outputDir = `test-results/${smokePort}`

// The server script is killed outright, so the run's main process cleans up; a
// failed run keeps its traces.
if (process.env.TEST_WORKER_INDEX === undefined) {
  process.on("exit", code => {
    rmSync(`.tmp-ui-smoke-${smokePort}`, { recursive: true, force: true })
    if (code === 0) {
      rmSync(outputDir, { recursive: true, force: true })
    }
  })
}

export default defineConfig({
  testDir: "./e2e",
  // Playwright empties this at the start of a run; concurrent runs each keep their own.
  outputDir,
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
