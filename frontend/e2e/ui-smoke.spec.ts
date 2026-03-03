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

test("run summary panel renders populated metadata for item 9.1-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-summary-${Date.now()}`
  const runId = `run-summary-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "SmokeFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "abc1234",
            model: "gpt-5",
            started_at: "2026-03-03T12:00:00Z",
            ended_at: "2026-03-03T12:02:00Z",
            last_error: "none",
            token_usage: 42,
          },
        ],
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-summary-panel")).toBeVisible()
  await expect(page.getByTestId("run-summary-panel")).toContainText(runId)
  await expect(page.getByTestId("run-summary-status")).toContainText("success")
  await expect(page.getByTestId("run-summary-result")).toContainText("success")
  await expect(page.getByTestId("run-summary-flow-name")).toContainText("SmokeFlow")
  await expect(page.getByTestId("run-summary-model")).toContainText("gpt-5")
  await expect(page.getByTestId("run-summary-working-directory")).toContainText(`${projectPath}/workspace`)
  await expect(page.getByTestId("run-summary-project-path")).toContainText(projectPath)
  await expect(page.getByTestId("run-summary-git-branch")).toContainText("main")
  await expect(page.getByTestId("run-summary-git-commit")).toContainText("abc1234")
  await expect(page.getByTestId("run-summary-last-error")).toContainText("none")
  await expect(page.getByTestId("run-summary-token-usage")).toContainText("42")
  await page.screenshot({ path: screenshotPath("08b-runs-panel-populated-summary.png"), fullPage: true })
})

test("run summary metadata refresh and stale-state indicator for item 9.1-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-refresh-${Date.now()}`
  const runId = `run-refresh-${Date.now()}`
  let refreshCount = 0

  await page.addInitScript(() => {
    ;(globalThis as typeof globalThis & { __RUNS_METADATA_STALE_AFTER_MS__?: number }).__RUNS_METADATA_STALE_AFTER_MS__ = 250
  })

  await page.route("**/runs", async (route) => {
    refreshCount += 1
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "RefreshFlow",
            status: refreshCount >= 2 ? "success" : "running",
            result: refreshCount >= 2 ? "success" : "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "def5678",
            model: "gpt-5",
            started_at: "2026-03-03T12:00:00Z",
            ended_at: refreshCount >= 2 ? "2026-03-03T12:00:10Z" : null,
            last_error: "none",
            token_usage: refreshCount >= 2 ? 108 : 7,
          },
        ],
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-summary-panel")).toBeVisible()
  await expect(page.getByTestId("run-summary-token-usage")).toContainText("7")
  await expect(page.getByTestId("run-metadata-freshness-indicator")).toContainText("Fresh")
  await expect(page.getByTestId("run-metadata-last-updated")).toContainText("Updated")

  await page.waitForTimeout(1300)
  await expect(page.getByTestId("run-metadata-freshness-indicator")).toContainText("Stale")
  await expect(page.getByTestId("run-metadata-stale-indicator")).toContainText(
    "Run metadata may be stale. Refresh to load the latest run status."
  )

  await page.getByTestId("runs-refresh-button").click()
  await expect(page.getByTestId("run-summary-token-usage")).toContainText("108")
  await expect(page.getByTestId("run-metadata-freshness-indicator")).toContainText("Fresh")
  await expect(page.getByTestId("run-metadata-stale-indicator")).toHaveCount(0)
  await page.screenshot({ path: screenshotPath("08c-runs-panel-refresh-stale-indicator.png"), fullPage: true })
})

test("run checkpoint viewer fetches checkpoint payload for item 9.2-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-checkpoint-${Date.now()}`
  const runId = `run-checkpoint-${Date.now()}`
  let checkpointFetchCount = 0

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "CheckpointFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "cafe123",
            model: "gpt-5",
            started_at: "2026-03-03T12:00:00Z",
            ended_at: "2026-03-03T12:02:00Z",
            last_error: "",
            token_usage: 12,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    checkpointFetchCount += 1
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "implement",
          completed_nodes: ["start", "plan"],
          retry_counts: { implement: 1 },
          timestamp: "2026-03-03T12:01:30Z",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-checkpoint-panel")).toBeVisible()
  await expect(page.getByTestId("run-checkpoint-payload")).toContainText("\"pipeline_id\":")
  await expect(page.getByTestId("run-checkpoint-payload")).toContainText("\"current_node\": \"implement\"")
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(1)

  await page.getByTestId("run-checkpoint-refresh-button").click()
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(2)
  await page.screenshot({ path: screenshotPath("08d-runs-panel-checkpoint-viewer.png"), fullPage: true })
})

test("run checkpoint viewer handles unavailable checkpoint for item 9.2-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-checkpoint-unavailable-${Date.now()}`
  const runId = `run-checkpoint-unavailable-${Date.now()}`
  let checkpointFetchCount = 0

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "CheckpointMissingFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "bead456",
            model: "gpt-5",
            started_at: "2026-03-03T12:05:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 3,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    checkpointFetchCount += 1
    await route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ detail: "Checkpoint unavailable" }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-checkpoint-panel")).toBeVisible()
  await expect(page.getByTestId("run-checkpoint-error")).toContainText("Checkpoint unavailable for this run.")
  await expect(page.getByTestId("run-checkpoint-error-help")).toContainText(
    "Run may still be in progress or did not persist checkpoint data yet."
  )
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(1)

  await page.getByTestId("run-checkpoint-refresh-button").click()
  await expect.poll(() => checkpointFetchCount).toBeGreaterThanOrEqual(2)
  await page.screenshot({ path: screenshotPath("08e-runs-panel-checkpoint-unavailable.png"), fullPage: true })
})

test("run context viewer supports searchable key/value inspection for item 9.3-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-context-${Date.now()}`
  const runId = `run-context-${Date.now()}`
  let contextFetchCount = 0

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "ContextFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "feed999",
            model: "gpt-5",
            started_at: "2026-03-03T12:10:00Z",
            ended_at: "2026-03-03T12:11:00Z",
            last_error: "",
            token_usage: 24,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "inspect",
          completed_nodes: ["start", "plan", "build"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    contextFetchCount += 1
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Ship context panel",
          "context.plan.ready": true,
          "context.retries": 2,
          summary: {
            stage: "inspect",
            owner: "ops",
          },
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-context-panel")).toBeVisible()
  await expect(page.getByTestId("run-context-table")).toBeVisible()
  await expect(page.getByTestId("run-context-row")).toHaveCount(4)
  await expect(page.getByTestId("run-context-row-type")).toHaveCount(4)
  await expect(page.getByTestId("run-context-table")).toContainText("boolean")
  await expect(page.getByTestId("run-context-table")).toContainText("number")
  await expect(page.getByTestId("run-context-table")).toContainText("object")
  await expect(page.getByTestId("run-context-table")).toContainText("true")
  await expect(page.getByTestId("run-context-table")).toContainText("2")
  await expect.poll(() => contextFetchCount).toBeGreaterThanOrEqual(1)

  await page.getByTestId("run-context-search-input").fill("graph.goal")
  await expect(page.getByTestId("run-context-row")).toHaveCount(1)
  await expect(page.getByTestId("run-context-row-type").first()).toContainText("string")
  await expect(page.getByTestId("run-context-row-value").first()).toContainText("Ship context panel")

  await page.getByTestId("run-context-search-input").fill("ops")
  await expect(page.getByTestId("run-context-row")).toHaveCount(1)
  await expect(page.getByTestId("run-context-row-type").first()).toContainText("object")
  await expect(page.getByTestId("run-context-row-value").first()).toContainText("\"owner\": \"ops\"")

  await page.getByTestId("run-context-search-input").fill("missing-key")
  await expect(page.getByTestId("run-context-empty")).toContainText("No context entries match the current search.")
  await page.screenshot({ path: screenshotPath("08f-runs-panel-context-viewer.png"), fullPage: true })
})

test("run context viewer renders typed scalar and structured values for item 9.3-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-context-typed-${Date.now()}`
  const runId = `run-context-typed-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "ContextTypedFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "type321",
            model: "gpt-5",
            started_at: "2026-03-03T12:20:00Z",
            ended_at: "2026-03-03T12:21:00Z",
            last_error: "",
            token_usage: 28,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "typed_render",
          completed_nodes: ["start"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          plain_string: "review notes",
          build_number: 17,
          approved: false,
          summary: {
            stage: "build",
            retries: 1,
          },
          tags: ["ui", "typed"],
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-context-panel")).toBeVisible()
  await expect(page.getByTestId("run-context-row")).toHaveCount(5)
  await expect(page.getByTestId("run-context-table")).toContainText("string")
  await expect(page.getByTestId("run-context-table")).toContainText("number")
  await expect(page.getByTestId("run-context-table")).toContainText("boolean")
  await expect(page.getByTestId("run-context-table")).toContainText("object")
  await expect(page.getByTestId("run-context-table")).toContainText("array")
  await expect(page.getByTestId("run-context-row-value-scalar")).toHaveCount(3)
  await expect(page.getByTestId("run-context-row-value-structured")).toHaveCount(2)
  await expect(page.getByTestId("run-context-table")).toContainText('"review notes"')
  await expect(page.getByTestId("run-context-table")).toContainText("17")
  await expect(page.getByTestId("run-context-table")).toContainText("false")
  await expect(page.getByTestId("run-context-table")).toContainText("\"stage\": \"build\"")
  await expect(page.getByTestId("run-context-table")).toContainText("\"ui\"")
  await page.screenshot({ path: screenshotPath("08g-runs-panel-context-typed-rendering.png"), fullPage: true })
})

test("run context viewer exposes copy/export actions for item 9.3-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-context-copy-export-${Date.now()}`
  const runId = `run-context-copy-export-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "ContextCopyExportFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "copy777",
            model: "gpt-5",
            started_at: "2026-03-03T12:30:00Z",
            ended_at: "2026-03-03T12:31:00Z",
            last_error: "",
            token_usage: 31,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "context_export",
          completed_nodes: ["start", "inspect"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Ship copy export",
          owner: "reviewer",
          retries: 1,
        },
      }),
    })
  })

  await page.goto("/")
  await page.evaluate(() => {
    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          ;(globalThis as typeof globalThis & { __copied_context_payload__?: string }).__copied_context_payload__ = value
        },
      },
    })
  })
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-context-panel")).toBeVisible()
  await expect(page.getByTestId("run-context-copy-button")).toBeVisible()
  await expect(page.getByTestId("run-context-export-button")).toBeVisible()

  await page.getByTestId("run-context-copy-button").click()
  await expect(page.getByTestId("run-context-copy-status")).toContainText("Filtered context copied.")
  await expect
    .poll(() => page.evaluate(() => (globalThis as typeof globalThis & { __copied_context_payload__?: string }).__copied_context_payload__ || ""))
    .toContain(`"pipeline_id": "${runId}"`)

  const downloadPromise = page.waitForEvent("download")
  await page.getByTestId("run-context-export-button").click()
  const download = await downloadPromise
  await expect(download.suggestedFilename()).toBe(`run-context-${runId}.json`)
  await page.screenshot({ path: screenshotPath("08h-runs-panel-context-copy-export.png"), fullPage: true })
})

test("run event timeline renders typed lifecycle and runtime events for item 9.4-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-event-timeline-${Date.now()}`
  const runId = `run-event-timeline-${Date.now()}`

  await page.addInitScript((targetRunId: string) => {
    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        const expectedPath = `/pipelines/${encodeURIComponent(targetRunId)}/events`
        if (!url.includes(expectedPath)) {
          return
        }

        const emit = (payload: Record<string, unknown>) => {
          this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }))
        }

        setTimeout(() => {
          this.onopen?.(new Event("open"))
          emit({ type: "PipelineStarted", current_node: "start" })
          emit({ type: "StageStarted", node_id: "plan", index: 1 })
          emit({ type: "ParallelStarted", branch_count: 2 })
          emit({ type: "InterviewStarted", stage: "review", question: "Approve?" })
        }, 0)

        setTimeout(() => {
          emit({ type: "CheckpointSaved", node_id: "review", persisted: true })
        }, 450)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    ;(window as typeof window & { EventSource: typeof EventSource }).EventSource = MockEventSource as unknown as typeof EventSource
  }, runId)

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TimelineFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "time901",
            model: "gpt-5",
            started_at: "2026-03-03T13:00:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 9,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "review",
          completed_nodes: ["start", "plan"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Timeline smoke",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-event-timeline-panel")).toBeVisible()
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(4)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("PipelineStarted")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("StageStarted")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("ParallelStarted")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("InterviewStarted")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(5)
  await expect(page.getByTestId("run-event-timeline-row-category")).toHaveCount(5)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("CheckpointSaved")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Lifecycle")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Stage")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Parallel")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Interview")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Checkpoint")
  const timelinePanel = page.getByTestId("run-event-timeline-panel")
  await timelinePanel.scrollIntoViewIfNeeded()
  await timelinePanel.screenshot({ path: screenshotPath("08i-runs-panel-event-timeline.png") })
})

test("run event timeline filtering supports type, node/stage, category, and severity for item 9.4-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-event-timeline-filters-${Date.now()}`
  const runId = `run-event-timeline-filters-${Date.now()}`

  await page.addInitScript((targetRunId: string) => {
    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        const expectedPath = `/pipelines/${encodeURIComponent(targetRunId)}/events`
        if (!url.includes(expectedPath)) {
          return
        }

        const emit = (payload: Record<string, unknown>) => {
          this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }))
        }

        setTimeout(() => {
          this.onopen?.(new Event("open"))
          emit({ type: "PipelineStarted", current_node: "start" })
          emit({ type: "StageStarted", node_id: "plan", index: 1 })
          emit({ type: "StageFailed", node_id: "review", index: 2, error: "validation failed" })
          emit({ type: "StageRetrying", node_id: "review", index: 2, attempt: 1 })
          emit({ type: "InterviewTimeout", stage: "review", duration: 15 })
          emit({ type: "CheckpointSaved", node_id: "review", persisted: true })
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    ;(window as typeof window & { EventSource: typeof EventSource }).EventSource = MockEventSource as unknown as typeof EventSource
  }, runId)

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TimelineFilterFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "time902",
            model: "gpt-5",
            started_at: "2026-03-03T13:15:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 11,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "review",
          completed_nodes: ["start", "plan"],
          retry_counts: { review: 1 },
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Timeline filter smoke",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-event-timeline-panel")).toBeVisible()
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(6)

  await page.getByTestId("run-event-timeline-filter-type").selectOption("StageFailed")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(1)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("StageFailed")

  await page.getByTestId("run-event-timeline-filter-type").selectOption("all")
  await page.getByTestId("run-event-timeline-filter-node-stage").fill("review")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(4)
  await expect(page.getByTestId("run-event-timeline-list")).not.toContainText("PipelineStarted")

  await page.getByTestId("run-event-timeline-filter-node-stage").fill("")
  await page.getByTestId("run-event-timeline-filter-category").selectOption("checkpoint")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(1)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("CheckpointSaved")

  await page.getByTestId("run-event-timeline-filter-category").selectOption("all")
  await page.getByTestId("run-event-timeline-filter-severity").selectOption("warning")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(2)
  await expect(page.getByTestId("run-event-timeline-row-severity")).toHaveCount(2)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("StageRetrying")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("InterviewTimeout")
  const timelinePanel = page.getByTestId("run-event-timeline-panel")
  await timelinePanel.scrollIntoViewIfNeeded()
  await timelinePanel.screenshot({ path: screenshotPath("08j-runs-panel-event-timeline-filters.png") })
})

test("run event timeline groups and correlates retry and interview sequences for item 9.4-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-event-timeline-correlation-${Date.now()}`
  const runId = `run-event-timeline-correlation-${Date.now()}`

  await page.addInitScript((targetRunId: string) => {
    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        const expectedPath = `/pipelines/${encodeURIComponent(targetRunId)}/events`
        if (!url.includes(expectedPath)) {
          return
        }

        const emit = (payload: Record<string, unknown>) => {
          this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }))
        }

        setTimeout(() => {
          this.onopen?.(new Event("open"))
          emit({ type: "StageStarted", node_id: "plan", index: 1 })
          emit({ type: "StageFailed", node_id: "plan", index: 1, error: "retry required" })
          emit({ type: "StageRetrying", node_id: "plan", index: 1, attempt: 1 })
          emit({ type: "StageCompleted", node_id: "plan", index: 1, outcome: "success" })
          emit({ type: "InterviewStarted", stage: "review", index: 2, question: "Approve?" })
          emit({ type: "InterviewTimeout", stage: "review", index: 2 })
          emit({ type: "InterviewCompleted", stage: "review", index: 2, answer: "yes" })
        }, 0)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    ;(window as typeof window & { EventSource: typeof EventSource }).EventSource = MockEventSource as unknown as typeof EventSource
  }, runId)

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TimelineCorrelationFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "time903",
            model: "gpt-5",
            started_at: "2026-03-03T13:30:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 13,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "review",
          completed_nodes: ["start", "plan"],
          retry_counts: { plan: 1 },
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Timeline correlation smoke",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-event-timeline-panel")).toBeVisible()
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(7)
  await expect(page.getByTestId("run-event-timeline-group")).toHaveCount(2)
  await expect(page.getByTestId("run-event-timeline-group-label")).toHaveCount(2)
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Retry sequence for plan (index 1)")
  await expect(page.getByTestId("run-event-timeline-list")).toContainText("Interview sequence for review (index 2)")
  await expect(page.getByTestId("run-event-timeline-row-correlation")).toHaveCount(7)
  const timelinePanel = page.getByTestId("run-event-timeline-panel")
  await timelinePanel.scrollIntoViewIfNeeded()
  await timelinePanel.screenshot({ path: screenshotPath("08k-runs-panel-event-timeline-grouping-correlation.png") })
})

test("run event timeline replays stream history and appends live events for item 9.4-04", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-event-timeline-replay-live-${Date.now()}`
  const runId = `run-event-timeline-replay-live-${Date.now()}`

  await page.addInitScript((targetRunId: string) => {
    class MockEventSource {
      url: string
      withCredentials = false
      readyState = 1
      onopen: ((event: Event) => void) | null = null
      onmessage: ((event: MessageEvent) => void) | null = null
      onerror: ((event: Event) => void) | null = null

      constructor(url: string) {
        this.url = url
        const expectedPath = `/pipelines/${encodeURIComponent(targetRunId)}/events`
        if (!url.includes(expectedPath)) {
          return
        }

        const emit = (payload: Record<string, unknown>) => {
          this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(payload) }))
        }

        setTimeout(() => {
          this.onopen?.(new Event("open"))
          // Simulate replayed history emitted on initial connection.
          emit({ type: "PipelineStarted", current_node: "start" })
          emit({ type: "StageStarted", node_id: "plan", index: 1 })
          emit({ type: "StageCompleted", node_id: "plan", index: 1, outcome: "ok" })
        }, 0)

        // Simulate a live append that arrives after replay completion.
        setTimeout(() => {
          emit({ type: "PipelineCompleted", result: "success" })
        }, 500)
      }

      close() {
        this.readyState = 2
      }

      addEventListener() {}

      removeEventListener() {}

      dispatchEvent() {
        return false
      }
    }

    ;(window as typeof window & { EventSource: typeof EventSource }).EventSource = MockEventSource as unknown as typeof EventSource
  }, runId)

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TimelineReplayFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "time904",
            model: "gpt-5",
            started_at: "2026-03-03T14:00:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 17,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "plan",
          completed_nodes: ["start"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Timeline replay smoke",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-event-timeline-panel")).toBeVisible()
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(3)
  await expect(page.getByTestId("run-event-timeline-row-type").first()).toContainText("StageCompleted")
  await expect(page.getByTestId("run-event-timeline-row-type")).toHaveCount(4)
  await expect(page.getByTestId("run-event-timeline-row-type").first()).toContainText("PipelineCompleted")
  const timelinePanel = page.getByTestId("run-event-timeline-panel")
  await timelinePanel.scrollIntoViewIfNeeded()
  await timelinePanel.screenshot({ path: screenshotPath("08l-runs-panel-event-timeline-replay-live-append.png") })
})

test("run artifact browser lists run outputs and supports view/download for item 9.5-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-artifacts-${Date.now()}`
  const runId = `run-artifacts-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "ArtifactFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "art9501",
            model: "gpt-5",
            started_at: "2026-03-03T15:00:00Z",
            ended_at: "2026-03-03T15:01:00Z",
            last_error: "",
            token_usage: 18,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/checkpoint`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        checkpoint: {
          current_node: "done",
          completed_nodes: ["start", "plan"],
          retry_counts: {},
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/context`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        context: {
          "graph.goal": "Artifact browser smoke",
        },
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/artifacts`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        pipeline_id: runId,
        artifacts: [
          {
            path: "manifest.json",
            size_bytes: 120,
            media_type: "application/json",
            viewable: true,
          },
          {
            path: "plan/prompt.md",
            size_bytes: 80,
            media_type: "text/markdown",
            viewable: true,
          },
          {
            path: "artifacts/tool_output.bin",
            size_bytes: 1024,
            media_type: "application/octet-stream",
            viewable: false,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/artifacts/**`, async (route) => {
    const url = new URL(route.request().url())
    if (url.pathname.endsWith(`/pipelines/${runId}/artifacts`)) {
      await route.fallback()
      return
    }
    if (url.pathname.endsWith("/manifest.json")) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ graph_id: "artifact-flow", started_at: "2026-03-03T15:00:00Z" }, null, 2),
      })
      return
    }
    if (url.pathname.endsWith("/prompt.md")) {
      await route.fulfill({
        status: 200,
        contentType: "text/markdown",
        body: "# Prompt\n\nDo the work.",
      })
      return
    }
    await route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ detail: "Artifact not found" }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-artifact-panel")).toBeVisible()
  await expect(page.getByTestId("run-artifact-row")).toHaveCount(3)
  const manifestRow = page.getByTestId("run-artifact-row").filter({ hasText: "manifest.json" }).first()
  await manifestRow.getByTestId("run-artifact-view-button").click()
  await expect(page.getByTestId("run-artifact-viewer-payload")).toContainText("\"graph_id\": \"artifact-flow\"")
  await expect(manifestRow.getByTestId("run-artifact-download-link")).toHaveAttribute("href", /download=1/)

  const artifactPanel = page.getByTestId("run-artifact-panel")
  await artifactPanel.scrollIntoViewIfNeeded()
  await artifactPanel.screenshot({ path: screenshotPath("08m-runs-panel-artifact-browser.png") })
})

test("semantic-equivalence save blocks mismatch and confirms no-op round-trip for item 5.3-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-semantic-equivalence-${Date.now()}`
  const semanticSaveBodies: string[] = []
  const semanticMismatchBodies: string[] = []
  const semanticEquivalentSavedBodies: string[] = []
  let mismatchInjected = false
  let mismatchTargetContent: string | null = null

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
      const hasSemanticEquivalenceGuard = body.includes('"expect_semantic_equivalence":true')
      let payload: { expect_semantic_equivalence?: boolean; content?: string } = {}
      try {
        payload = JSON.parse(body) as { expect_semantic_equivalence?: boolean; content?: string }
      } catch {
        payload = {}
      }

      if (payload.expect_semantic_equivalence === true || hasSemanticEquivalenceGuard) {
        semanticSaveBodies.push(body)
        if (
          mismatchTargetContent !== null
          && mismatchInjected === false
          && payload.content === mismatchTargetContent
        ) {
          mismatchInjected = true
          semanticMismatchBodies.push(body)
          await route.fulfill({
            status: 409,
            contentType: "application/json",
            body: '{"detail":{"status":"semantic_mismatch","error":"semantic equivalence check failed: output DOT would change flow behavior"}}',
          })
          return
        }
        semanticEquivalentSavedBodies.push(body)
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
  const rawDotEditor = page.getByTestId("raw-dot-editor")
  await expect(rawDotEditor).toBeVisible()
  const rawDotEntry = await rawDotEditor.inputValue()
  mismatchTargetContent = rawDotEntry
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(rawDotEditor).toBeVisible()
  const rawHandoffError = page.getByTestId("raw-dot-handoff-error")
  if ((await rawHandoffError.count()) > 0) {
    await expect(rawHandoffError).toContainText("Safe handoff requires valid DOT.")
  }
  await expect(page.getByRole("button", { name: "Add Node" })).toHaveCount(0)
  await expect.poll(() => semanticMismatchBodies.length).toBeGreaterThanOrEqual(1)
  await page.screenshot({ path: screenshotPath("19a-semantic-equivalence-mismatch-blocked.png"), fullPage: true })

  const equivalentSavesBeforeRoundTrip = semanticEquivalentSavedBodies.length
  await rawDotEditor.fill(rawDotEntry)
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(page.getByRole("button", { name: "Add Node" })).toBeVisible()
  await expect.poll(() => semanticEquivalentSavedBodies.length).toBeGreaterThan(equivalentSavesBeforeRoundTrip)
  await page.screenshot({ path: screenshotPath("19b-semantic-equivalence-round-trip-saved.png"), fullPage: true })
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

test("planning/build failures show diagnostics and rerun affordances for item 8.5-05", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-workflow-failure-${Date.now()}`

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()

  await page.getByTestId("nav-mode-editor").click()
  const firstFlowButton = page.locator("button").filter({ hasText: ".dot" }).first()
  await expect(firstFlowButton).toBeVisible()
  await firstFlowButton.click()
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
})
