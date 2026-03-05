import { expect, test } from '@playwright/test'
import {
  cleanupSmokeFlowsForPage,
  cloneFlowForSmokeTest,
  deleteFlowAfterSmoke,
  ensureScreenshotDir,
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

test("warning-only diagnostics still allow execute with explicit banner for item 7.2-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-warning-only-${Date.now()}`
  const promptToken = `warning-only-${Date.now()}`
  const warningMessage = `Warning-only diagnostic ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-warning-only")

  try {
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

    await page.goto("/")
    await page.getByTestId("project-path-input").fill(projectPath)
    await page.getByTestId("project-register-button").click()
    await page.getByTestId("nav-mode-editor").click()

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

    const promptNode = page
      .locator(".react-flow__node")
      .filter({ hasText: "Extract Testable Declarations" })
      .first()
    await expect(promptNode).toBeVisible()
    await promptNode.click()

    const promptField = page.getByPlaceholder("Enter system prompt instructions...")
    await expect(promptField).toBeVisible()

    const previewRequest = page.waitForRequest(
      (request) =>
        request.url().includes("/preview") &&
        request.method() === "POST" &&
        (request.postData() || "").includes(promptToken),
    )

    await promptField.fill(promptToken)
    await previewRequest

    await expect(page.getByTestId("execute-button")).toBeEnabled()
    await expect(page.getByTestId("execute-warning-banner")).toBeVisible()
    await expect(page.getByTestId("execute-warning-banner")).toContainText("Warnings present; run allowed.")
    await page.screenshot({ path: screenshotPath("16-warning-only-execute-banner.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("diagnostics transitions toggle execute blocking and warning state for item 7.2-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-diagnostic-transition-${Date.now()}`
  const errorToken = `diagnostic-error-${Date.now()}`
  const warningToken = `diagnostic-warning-${Date.now()}`
  const cleanToken = `diagnostic-clean-${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-diagnostic-transition")

  try {
    await page.route("**/preview", async (route) => {
      const body = route.request().postData() || ""
      if (body.includes(errorToken)) {
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
      if (body.includes(warningToken)) {
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
      if (body.includes(cleanToken)) {
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

    await page.goto("/")
    await page.getByTestId("project-path-input").fill(projectPath)
    await page.getByTestId("project-register-button").click()
    await page.getByTestId("nav-mode-editor").click()

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

    const promptNode = page
      .locator(".react-flow__node")
      .filter({ hasText: "Extract Testable Declarations" })
      .first()
    await expect(promptNode).toBeVisible()
    await promptNode.click()

    const promptField = page.getByPlaceholder("Enter system prompt instructions...")
    await expect(promptField).toBeVisible()

    const waitForPreviewToken = (token: string) =>
      page.waitForRequest(
        (request) =>
          request.url().includes("/preview") &&
          request.method() === "POST" &&
          (request.postData() || "").includes(token),
      )

    const errorPreviewRequest = waitForPreviewToken(errorToken)
    await promptField.fill(errorToken)
    await errorPreviewRequest
    await expect(page.getByTestId("execute-button")).toBeDisabled()
    await expect(page.getByTestId("execute-button")).toHaveAttribute("title", "Fix validation errors before running.")
    await expect(page.getByTestId("execute-warning-banner")).toHaveCount(0)

    const warningPreviewRequest = waitForPreviewToken(warningToken)
    await promptField.fill(warningToken)
    await warningPreviewRequest
    await expect(page.getByTestId("execute-button")).toBeEnabled()
    await expect(page.getByTestId("execute-warning-banner")).toBeVisible()
    await expect(page.getByTestId("execute-warning-banner")).toContainText("Warnings present; run allowed.")
    await page.screenshot({ path: screenshotPath("17-diagnostic-transition-execute-state.png"), fullPage: true })

    const cleanPreviewRequest = waitForPreviewToken(cleanToken)
    await promptField.fill(cleanToken)
    await cleanPreviewRequest
    await expect(page.getByTestId("execute-button")).toBeEnabled()
    await expect(page.getByTestId("execute-warning-banner")).toHaveCount(0)
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("planning/build failures show diagnostics and rerun affordances for item 8.5-05", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-workflow-failure-${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-workflow-failure")

  try {
    await page.goto("/")
    await page.getByTestId("project-path-input").fill(projectPath)
    await page.getByTestId("project-register-button").click()

    await page.getByTestId("nav-mode-editor").click()
    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()
    await expect(page.getByTestId("top-nav-active-flow")).not.toContainText("No active flow")

    await page.getByTestId("nav-mode-projects").click()

    const specEntrypoint = page.getByTestId("project-spec-entrypoint")
    await specEntrypoint.getByRole("button").click()
    await page.getByTestId("project-spec-approve-for-plan-button").click()
    await expect(page.getByText("Spec status:")).toContainText("approved")

    await page.route("**/api/projects/metadata?*", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ branch: "main" }),
      })
    })
    await page.route("**/api/flows/*", async (route) => {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "forced smoke launch failure" }),
      })
    })

    await page.getByTestId("project-plan-generation-launch-button").click()
    await expect(page.getByTestId("project-plan-failure-diagnostics")).toBeVisible()
    await expect(page.getByTestId("project-plan-failure-message")).toBeVisible()
    await expect(page.getByTestId("project-plan-generation-rerun-button")).toBeEnabled()
    await page.screenshot({ path: screenshotPath("20a-plan-failure-rerun-enabled.png"), fullPage: true })

    await page.getByTestId("project-spec-edit-proposal-preview-button").click()
    await expect(page.getByTestId("project-spec-edit-proposal-preview")).toBeVisible()
    page.once("dialog", async (dialog) => {
      await dialog.accept()
    })
    await page.getByTestId("project-spec-edit-proposal-apply-button").click()
    await expect(page.getByText("Spec status:")).toContainText("draft")
    await expect(page.getByTestId("project-plan-generation-rerun-button")).toBeDisabled()
    await expect(page.getByTestId("project-plan-generation-rerun-disabled-reason")).toBeVisible()
    await page.screenshot({ path: screenshotPath("20b-plan-failure-rerun-disabled.png"), fullPage: true })

    const planEntrypoint = page.getByTestId("project-plan-entrypoint")
    await planEntrypoint.getByRole("button").click()
    await page.getByTestId("project-plan-approve-button").click()

    await page.getByTestId("execute-button").click()
    await expect(page.getByTestId("build-workflow-failure-diagnostics")).toBeVisible()
    await expect(page.getByTestId("build-workflow-failure-message")).toBeVisible()
    await expect(page.getByTestId("build-workflow-rerun-button")).toBeEnabled()
    await page.screenshot({ path: screenshotPath("20c-build-failure-rerun-enabled.png"), fullPage: true })

    await page.getByTestId("project-plan-reject-button").click()
    await expect(page.getByTestId("build-workflow-rerun-button")).toBeDisabled()
    await expect(page.getByTestId("build-workflow-rerun-disabled-reason")).toBeVisible()
    await page.screenshot({ path: screenshotPath("20d-build-failure-rerun-disabled.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})
