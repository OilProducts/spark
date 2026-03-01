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
  await expect(page.getByTestId("project-metadata-name")).toBeVisible()
  await expect(page.getByTestId("project-metadata-directory")).toBeVisible()
  await expect(page.getByTestId("project-metadata-branch")).toBeVisible()
  await expect(page.getByTestId("project-metadata-last-activity")).toBeVisible()
  await expect(page.getByTestId("project-metadata-branch")).toContainText("Branch:")
  await expect(page.getByTestId("project-metadata-last-activity")).toContainText("Last activity:")
  await expect(page.getByTestId("project-metadata-last-activity")).not.toContainText("No activity yet")
  await page.getByTestId("project-metadata-last-activity").scrollIntoViewIfNeeded()
  await expect(page.getByTestId("top-nav-active-project")).toContainText("/tmp/ui-smoke-project")
  await page.screenshot({ path: screenshotPath("02-projects-panel.png"), fullPage: true })

  const proposalPreviewButton = page.getByTestId("project-spec-edit-proposal-preview-button")
  await expect(proposalPreviewButton).toBeVisible()
  await proposalPreviewButton.click()
  const proposalPreview = page.getByTestId("project-spec-edit-proposal-preview")
  await expect(proposalPreview).toBeVisible()
  await expect(proposalPreview).toContainText("Proposal preview")
  await expect(proposalPreview).toContainText("Before:")
  await expect(proposalPreview).toContainText("After:")
  await page.screenshot({ path: screenshotPath("02b-spec-edit-proposal-preview.png"), fullPage: true })

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

test("prompt edits trigger live preview diagnostics before blur for item 5.1-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-live-${Date.now()}`
  const promptToken = `live-prompt-${Date.now()}`
  const diagnosticMessage = `Live prompt diagnostic ${Date.now()}`

  await page.route("**/preview", async (route) => {
    const body = route.request().postData() || ""
    if (body.includes(promptToken)) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          status: "ok",
          diagnostics: [
            {
              rule_id: "live_prompt",
              severity: "warning",
              message: diagnosticMessage,
            },
          ],
        }),
      })
      return
    }
    await route.continue()
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-editor").click()

  const firstFlowButton = page.locator("button").filter({ hasText: ".dot" }).first()
  await expect(firstFlowButton).toBeVisible()
  await firstFlowButton.click()

  await expect(page.getByRole("button", { name: "Add Node" })).toBeVisible()
  await page.getByRole("button", { name: "Add Node" }).click()

  const newNode = page.locator(".react-flow__node").filter({ hasText: "New Node" }).last()
  await expect(newNode).toBeVisible()
  await newNode.click()

  const promptField = page.getByPlaceholder("Enter system prompt instructions...")
  await expect(promptField).toBeVisible()

  const previewRequest = page.waitForRequest(
    (request) =>
      request.url().includes("/preview") &&
      request.method() === "POST" &&
      (request.postData() || "").includes(promptToken),
  )

  await promptField.fill(promptToken)
  await expect(promptField).toBeFocused()
  await previewRequest
  await expect(page.getByText(diagnosticMessage)).toBeVisible()
  await page.screenshot({ path: screenshotPath("09-live-prompt-diagnostics.png"), fullPage: true })
})

test("stylesheet parse diagnostics render in graph settings for item 6.5-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-stylesheet-${Date.now()}`
  const stylesheetToken = ".bad$class { llm_model: gpt-5; }"
  const diagnosticMessage = `Stylesheet syntax diagnostic ${Date.now()}`

  await page.route("**/preview", async (route) => {
    const body = route.request().postData() || ""
    if (body.includes(stylesheetToken)) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          status: "ok",
          diagnostics: [
            {
              rule_id: "stylesheet_syntax",
              severity: "error",
              message: diagnosticMessage,
              line: 1,
            },
          ],
        }),
      })
      return
    }
    await route.continue()
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-editor").click()

  const firstFlowButton = page.locator("button").filter({ hasText: ".dot" }).first()
  await expect(firstFlowButton).toBeVisible()
  await firstFlowButton.click()

  await expect(page.locator('[data-inspector-scope="graph"]')).toBeVisible()
  const advancedToggle = page.getByTestId("graph-advanced-toggle")
  await expect(advancedToggle).toBeVisible()
  await advancedToggle.click()
  await expect(page.getByTestId("graph-model-stylesheet-editor")).toBeVisible()

  const previewRequest = page.waitForRequest(
    (request) =>
      request.url().includes("/preview") &&
      request.method() === "POST" &&
      (request.postData() || "").includes(stylesheetToken),
  )

  const stylesheetInput = page.getByTestId("model-stylesheet-editor").locator("textarea")
  await stylesheetInput.fill(stylesheetToken)
  await previewRequest

  await expect(page.getByTestId("graph-model-stylesheet-selector-guidance")).toBeVisible()
  await expect(page.getByTestId("graph-model-stylesheet-diagnostics").getByText(diagnosticMessage)).toBeVisible()
  await page.screenshot({ path: screenshotPath("10-stylesheet-diagnostics.png"), fullPage: true })
})
