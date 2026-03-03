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

test("no-op semantic-equivalence save paths send guarded requests for item 5.3-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-semantic-equivalence-${Date.now()}`
  const semanticSaveBodies: string[] = []

  await page.route("**/api/flows", async (route) => {
    const request = route.request()
    if (request.method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(["semantic.dot"]),
      })
      return
    }

    if (request.method() === "POST") {
      const body = request.postData() || ""
      if (body.includes('"expect_semantic_equivalence":true')) {
        semanticSaveBodies.push(body)
      }
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", name: "semantic.dot" }),
      })
      return
    }

    await route.continue()
  })

  await page.route("**/api/flows/semantic.dot", async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue()
      return
    }

    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        name: "semantic.dot",
        content: [
          "digraph semantic {",
          '  start [shape=Mdiamond, label=label="Start"];',
          '  done [shape=Msquare, label="Done"];',
          "  start -> done;",
          "}",
        ].join("\n"),
      }),
    })
  })

  await page.route("**/preview", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "ok",
        graph: {
          nodes: [
            { id: "start", shape: "Mdiamond", label: "Start" },
            { id: "done", shape: "Msquare", label: "Done" },
          ],
          edges: [{ from: "start", to: "done" }],
        },
        diagnostics: [],
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-editor").click()

  const flowButton = page.getByRole("button", { name: "semantic.dot" })
  await expect(flowButton).toBeVisible()
  await flowButton.click()
  await expect(page.getByTestId("canvas-workspace-primary")).toBeVisible()
  await expect.poll(() => semanticSaveBodies.length).toBeGreaterThanOrEqual(1)

  await page.getByRole("button", { name: "Raw DOT" }).click()
  await expect(page.getByTestId("raw-dot-editor")).toBeVisible()
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(page.getByRole("button", { name: "Add Node" })).toBeVisible()
  await expect.poll(() => semanticSaveBodies.length).toBeGreaterThanOrEqual(2)
  await page.screenshot({ path: screenshotPath("19-semantic-equivalence-noop-save.png"), fullPage: true })
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

test("validation panel supports filter and sort controls for item 7.1-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-validation-panel-${Date.now()}`
  const promptToken = `validation-panel-${Date.now()}`
  const warningLateMessage = `Validation warning late ${Date.now()}`
  const warningEarlyMessage = `Validation warning early ${Date.now()}`
  const errorMessage = `Validation error ${Date.now()}`

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
})

test("inline node and edge diagnostic badges render for item 7.1-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-inline-badges-${Date.now()}`
  const promptToken = `inline-badges-${Date.now()}`
  const nodeDiagnosticMessage = `Node diagnostic ${Date.now()}`
  const edgeDiagnosticMessage = `Edge diagnostic ${Date.now()}`
  let nodeId: string | null = null

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

  const flowButton = page.getByRole("button", { name: "implement-spec.dot" })
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
})

test("inspector field-level diagnostics map to matching fields for item 7.1-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-field-diags-${Date.now()}`
  const promptToken = `field-diags-${Date.now()}`
  const nodeDiagnosticMessage = `Prompt is required ${Date.now()}`
  const edgeDiagnosticMessage = `Condition syntax is invalid ${Date.now()}`
  const nodeFallbackDiagnosticMessage = `Fallback retry target missing ${Date.now()}`
  const edgeFidelityDiagnosticMessage = `Edge fidelity value not recognized ${Date.now()}`
  let selectedNodeId: string | null = null
  const selectedEdgeSource = "audit_human_gate"
  const selectedEdgeTarget = "audit_rework"

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

  const flowButton = page.getByRole("button", { name: "implement-spec.dot" })
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
})

test("validation diagnostics navigate to matching canvas entities for item 7.3-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-diagnostic-nav-${Date.now()}`
  const promptToken = `diagnostic-nav-${Date.now()}`
  const nodeDiagnosticMessage = `Node navigation diagnostic ${Date.now()}`
  const edgeDiagnosticMessage = `Edge navigation diagnostic ${Date.now()}`
  const edgeSource = "audit_human_gate"
  const edgeTarget = "audit_rework"
  let selectedNodeId: string | null = null

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

  const flowButton = page.getByRole("button", { name: "implement-spec.dot" })
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
})

test("warning-only diagnostics still allow execute with explicit banner for item 7.2-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-warning-only-${Date.now()}`
  const promptToken = `warning-only-${Date.now()}`
  const warningMessage = `Warning-only diagnostic ${Date.now()}`

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

  const flowButton = page.getByRole("button", { name: "implement-spec.dot" })
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
})

test("diagnostics transitions toggle execute blocking and warning state for item 7.2-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-diagnostic-transition-${Date.now()}`
  const errorToken = `diagnostic-error-${Date.now()}`
  const warningToken = `diagnostic-warning-${Date.now()}`
  const cleanToken = `diagnostic-clean-${Date.now()}`

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

  const flowButton = page.getByRole("button", { name: "implement-spec.dot" })
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

test("stylesheet selector/effective previews render in graph settings for item 6.5-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-stylesheet-preview-${Date.now()}`
  const stylesheetToken = "* { llm_provider: openai; } .critical { llm_model: gpt-5.2; }"

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
})
