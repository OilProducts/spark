import { mkdirSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { expect, test } from "@playwright/test"

const testDir = path.dirname(fileURLToPath(import.meta.url))
const screenshotDir = path.resolve(testDir, "..", "artifacts", "ui-smoke")

const screenshotPath = (name: string) => path.join(screenshotDir, name)

test.beforeAll(() => {
  mkdirSync(screenshotDir, { recursive: true })
})

test("primary UI shells render and can be navigated", async ({ page }) => {
  await page.goto("/")

  await expect(page.getByTestId("top-nav")).toBeVisible()
  await expect(page.getByTestId("nav-mode-projects")).toBeVisible()
  await expect(page.getByTestId("nav-mode-editor")).toBeVisible()
  await expect(page.getByTestId("nav-mode-settings")).toBeVisible()
  await expect(page.getByTestId("nav-mode-runs")).toBeVisible()
  await expect(page.getByTestId("projects-panel")).toBeVisible()
  await expect(page.getByTestId("canvas-workspace-primary")).toHaveCount(0)
  await page.screenshot({ path: screenshotPath("01-projects-shell.png"), fullPage: true })

  await page.getByTestId("project-path-input").fill("/tmp/ui-smoke-project")
  await page.getByTestId("project-register-button").click()
  await expect(page.getByTestId("project-registry-list").getByText("/tmp/ui-smoke-project")).toBeVisible()
  await expect(page.getByTestId("top-nav-active-project")).toContainText("/tmp/ui-smoke-project")
  await page.screenshot({ path: screenshotPath("02-projects-panel.png"), fullPage: true })

  await page.getByTestId("nav-mode-editor").click()
  const firstFlowButton = page.locator("button").filter({ hasText: ".dot" }).first()
  await expect(firstFlowButton).toBeVisible()
  await firstFlowButton.click()

  await expect(page.getByTestId("canvas-workspace-primary")).toBeVisible()
  await expect(page.locator('[data-inspector-scope="graph"]')).toBeVisible()
  await page.screenshot({ path: screenshotPath("03-graph-inspector.png"), fullPage: true })

  const firstNode = page.locator(".react-flow__node").first()
  await expect(firstNode).toBeVisible()
  await firstNode.click()

  await expect(page.locator('[data-inspector-scope="node"]')).toBeVisible()
  await page.screenshot({ path: screenshotPath("04-node-inspector.png"), fullPage: true })

  const firstEdge = page.locator(".react-flow__edge-interaction").first()
  await firstEdge.click({ force: true })

  await expect(page.locator('[data-inspector-scope="edge"]')).toBeVisible()
  await page.screenshot({ path: screenshotPath("05-edge-inspector.png"), fullPage: true })

  await page.getByTestId("nav-mode-execution").click()
  await expect(page.getByTestId("canvas-workspace-primary")).toBeVisible()
  await expect(page.getByText("Terminal Output")).toBeVisible()
  await page.screenshot({ path: screenshotPath("06-execution-panel.png"), fullPage: true })

  await page.getByTestId("nav-mode-settings").click()
  await expect(page.getByTestId("settings-panel")).toBeVisible()
  await expect(page.getByTestId("canvas-workspace-primary")).toHaveCount(0)
  await page.screenshot({ path: screenshotPath("07-settings-panel.png"), fullPage: true })

  await page.getByTestId("nav-mode-runs").click()
  await expect(page.getByTestId("runs-panel")).toBeVisible()
  await expect(page.getByTestId("canvas-workspace-primary")).toHaveCount(0)
  await page.screenshot({ path: screenshotPath("08-runs-panel.png"), fullPage: true })
})
