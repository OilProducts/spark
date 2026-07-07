import { expect, test } from '@playwright/test'
import {
  ensureScreenshotDir,
  gotoWithRegisteredProject,
  screenshotPath,
  stubProjectMetadata,
} from '../fixtures/smoke-helpers'

test.beforeAll(() => {
  ensureScreenshotDir()
})

test.beforeEach(async ({ page }) => {
  await stubProjectMetadata(page)
})

test("raw YAML save blocks parse errors and hydrates valid handoff", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-raw-yaml-${Date.now()}`
  const parseErrorBodies: string[] = []
  const savedBodies: string[] = []
  const flowName = "semantic.yaml"
  const flowYaml = `schema_version: "1"
id: semantic
title: Semantic YAML
nodes:
  start:
    kind: start
    label: Start
    config:
      kind: start
  done:
    kind: exit
    label: Done
    config:
      kind: exit
edges:
  - from: start
    to: done
`

  await page.route("**/attractor/api/flows", async (route) => {
    const request = route.request()
    if (request.method() === "GET") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([flowName]),
      })
      return
    }

    if (request.method() === "POST") {
      const body = request.postData() || ""
      let payload: { content?: string } = {}
      try {
        payload = JSON.parse(body) as { content?: string }
      } catch {
        payload = {}
      }
      if (payload.content?.includes("nodes: [")) {
        parseErrorBodies.push(body)
        await route.fulfill({
          status: 400,
          contentType: "application/json",
          body: '{"detail":{"status":"parse_error","error":"invalid YAML sequence"}}',
        })
        return
      }
      savedBodies.push(body)
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ status: "saved", name: flowName }),
      })
      return
    }

    await route.continue()
  })

  await page.route(`**/attractor/api/flows/${flowName}`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.continue()
      return
    }

    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        name: flowName,
        content: flowYaml,
      }),
    })
  })

  await page.route("**/attractor/preview", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        status: "ok",
        flow: {
          schema_version: "1",
          id: "semantic",
          title: "Semantic YAML",
          nodes: {
            start: { kind: "start", label: "Start", config: { kind: "start" } },
            done: { kind: "exit", label: "Done", config: { kind: "exit" } },
          },
          edges: [{ from: "start", to: "done" }],
        },
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

  await gotoWithRegisteredProject(page, projectPath)
  await page.getByTestId("nav-mode-editor").click()

  const flowButton = page.getByRole("button", { name: flowName })
  await expect(flowButton).toBeVisible()
  await flowButton.click()
  await expect(page.getByTestId("canvas-workspace-primary")).toBeVisible()

  await page.getByRole("button", { name: "Raw YAML" }).click()
  const rawYamlEditor = page.getByTestId("raw-yaml-editor")
  await expect(rawYamlEditor).toBeVisible()
  const rawYamlEntry = await rawYamlEditor.inputValue()
  const invalidYaml = `${rawYamlEntry}\nnodes: [`
  const equivalentYaml = `${rawYamlEntry}\nmetadata:\n  smoke_round_trip: true\n`
  await rawYamlEditor.fill(invalidYaml)
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(rawYamlEditor).toBeVisible()
  const rawHandoffError = page.getByTestId("raw-yaml-handoff-error")
  if ((await rawHandoffError.count()) > 0) {
    await expect(rawHandoffError).toContainText("Safe handoff requires valid YAML.")
  }
  await expect(page.getByRole("button", { name: "Add Node" })).toHaveCount(0)
  await expect.poll(() => parseErrorBodies.length).toBeGreaterThanOrEqual(1)
  await page.screenshot({ path: screenshotPath("19a-raw-yaml-parse-error-blocked.png"), fullPage: true })

  const savedBeforeRoundTrip = savedBodies.length
  await rawYamlEditor.fill(equivalentYaml)
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(page.getByRole("button", { name: "Add Node" })).toBeVisible()
  await expect.poll(() => savedBodies.length).toBeGreaterThan(savedBeforeRoundTrip)
  await page.screenshot({ path: screenshotPath("19b-raw-yaml-round-trip-saved.png"), fullPage: true })
})
