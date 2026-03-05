import { mkdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Page } from '@playwright/test'

const testDir = path.dirname(fileURLToPath(import.meta.url))
const screenshotDir = path.resolve(testDir, '..', '..', 'artifacts', 'ui-smoke')
const IMPLEMENT_SPEC_FLOW = 'implement-spec.dot'
const smokeFlowRegistry = new WeakMap<Page, Set<string>>()

function getRegisteredSmokeFlows(page: Page): Set<string> {
  const existing = smokeFlowRegistry.get(page)
  if (existing) {
    return existing
  }
  const created = new Set<string>()
  smokeFlowRegistry.set(page, created)
  return created
}

export const ensureScreenshotDir = () => {
  mkdirSync(screenshotDir, { recursive: true })
}

export const screenshotPath = (name: string) => path.join(screenshotDir, name)

export async function stubProjectMetadata(page: Page, metadata?: { branch?: string; commit?: string }) {
  await page.route('**/api/projects/metadata**', async (route) => {
    const requestUrl = new URL(route.request().url())
    const directory = requestUrl.searchParams.get('directory') ?? ''
    const name = directory.split('/').filter(Boolean).pop() ?? directory
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        name,
        directory,
        branch: metadata?.branch ?? 'main',
        commit: metadata?.commit ?? 'smoke-test',
      }),
    })
  })
}

export async function cloneFlowForSmokeTest(page: Page, clonePrefix: string): Promise<string> {
  const cloneName = `${clonePrefix}-${Date.now()}.dot`
  const sourceResponse = await page.request.get(`/api/flows/${encodeURIComponent(IMPLEMENT_SPEC_FLOW)}`)
  if (!sourceResponse.ok()) {
    throw new Error(`Failed to load ${IMPLEMENT_SPEC_FLOW} for smoke clone: HTTP ${sourceResponse.status()}`)
  }

  const sourcePayload = (await sourceResponse.json()) as { content?: unknown }
  if (typeof sourcePayload.content !== 'string' || sourcePayload.content.length === 0) {
    throw new Error(`Source flow ${IMPLEMENT_SPEC_FLOW} returned empty content for smoke clone.`)
  }

  const saveResponse = await page.request.post('/api/flows', {
    data: { name: cloneName, content: sourcePayload.content },
  })
  if (!saveResponse.ok()) {
    throw new Error(`Failed to persist smoke clone ${cloneName}: HTTP ${saveResponse.status()}`)
  }
  getRegisteredSmokeFlows(page).add(cloneName)
  return cloneName
}

export async function deleteFlowAfterSmoke(page: Page, flowName: string): Promise<void> {
  const response = await page.request.delete(`/api/flows/${encodeURIComponent(flowName)}`)
  if (!response.ok() && response.status() !== 404) {
    throw new Error(`Failed to delete smoke clone ${flowName}: HTTP ${response.status()}`)
  }
  const registered = smokeFlowRegistry.get(page)
  if (registered) {
    registered.delete(flowName)
    if (registered.size === 0) {
      smokeFlowRegistry.delete(page)
    }
  }
}

export async function cleanupSmokeFlowsForPage(page: Page): Promise<void> {
  const registered = smokeFlowRegistry.get(page)
  if (!registered || registered.size === 0) {
    return
  }
  const flowNames = [...registered]
  for (const flowName of flowNames) {
    try {
      await deleteFlowAfterSmoke(page, flowName)
    } catch {
      // Best-effort cleanup; afterEach should not fail due to stale temp flow deletion errors.
    }
  }
  smokeFlowRegistry.delete(page)
}
