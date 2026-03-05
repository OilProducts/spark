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

test("primary UI shells render and can be navigated", async ({ page }) => {
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-shell")

  try {
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
    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

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
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("prompt edits trigger live preview diagnostics before blur for item 5.1-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-live-${Date.now()}`
  const promptToken = `live-prompt-${Date.now()}`
  const diagnosticMessage = `Live prompt diagnostic ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-live-prompt")

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

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

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
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("medium graph performance profile renders optimizations for item 13.3-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-medium-${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-medium-graph")
  const nodeCount = 30
  const nodes = Array.from({ length: nodeCount }, (_, index) => ({
    id: `node_${index}`,
    attrs: { label: `Node ${index}` },
  }))
  const edges = nodes.slice(0, -1).map((node, index) => ({
    source: node.id,
    target: nodes[index + 1].id,
    attrs: {},
  }))

  try {
    await page.route("**/preview", async (route) => {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          status: "ok",
          graph: {
            nodes,
            edges,
            graph_attrs: {},
          },
          diagnostics: [],
        }),
      })
    })

    await page.goto("/")
    await page.getByTestId("project-path-input").fill(projectPath)
    await page.getByTestId("project-register-button").click()
    await page.getByTestId("nav-mode-editor").click()

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

    const profile = page.getByTestId("canvas-performance-profile")
    await expect(profile).toContainText("medium")
    await expect(profile).toContainText("Optimizations")
    await expect(profile).toContainText("visible-only")
    await expect(profile).toContainText("debounced-preview")
    await page.screenshot({ path: screenshotPath("13e-medium-graph-performance-profile.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("validation panel supports filter and sort controls for item 7.1-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-validation-panel-${Date.now()}`
  const promptToken = `validation-panel-${Date.now()}`
  const warningLateMessage = `Validation warning late ${Date.now()}`
  const warningEarlyMessage = `Validation warning early ${Date.now()}`
  const errorMessage = `Validation error ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-validation-panel")

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
                rule_id: "warning_late",
                severity: "warning",
                message: warningLateMessage,
                line: 14,
              },
              {
                rule_id: "error_mid",
                severity: "error",
                message: errorMessage,
                line: 6,
              },
              {
                rule_id: "warning_early",
                severity: "warning",
                message: warningEarlyMessage,
                line: 2,
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
    await previewRequest

    const diagnostics = page.getByTestId("validation-diagnostic-item")
    await expect(diagnostics).toHaveCount(3)
    await expect(diagnostics.first()).toContainText(errorMessage)

    await page.getByTestId("validation-filter-warning").click()
    await expect(diagnostics).toHaveCount(2)
    await expect(diagnostics.filter({ hasText: errorMessage })).toHaveCount(0)

    await page.getByTestId("validation-sort-select").selectOption("line")
    await expect(diagnostics.first()).toContainText(warningEarlyMessage)
    await page.screenshot({ path: screenshotPath("13-validation-panel-filter-sort.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("inline node and edge diagnostic badges render for item 7.1-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-inline-badges-${Date.now()}`
  const promptToken = `inline-badges-${Date.now()}`
  const nodeDiagnosticMessage = `Node diagnostic ${Date.now()}`
  const edgeDiagnosticMessage = `Edge diagnostic ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-inline-badges")
  let nodeId: string | null = null

  try {
    await page.route("**/preview", async (route) => {
      const body = route.request().postData() || ""
      if (body.includes(promptToken) && nodeId) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "node_inline_badge",
                severity: "warning",
                message: nodeDiagnosticMessage,
                node_id: nodeId,
              },
              {
                rule_id: "edge_inline_badge",
                severity: "error",
                message: edgeDiagnosticMessage,
                edge: ["start", "ingest_spec"],
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

    await expect(page.getByRole("button", { name: "Add Node" })).toBeVisible()
    await page.getByRole("button", { name: "Add Node" }).click()

    const newNode = page.locator(".react-flow__node").filter({ hasText: "New Node" }).last()
    await expect(newNode).toBeVisible()
    await newNode.click()

    nodeId = await newNode.getAttribute("data-id")
    if (!nodeId) {
      throw new Error("Expected the newly added node to expose a data-id for inline diagnostic badge test.")
    }

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

    await expect(page.getByTestId("node-diagnostic-badge")).toContainText("1 Warn")
    await expect(page.getByTestId("edge-diagnostic-badge").first()).toContainText("1 Error")
    await page.screenshot({ path: screenshotPath("14-inline-diagnostic-badges.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("inspector field-level diagnostics map to matching fields for item 7.1-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-field-diags-${Date.now()}`
  const promptToken = `field-diags-${Date.now()}`
  const nodeDiagnosticMessage = `Prompt is required ${Date.now()}`
  const edgeDiagnosticMessage = `Condition syntax is invalid ${Date.now()}`
  const nodeFallbackDiagnosticMessage = `Fallback retry target missing ${Date.now()}`
  const edgeFidelityDiagnosticMessage = `Edge fidelity value not recognized ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-field-diags")
  let selectedNodeId: string | null = null
  const selectedEdgeSource = "audit_human_gate"
  const selectedEdgeTarget = "audit_rework"

  try {
    await page.route("**/preview", async (route) => {
      const body = route.request().postData() || ""
      if (body.includes(promptToken) && selectedNodeId) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "prompt_on_llm_nodes",
                severity: "warning",
                message: nodeDiagnosticMessage,
                node_id: selectedNodeId,
              },
              {
                rule_id: "condition_syntax",
                severity: "error",
                message: edgeDiagnosticMessage,
                edge: [selectedEdgeSource, selectedEdgeTarget],
              },
              {
                rule_id: "retry_target_exists",
                severity: "warning",
                message: `node '${selectedNodeId}' fallback_retry_target references missing node '${nodeFallbackDiagnosticMessage}'`,
                node_id: selectedNodeId,
              },
              {
                rule_id: "fidelity_valid",
                severity: "warning",
                message: `edge ${selectedEdgeSource}->${selectedEdgeTarget} fidelity '${edgeFidelityDiagnosticMessage}' is not a recognized mode`,
                edge: [selectedEdgeSource, selectedEdgeTarget],
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
    selectedNodeId = await promptNode.getAttribute("data-id")
    if (!selectedNodeId) {
      throw new Error("Expected selected node to expose data-id for field diagnostics mapping smoke test.")
    }

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

    await expect(page.getByTestId("node-field-diagnostics-prompt")).toContainText(nodeDiagnosticMessage)
    await page.getByRole("button", { name: "Show Advanced" }).click()
    await expect(page.getByTestId("node-field-diagnostics-fallback_retry_target")).toContainText(
      nodeFallbackDiagnosticMessage,
    )

    await page
      .getByRole("group", { name: "Edge from audit_human_gate to audit_rework" })
      .click({ force: true })
    await expect(page.getByTestId("edge-field-diagnostics-condition")).toContainText(edgeDiagnosticMessage)
    await expect(page.getByTestId("edge-field-diagnostics-fidelity")).toContainText(edgeFidelityDiagnosticMessage)
    await page.screenshot({ path: screenshotPath("15-inspector-field-level-diagnostics.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("validation diagnostics navigate to matching canvas entities for item 7.3-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-diagnostic-nav-${Date.now()}`
  const promptToken = `diagnostic-nav-${Date.now()}`
  const nodeDiagnosticMessage = `Node navigation diagnostic ${Date.now()}`
  const edgeDiagnosticMessage = `Edge navigation diagnostic ${Date.now()}`
  const edgeSource = "audit_human_gate"
  const edgeTarget = "audit_rework"
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-diagnostic-nav")
  let selectedNodeId: string | null = null

  try {
    await page.route("**/preview", async (route) => {
      const body = route.request().postData() || ""
      if (body.includes(promptToken) && selectedNodeId) {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            status: "ok",
            diagnostics: [
              {
                rule_id: "node_navigation",
                severity: "warning",
                message: nodeDiagnosticMessage,
                node_id: selectedNodeId,
              },
              {
                rule_id: "edge_navigation",
                severity: "error",
                message: edgeDiagnosticMessage,
                edge: [edgeSource, edgeTarget],
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
    selectedNodeId = await promptNode.getAttribute("data-id")
    if (!selectedNodeId) {
      throw new Error("Expected selected node to expose data-id for diagnostic navigation smoke test.")
    }

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

    const diagnostics = page.getByTestId("validation-diagnostic-item")
    await expect(diagnostics.filter({ hasText: nodeDiagnosticMessage })).toHaveCount(1)
    await expect(diagnostics.filter({ hasText: edgeDiagnosticMessage })).toHaveCount(1)

    await diagnostics.filter({ hasText: nodeDiagnosticMessage }).first().click()
    await expect(page.locator(".react-flow__node.selected")).toHaveCount(1)
    await expect(page.locator(`.react-flow__node[data-id="${selectedNodeId}"]`)).toHaveClass(/selected/)
    await expect(page.locator('[data-inspector-scope="node"]')).toBeVisible()

    await diagnostics.filter({ hasText: edgeDiagnosticMessage }).first().click()
    await expect(page.locator(".react-flow__edge.selected")).toHaveCount(1)
    await expect(page.locator('[data-inspector-scope="edge"]')).toBeVisible()

    await page.screenshot({ path: screenshotPath("18-diagnostic-navigation-to-canvas.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("stylesheet parse diagnostics render in graph settings for item 6.5-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-stylesheet-${Date.now()}`
  const stylesheetToken = ".bad$class { llm_model: gpt-5; }"
  const diagnosticMessage = `Stylesheet syntax diagnostic ${Date.now()}`
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-stylesheet-diagnostics")

  try {
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

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

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
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})

test("stylesheet selector/effective previews render in graph settings for item 6.5-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-stylesheet-preview-${Date.now()}`
  const stylesheetToken = "* { llm_provider: openai; } .critical { llm_model: gpt-5.2; }"
  const flowName = await cloneFlowForSmokeTest(page, "ui-smoke-stylesheet-preview")

  try {
    await page.goto("/")
    await page.getByTestId("project-path-input").fill(projectPath)
    await page.getByTestId("project-register-button").click()
    await page.getByTestId("nav-mode-editor").click()

    const flowButton = page.getByRole("button", { name: flowName })
    await expect(flowButton).toBeVisible()
    await flowButton.click()

    await expect(page.locator('[data-inspector-scope="graph"]')).toBeVisible()
    const advancedToggle = page.getByTestId("graph-advanced-toggle")
    await expect(advancedToggle).toBeVisible()
    await advancedToggle.click()

    const stylesheetInput = page.getByTestId("model-stylesheet-editor").locator("textarea")
    await stylesheetInput.fill(stylesheetToken)

    await expect(page.getByTestId("graph-model-stylesheet-selector-preview")).toBeVisible()
    await expect(page.getByTestId("graph-model-stylesheet-effective-preview")).toBeVisible()
    await expect(page.getByTestId("graph-model-stylesheet-precedence-guidance")).toBeVisible()
    await expect(page.getByTestId("graph-model-stylesheet-selector-preview")).toContainText(".critical")
    await expect(page.getByTestId("graph-model-stylesheet-effective-preview")).toContainText("(stylesheet)")
    await expect(page.getByTestId("graph-model-stylesheet-effective-preview")).toContainText("(graph default)")
    await page
      .getByTestId("graph-model-stylesheet-effective-preview")
      .screenshot({ path: screenshotPath("12-stylesheet-precedence-rendering.png") })
    await page.screenshot({ path: screenshotPath("11-stylesheet-selector-effective-preview.png"), fullPage: true })
  } finally {
    await deleteFlowAfterSmoke(page, flowName)
  }
})
