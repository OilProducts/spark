import { expect, test } from '@playwright/test'
import {
  cleanupSmokeFlowsForPage,
  createFlowForSmokeTest,
  deleteFlowAfterSmoke,
  ensureScreenshotDir,
  gotoWithRegisteredProject,
  runFlowInProject,
  screenshotPath,
  stubProjectMetadata,
} from '../fixtures/smoke-helpers'

test.beforeAll(() => {
  ensureScreenshotDir()
})

test.beforeEach(async ({ page }) => {
  await stubProjectMetadata(page)
})

test.afterEach(async ({ page }) => {
  await cleanupSmokeFlowsForPage(page)
})

const diagnosticFlow = (id: string, prompt: string) => ({
  schema_version: "1",
  id,
  title: id,
  nodes: {
    start: { kind: "start", label: "Start", config: { kind: "start" } },
    extract_declarations: { kind: "agent_task", label: "Extract Testable Declarations", config: { kind: "agent_task", prompt } },
    done: { kind: "exit", label: "Done", config: { kind: "exit" } },
  },
  edges: [{ from: "start", to: "extract_declarations" }, { from: "extract_declarations", to: "done" }],
})

// The editor previews the flow itself (carrying its token); the launch form previews it by name.
const previewIs = (body: string, token: string, flowName: string) => body.includes(token) || body.includes(flowName)

test("warning-only diagnostics still allow the editor run with explicit banner for item 7.2-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-warning-only-${Date.now()}`
  const promptToken = `warning-only-${Date.now()}`
  const warningMessage = `Warning-only diagnostic ${Date.now()}`
  const flowName = await createFlowForSmokeTest(page, "ui-smoke-warning-only", projectPath, diagnosticFlow('warning_only', promptToken))

  try {
    await page.route("**/attractor/preview", async (route) => {
      const body = route.request().postData() || ""
      if (previewIs(body, promptToken, flowName)) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "warning_only_state",
                severity: "warning",
                message: warningMessage,
              },
            ],
          }),
        })
        return
      }
      await route.continue()
    })

    await gotoWithRegisteredProject(page, projectPath)
    await page.getByTestId("activity-flows").click()
    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    const previewRequest = page.waitForRequest(
      (request) =>
        request.url().includes("/attractor/preview") &&
        request.method() === "POST" &&
        (request.postData() || "").includes(promptToken),
    )
    await flowButton.click()
    await previewRequest

    await expect(page.getByTestId("editor-run-button")).toBeEnabled()
    await runFlowInProject(page, projectPath)
    await expect(page.getByTestId("editor-run-panel")).toBeVisible()
    await expect(page.getByTestId("launch-panel-start-button")).toBeEnabled()
    await expect(page.getByTestId("execute-warning-banner")).toBeVisible()
    await expect(page.getByTestId("execute-warning-banner")).toContainText("Warnings present; run allowed.")
    await page.screenshot({ path: screenshotPath("16-warning-only-execute-banner.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("diagnostics transitions toggle the editor run blocking and warning state for item 7.2-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-diagnostic-transition-${Date.now()}`
  const errorToken = `diagnostic-error-${Date.now()}`
  const warningToken = `diagnostic-warning-${Date.now()}`
  const cleanToken = `diagnostic-clean-${Date.now()}`
  const errorFlowName = await createFlowForSmokeTest(page, "ui-smoke-diagnostic-error", projectPath, diagnosticFlow('diagnostic_error', errorToken))
  const warningFlowName = await createFlowForSmokeTest(page, "ui-smoke-diagnostic-warning", projectPath, diagnosticFlow('diagnostic_warning', warningToken))
  const cleanFlowName = await createFlowForSmokeTest(page, "ui-smoke-diagnostic-clean", projectPath, diagnosticFlow('diagnostic_clean', cleanToken))

  try {
    await page.route("**/attractor/preview", async (route) => {
      const body = route.request().postData() || ""
      if (previewIs(body, errorToken, errorFlowName)) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "blocking_error_transition",
                severity: "error",
                message: "Transition error diagnostic",
              },
              {
                rule_id: "warning_with_error_transition",
                severity: "warning",
                message: "Transition warning diagnostic",
              },
            ],
          }),
        })
        return
      }
      if (previewIs(body, warningToken, warningFlowName)) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "warning_only_transition",
                severity: "warning",
                message: "Transition warning diagnostic",
              },
            ],
          }),
        })
        return
      }
      if (previewIs(body, cleanToken, cleanFlowName)) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [],
          }),
        })
        return
      }
      await route.continue()
    })

    await gotoWithRegisteredProject(page, projectPath)
    await page.getByTestId("activity-flows").click()

    const waitForPreviewToken = (token: string) =>
      page.waitForRequest(
        (request) =>
          request.url().includes("/attractor/preview") &&
          request.method() === "POST" &&
          (request.postData() || "").includes(token),
      )

    const errorPreviewRequest = waitForPreviewToken(errorToken)
    await page.getByRole('button', { name: errorFlowName }).click()
    await errorPreviewRequest
    await expect(page.getByTestId("editor-run-button")).toBeDisabled()
    await expect(page.getByTestId("editor-run-button")).toHaveAttribute("title", "Fix validation errors before running.")

    const warningPreviewRequest = waitForPreviewToken(warningToken)
    await page.getByRole('button', { name: warningFlowName }).click()
    await warningPreviewRequest
    await expect(page.getByTestId("editor-run-button")).toBeEnabled()
    await runFlowInProject(page, projectPath)
    await expect(page.getByTestId("editor-run-panel")).toBeVisible()
    await expect(page.getByTestId("execute-warning-banner")).toBeVisible()
    await expect(page.getByTestId("execute-warning-banner")).toContainText("Warnings present; run allowed.")
    await page.screenshot({ path: screenshotPath("17-diagnostic-transition-execute-state.png"), fullPage: true })

    const cleanPreviewRequest = waitForPreviewToken(cleanToken)
    await page.getByRole('button', { name: cleanFlowName }).click()
    await cleanPreviewRequest
    // Switching flows closes the run panel; reopen it for the clean flow.
    await expect(page.getByTestId("editor-run-button")).toBeEnabled()
    await runFlowInProject(page, projectPath)
    await expect(page.getByTestId("editor-run-panel")).toBeVisible()
    await expect(page.getByTestId("launch-panel-start-button")).toBeEnabled()
    await expect(page.getByTestId("execute-warning-banner")).toHaveCount(0)
  } finally {
    await deleteFlowAfterSmoke(page, errorFlowName)
    await deleteFlowAfterSmoke(page, warningFlowName)
    await deleteFlowAfterSmoke(page, cleanFlowName)
  }
})

test("launch failures surface diagnostics and retry affordances for direct runs", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-launch-failure-${Date.now()}`
  const flowName = await createFlowForSmokeTest(page, "ui-smoke-launch-failure", projectPath)
  let pipelineStartAttempts = 0

  try {
    await page.route("**/attractor/pipelines", async (route) => {
      pipelineStartAttempts += 1
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ detail: "forced smoke launch failure" }),
      })
    })

    await gotoWithRegisteredProject(page, projectPath)
    await page.getByTestId("activity-flows").click()
    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

    await expect(page.getByTestId("editor-run-button")).toBeEnabled()
    await runFlowInProject(page, projectPath)
    await expect(page.getByTestId("editor-run-panel")).toBeVisible()
    await expect(page.getByTestId("launch-panel-flow-name")).toContainText(flowName)
    await expect(page.getByTestId("launch-panel-start-button")).toBeEnabled()
    await page.getByTestId("launch-panel-start-button").click()
    await expect(page.getByTestId("run-start-error-banner")).toContainText("forced smoke launch failure")
    await expect(page.getByTestId("launch-failure-diagnostics")).toBeVisible()
    await expect(page.getByTestId("launch-failure-message")).toContainText("forced smoke launch failure")
    await expect(page.getByTestId("launch-retry-button")).toBeEnabled()
    await expect.poll(() => pipelineStartAttempts).toBe(1)
    await page.screenshot({ path: screenshotPath("20-launch-failure-retry-enabled.png"), fullPage: true })

    await page.getByTestId("launch-retry-button").click()
    await expect.poll(() => pipelineStartAttempts).toBe(2)
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})
