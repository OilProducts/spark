import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import { gotoWithRegisteredProject, stubProjectMetadata } from '../fixtures/smoke-helpers'

// A project's flows and roles live in its org repo.
const orgDir = (projectPath: string) => path.join(projectPath, '.spark', 'agent-org')

const flowFiles = (projectPath: string, flowName: string) => {
  const flowDir = path.join(orgDir(projectPath), 'flows', flowName)
  return Object.fromEntries(readdirSync(flowDir).map((name) => [name, readFileSync(path.join(flowDir, name), 'utf-8')]))
}

test.beforeEach(async ({ page }) => {
  await stubProjectMetadata(page)
})

test("editing one role-bearing node rewrites only that node's file", async ({ page }) => {
  const projectPath = mkdtempSync(path.join(os.tmpdir(), 'ui-smoke-project-roles-'))
  const flowName = `ui-smoke-roles-${Date.now()}`
  const rolesDir = path.join(orgDir(projectPath), 'roles')
  mkdirSync(rolesDir, { recursive: true })
  writeFileSync(path.join(rolesDir, "reviewer.md"), "---\ntype: Role\n---\n\nYou are a careful reviewer.\n")
  const created = await page.request.post("/attractor/api/flows", {
    data: {
      project_path: projectPath,
      name: flowName,
      flow: {
        schema_version: "1",
        id: "roles",
        title: "Roles",
        nodes: {
          start: { kind: "start", label: "Start" },
          first: { kind: "agent_task", label: "First", role: "reviewer", config: { kind: "agent_task", prompt: "Review A." } },
          second: { kind: "agent_task", label: "Second", role: "reviewer", config: { kind: "agent_task", prompt: "Review B." } },
          done: { kind: "exit", label: "Done" },
        },
        edges: [
          { from: "start", to: "first" },
          { from: "first", to: "second" },
          { from: "second", to: "done" },
        ],
      },
    },
  })
  expect(created.ok(), await created.text()).toBe(true)
  const before = flowFiles(projectPath, flowName)
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

    const after = flowFiles(projectPath, flowName)
    expect(Object.keys(after).sort()).toEqual(Object.keys(before).sort())
    expect(after["first.md"]).toContain("Review A twice.")
    expect(after["first.md"]).toContain("role: reviewer")
    for (const name of Object.keys(before).filter((file) => file !== "first.md")) {
      expect(after[name], `${name} must stay byte-identical`).toBe(before[name])
    }
  } finally {
    rmSync(projectPath, { recursive: true, force: true })
  }
})
