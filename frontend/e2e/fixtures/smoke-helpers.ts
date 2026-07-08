import { mkdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, type Page } from '@playwright/test'

const testDir = path.dirname(fileURLToPath(import.meta.url))
const screenshotDir = path.resolve(testDir, '..', '..', 'artifacts', 'ui-smoke')
const repoRoot = path.resolve(testDir, '..', '..', '..')
const smokeFixturePath = path.join(repoRoot, 'crates', 'test-fixtures', 'flows', 'flows', 'editor-smoke-base.dot')
const smokeFixtureContent = readFileSync(smokeFixturePath, 'utf-8')
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

const buildSmokeProjectRecord = (projectPath: string) => {
  const displayName = path.basename(projectPath) || projectPath
  return {
    project_id: `smoke-${displayName}`,
    project_path: projectPath,
    display_name: displayName,
    created_at: '2026-03-11T09:00:00Z',
    last_opened_at: '2026-03-11T09:30:00Z',
    last_accessed_at: '2026-03-11T09:45:00Z',
    is_favorite: false,
    active_conversation_id: null,
  }
}

export async function stubProjectMetadata(page: Page, metadata?: { branch?: string; commit?: string }) {
  await page.route('**/workspace/api/projects/metadata**', async (route) => {
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

export async function stubProjectRegistration(page: Page, projectPath: string) {
  let registered = false
  const projectRecord = buildSmokeProjectRecord(projectPath)

  await page.route('**/workspace/api/projects', async (route) => {
    if (route.request().method() !== 'GET') {
      await route.continue()
      return
    }
    const requestUrl = new URL(route.request().url())
    if (requestUrl.pathname !== '/workspace/api/projects') {
      await route.continue()
      return
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(registered ? [projectRecord] : []),
    })
  })

  await page.route('**/workspace/api/projects/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([]),
    })
  })

  await page.route('**/workspace/api/projects/browse**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        current_path: projectPath,
        parent_path: path.dirname(projectPath),
        entries: [],
      }),
    })
  })

  await page.route('**/workspace/api/projects/register', async (route) => {
    registered = true
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(projectRecord),
    })
  })

  await page.route('**/workspace/api/projects/state', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(projectRecord),
    })
  })
}

export async function registerProjectForSmokeTest(page: Page, projectPath: string) {
  await page.getByTestId('top-nav-project-add-button').click()
  await page.getByTestId('project-browser-select-button').click()
  await expect(page.getByTestId('top-nav-project-switcher')).toContainText(path.basename(projectPath) || projectPath)
}

export async function gotoWithRegisteredProject(page: Page, projectPath: string) {
  await stubProjectRegistration(page, projectPath)
  await page.goto('/')
  await registerProjectForSmokeTest(page, projectPath)
}

export async function createFlowForSmokeTest(page: Page, flowPrefix: string): Promise<string> {
  const flowName = `${flowPrefix}-${Date.now()}.dot`
  const saveResponse = await page.request.post('/attractor/api/flows', {
    data: { name: flowName, content: smokeFixtureContent },
  })
  if (!saveResponse.ok()) {
    throw new Error(`Failed to persist smoke flow ${flowName}: HTTP ${saveResponse.status()}`)
  }
  getRegisteredSmokeFlows(page).add(flowName)
  return flowName
}

export async function deleteFlowAfterSmoke(page: Page, flowName: string): Promise<void> {
  let response: Awaited<ReturnType<Page['request']['delete']>> | null = null
  try {
    response = await page.request.delete(`/attractor/api/flows/${encodeURIComponent(flowName)}`, {
      timeout: 2_000,
    })
  } catch (error) {
    if (error instanceof Error && /timeout/i.test(error.message)) {
      return
    }
    throw error
  }
  if (!response) {
    return
  }
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
