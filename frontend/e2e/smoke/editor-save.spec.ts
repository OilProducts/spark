import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'
import {
  ensureScreenshotDir,
  gotoWithRegisteredProject,
  screenshotPath,
  stubProjectMetadata,
} from '../fixtures/smoke-helpers'

// The smoke server's temporary dirs (scripts/run-rust-smoke-server.mjs).
const smokeRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '.tmp-ui-smoke')
const smokeFlowsDir = path.join(smokeRoot, 'flows')
const smokeRolesDir = path.join(smokeRoot, 'spark-home', 'roles')

const flowFiles = (flowName: string) => Object.fromEntries(
  readdirSync(path.join(smokeFlowsDir, flowName)).map((name) => [name, readFileSync(path.join(smokeFlowsDir, flowName, name), 'utf-8')]),
)

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
  await page.getByTestId("activity-flows").click()

  const flowButton = page.getByRole("button", { name: flowName })
  await expect(flowButton).toBeVisible()
  await flowButton.click()
  await expect(page.getByTestId("canvas-workspace-primary")).toBeVisible()

  await page.getByRole("button", { name: "YAML", exact: true }).click()
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
  await expect(page.getByRole("button", { name: "+ Node", exact: true })).toHaveCount(0)
  await expect.poll(() => parseErrorBodies.length).toBeGreaterThanOrEqual(1)
  await page.screenshot({ path: screenshotPath("19a-raw-yaml-parse-error-blocked.png"), fullPage: true })

  const savedBeforeRoundTrip = savedBodies.length
  await rawYamlEditor.fill(equivalentYaml)
  await page.getByRole("button", { name: "Structured" }).click()
  await expect(page.getByRole("button", { name: "+ Node", exact: true })).toBeVisible()
  await expect.poll(() => savedBodies.length).toBeGreaterThan(savedBeforeRoundTrip)
  await page.screenshot({ path: screenshotPath("19b-raw-yaml-round-trip-saved.png"), fullPage: true })
})

test("editing one role-bearing node rewrites only that node's file", async ({ page }) => {
  const projectPath = `/tmp/ui-smoke-project-roles-${Date.now()}`
  const flowName = `ui-smoke-roles-${Date.now()}`
  mkdirSync(smokeRolesDir, { recursive: true })
  writeFileSync(path.join(smokeRolesDir, "reviewer.md"), "---\ntype: Role\n---\n\nYou are a careful reviewer.\n")
  const created = await page.request.post("/attractor/api/flows", {
    data: {
      name: flowName,
      content: `schema_version: "1"
id: roles
title: Roles
nodes:
  start: {kind: start, label: Start}
  first:
    kind: agent_task
    label: First
    role: reviewer
    config: {kind: agent_task, prompt: Review A.}
  second:
    kind: agent_task
    label: Second
    role: reviewer
    config: {kind: agent_task, prompt: Review B.}
  done: {kind: exit, label: Done}
edges:
  - {from: start, to: first}
  - {from: first, to: second}
  - {from: second, to: done}
`,
    },
  })
  expect(created.ok(), await created.text()).toBe(true)
  const before = flowFiles(flowName)
  expect(before["second.md"]).toContain("role: reviewer")

  try {
    await gotoWithRegisteredProject(page, projectPath)
    await page.getByTestId("activity-flows").click()
    await page.getByRole("button", { name: flowName }).click()
    await page.locator('.react-flow__node[data-id="first"]').click()
    const promptField = page.getByPlaceholder("Enter system prompt instructions...")
    await expect(promptField).toHaveValue("Review A.")
    const saved = page.waitForResponse((response) => (
      response.url().endsWith("/attractor/api/flows")
      && response.request().method() === "POST"
      && (response.request().postData() ?? "").includes("Review A twice.")
    ))
    await promptField.fill("Review A twice.")
    expect((await saved).ok()).toBe(true)

    const after = flowFiles(flowName)
    expect(Object.keys(after).sort()).toEqual(Object.keys(before).sort())
    expect(after["first.md"]).toContain("Review A twice.")
    expect(after["first.md"]).toContain("role: reviewer")
    for (const name of Object.keys(before).filter((file) => file !== "first.md")) {
      expect(after[name], `${name} must stay byte-identical`).toBe(before[name])
    }
  } finally {
    await page.request.delete(`/attractor/api/flows/${encodeURIComponent(flowName)}`)
  }
})
