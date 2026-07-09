import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

const smokeFlowContent = `schema_version: "1"
id: rust_product_shell
title: Rust Product Shell
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

test.beforeAll(() => {
  ensureScreenshotDir()
})

test('Rust product shell serves the built SPA and owns core browser routes', async ({ page }) => {
  const rootResponse = await page.goto('/')
  expect(rootResponse?.ok()).toBeTruthy()
  expect(rootResponse?.headers()['content-type']).toContain('text/html')
  await expect(page.getByTestId('app-shell')).toBeVisible()
  await expect(page.getByTestId('top-nav')).toBeVisible()

  const attractorStatus = await page.request.get('/attractor/status')
  expect(attractorStatus.ok()).toBeTruthy()
  expect(attractorStatus.headers()['content-type']).toContain('application/json')
  await expect(attractorStatus.json()).resolves.toMatchObject({ status: 'idle' })

  const workspaceSettings = await page.request.get('/workspace/api/settings')
  expect(workspaceSettings.ok()).toBeTruthy()
  expect(workspaceSettings.headers()['content-type']).toContain('application/json')
  await expect(workspaceSettings.json()).resolves.toHaveProperty('execution_placement')

  const missingApi = await page.request.get('/workspace/api/not-a-real-route')
  expect(missingApi.status()).toBe(404)
  expect(missingApi.headers()['content-type']).toContain('application/json')
  await expect(missingApi.json()).resolves.toEqual({ detail: 'Not Found' })

  const flowName = `rust-shell-${Date.now()}.yaml`
  const projectRoot = mkdtempSync(path.join(os.tmpdir(), 'spark-rust-shell-project-'))
  mkdirSync(projectRoot, { recursive: true })
  try {
    const saveFlow = await page.request.post('/attractor/api/flows', {
      data: { name: flowName, content: smokeFlowContent },
    })
    expect(saveFlow.ok()).toBeTruthy()

    const listFlows = await page.request.get('/attractor/api/flows')
    expect(listFlows.ok()).toBeTruthy()
    await expect(listFlows.json()).resolves.toContain(flowName)

    const readFlow = await page.request.get(`/attractor/api/flows/${encodeURIComponent(flowName)}`)
    expect(readFlow.ok()).toBeTruthy()
    await expect(readFlow.json()).resolves.toMatchObject({
      name: flowName,
      content: expect.stringContaining('rust_product_shell'),
    })

    const registerProject = await page.request.post('/workspace/api/projects/register', {
      data: { project_path: projectRoot },
    })
    expect(registerProject.ok()).toBeTruthy()
    await expect(registerProject.json()).resolves.toMatchObject({
      project_path: projectRoot,
      display_name: path.basename(projectRoot),
    })

    const updateProject = await page.request.patch('/workspace/api/projects/state', {
      data: { project_path: projectRoot, is_favorite: true },
    })
    expect(updateProject.ok()).toBeTruthy()
    await expect(updateProject.json()).resolves.toMatchObject({
      project_path: projectRoot,
      is_favorite: true,
    })

    await page.getByTestId('nav-mode-editor').click()
    await expect(page.getByTestId('canvas-workspace-primary')).toBeVisible()
    await page.getByTestId('nav-mode-runs').click()
    await expect(page.getByTestId('runs-panel')).toBeVisible()
    await page.screenshot({ path: screenshotPath('00-rust-product-shell.png'), fullPage: true })
  } finally {
    await page.request.delete(`/attractor/api/flows/${encodeURIComponent(flowName)}`).catch(() => undefined)
    rmSync(projectRoot, { recursive: true, force: true })
  }
})
