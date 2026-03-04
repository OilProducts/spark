import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

test.beforeAll(() => {
  ensureScreenshotDir()
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

test("run history rows include project identity and git metadata for item 9.6-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-traceability-${Date.now()}`
  const runId = `run-traceability-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TraceabilityFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "feature/traceability",
            git_commit: "fedcba9876543210",
            model: "gpt-5",
            started_at: "2026-03-03T12:00:00Z",
            ended_at: "2026-03-03T12:01:00Z",
            last_error: "",
            token_usage: 21,
          },
        ],
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-history-row-project-path").first()).toContainText(`Project: ${projectPath}`)
  await expect(page.getByTestId("run-history-row-git-branch").first()).toContainText("Branch: feature/traceability")
  await expect(page.getByTestId("run-history-row-git-commit").first()).toContainText("Commit: fedcba9876543210")
  await page.screenshot({ path: screenshotPath("08p-runs-panel-run-history-traceability.png"), fullPage: true })
})

test("run history rows link associated spec and plan artifacts when available for item 9.6-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-artifact-links-${Date.now()}`
  const runId = `run-artifact-links-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "TraceabilityLinksFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "feature/traceability-links",
            git_commit: "0123456789abcdef",
            spec_id: "spec-project-1700000000",
            plan_id: "plan-project-1700000000",
            model: "gpt-5",
            started_at: "2026-03-03T12:10:00Z",
            ended_at: "2026-03-03T12:11:00Z",
            last_error: "",
            token_usage: 19,
          },
        ],
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  await expect(page.getByTestId("run-history-row-spec-artifact-link").first()).toContainText("Spec artifact: spec-project-1700000000")
  await expect(page.getByTestId("run-history-row-plan-artifact-link").first()).toContainText("Plan artifact: plan-project-1700000000")
  await page.screenshot({ path: screenshotPath("08q-runs-panel-run-history-spec-plan-links.png"), fullPage: true })
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

test("pending human gates are discoverable in execution and runs views for item 10.1-01", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-human-gate-discoverability-${Date.now()}`
  const runId = `run-human-gate-discoverability-${Date.now()}`
  const pendingPrompt = "Approve production deploy?"

  await page.addInitScript(({ targetRunId, questionPrompt }: { targetRunId: string; questionPrompt: string }) => {
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
          emit({ type: "InterviewStarted", stage: "review_gate", index: 2, question: questionPrompt })
          emit({
            type: "human_gate",
            question_id: "gate-1",
            question_type: "MULTIPLE_CHOICE",
            node_id: "review_gate",
            prompt: questionPrompt,
            options: [
              { key: "A", label: "Approve", value: "approve", description: "Ship now to production." },
              { key: "R", label: "Request Rework", value: "rework", description: "Send build back for revision." },
            ],
          })
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
  }, { targetRunId: runId, questionPrompt: pendingPrompt })

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "HumanGateFlow",
            status: "running",
            result: "running",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "human101",
            model: "gpt-5",
            started_at: "2026-03-03T13:45:00Z",
            ended_at: null,
            last_error: "",
            token_usage: 12,
          },
        ],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: runId,
        status: "running",
        working_directory: `${projectPath}/workspace`,
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
          current_node: "review_gate",
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
          "graph.goal": "Human gate discoverability smoke",
        },
      }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  const pendingGatesPanel = page.getByTestId("run-pending-human-gates-panel")
  await expect(pendingGatesPanel).toBeVisible()
  await expect(page.getByTestId("run-pending-human-gate-item")).toContainText(pendingPrompt)
  await expect(page.getByTestId("run-pending-human-gate-option-metadata-approve")).toContainText("[A]")
  await expect(page.getByTestId("run-pending-human-gate-option-metadata-approve")).toContainText("Ship now to production.")
  await pendingGatesPanel.scrollIntoViewIfNeeded()
  await pendingGatesPanel.screenshot({ path: screenshotPath("10b-human-gate-discoverability-runs.png") })

  await page.getByRole("button", { name: "Open" }).first().click()
  await expect(page.getByTestId("execution-pending-human-gate-banner")).toBeVisible()
  await expect(page.getByTestId("execution-pending-human-gate-banner")).toContainText("Pending human gate")
  await expect(page.getByTestId("execution-pending-human-gate-banner")).toContainText(pendingPrompt)
  await page.screenshot({ path: screenshotPath("10a-human-gate-discoverability.png"), fullPage: true })
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

test("run graphviz viewer renders /pipelines/{id}/graph output for item 9.5-02", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-graphviz-${Date.now()}`
  const runId = `run-graphviz-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "GraphvizFlow",
            status: "success",
            result: "success",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "graph9502",
            model: "gpt-5",
            started_at: "2026-03-03T15:05:00Z",
            ended_at: "2026-03-03T15:06:00Z",
            last_error: "",
            token_usage: 22,
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
          "graph.goal": "Graphviz viewer smoke",
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
        artifacts: [],
      }),
    })
  })

  await page.route(`**/pipelines/${runId}/graph`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: [
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 420 120\">",
        "<rect x=\"1\" y=\"1\" width=\"418\" height=\"118\" fill=\"#ffffff\" stroke=\"#0f172a\" />",
        "<text x=\"24\" y=\"64\" font-family=\"monospace\" font-size=\"20\" fill=\"#0f172a\">Graphviz Smoke: ",
        runId,
        "</text>",
        "</svg>",
      ].join(""),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  const graphvizPanel = page.getByTestId("run-graphviz-panel")
  await expect(graphvizPanel).toBeVisible()
  await expect(page.getByTestId("run-graphviz-viewer-image")).toBeVisible()
  await expect(page.getByTestId("run-graphviz-viewer-image")).toHaveAttribute("src", /data:image\/svg\+xml/)
  await graphvizPanel.scrollIntoViewIfNeeded()
  await graphvizPanel.screenshot({ path: screenshotPath("08n-runs-panel-graphviz-viewer.png") })
})

test("run artifact browser handles missing files and partial run states for item 9.5-03", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-runs-artifacts-missing-${Date.now()}`
  const runId = `run-artifacts-missing-${Date.now()}`

  await page.route("**/runs", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        runs: [
          {
            run_id: runId,
            flow_name: "ArtifactMissingFlow",
            status: "failed",
            result: "failed",
            working_directory: `${projectPath}/workspace`,
            project_path: projectPath,
            git_branch: "main",
            git_commit: "art9503",
            model: "gpt-5",
            started_at: "2026-03-03T15:10:00Z",
            ended_at: "2026-03-03T15:11:00Z",
            last_error: "stage artifact missing",
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
          "graph.goal": "Missing artifact handling smoke",
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
            path: "plan/prompt.md",
            size_bytes: 80,
            media_type: "text/markdown",
            viewable: true,
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
    await route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ detail: "Artifact not found" }),
    })
  })

  await page.route(`**/pipelines/${runId}/graph`, async (route) => {
    await route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ detail: "Graph visualization unavailable" }),
    })
  })

  await page.goto("/")
  await page.getByTestId("project-path-input").fill(projectPath)
  await page.getByTestId("project-register-button").click()
  await page.getByTestId("nav-mode-runs").click()

  const artifactPanel = page.getByTestId("run-artifact-panel")
  await expect(artifactPanel).toBeVisible()
  await expect(page.getByTestId("run-artifact-partial-run-note")).toContainText(
    "This run may be partial or artifacts may have been pruned."
  )
  await expect(page.getByTestId("run-artifact-partial-run-note")).toContainText(
    "Missing expected files: manifest.json, checkpoint.json."
  )

  const promptRow = page.getByTestId("run-artifact-row").filter({ hasText: "plan/prompt.md" }).first()
  await promptRow.getByTestId("run-artifact-view-button").click()
  await expect(page.getByTestId("run-artifact-viewer-error")).toContainText(
    "Artifact preview unavailable because the file was not found for this run."
  )

  await artifactPanel.scrollIntoViewIfNeeded()
  await artifactPanel.screenshot({ path: screenshotPath("08o-runs-panel-artifact-missing-partial.png") })
})
