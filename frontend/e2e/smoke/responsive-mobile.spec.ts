import { expect, test, type Page } from '@playwright/test'
import { ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

const MOBILE_PROJECT_PATH = '/tmp/ui-smoke-mobile-project'
const MOBILE_FLOW_NAME = 'implement-spec.dot'
const MOBILE_RUN_ID = 'run-mobile-ops'
const MOBILE_FLOW_DOT = `digraph G {
  start [label="Start"]
  task [label="Task"]
  start -> task
}`

const seedRouteState = async (page: Page) => {
  await page.addInitScript(
    ({ projectPath, flowName, runId }) => {
      window.localStorage.setItem(
        'sparkspawn.ui_route_state',
        JSON.stringify({
          viewMode: 'projects',
          activeProjectPath: projectPath,
          activeFlow: flowName,
          selectedRunId: runId,
        }),
      )
      window.localStorage.setItem(
        'sparkspawn.project_registry_state',
        JSON.stringify({
          [projectPath]: {
            directoryPath: projectPath,
            isFavorite: true,
            lastAccessedAt: new Date(0).toISOString(),
          },
        }),
      )
    },
    {
      projectPath: MOBILE_PROJECT_PATH,
      flowName: MOBILE_FLOW_NAME,
      runId: MOBILE_RUN_ID,
    },
  )
}

const stubResponsiveSmokeApis = async (page: Page) => {
  await page.route('**/*', async (route) => {
    const requestUrl = route.request().url()
    const requestMethod = route.request().method()
    const url = new URL(requestUrl)
    const pathname = url.pathname

    if (pathname === '/status') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'running',
          last_run_id: MOBILE_RUN_ID,
          last_working_directory: MOBILE_PROJECT_PATH,
          last_flow_name: MOBILE_FLOW_NAME,
        }),
      })
      return
    }

    if (pathname === '/runs') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          runs: [],
        }),
      })
      return
    }

    if (pathname === '/api/flows' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([MOBILE_FLOW_NAME]),
      })
      return
    }

    if (pathname === `/api/flows/${encodeURIComponent(MOBILE_FLOW_NAME)}` && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          name: MOBILE_FLOW_NAME,
          content: MOBILE_FLOW_DOT,
        }),
      })
      return
    }

    if (pathname === '/preview' && requestMethod === 'POST') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'ok',
          graph: {
            nodes: [],
            edges: [],
          },
          diagnostics: [],
        }),
      })
      return
    }

    if (pathname === '/api/projects/metadata' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          branch: 'main',
          commit: 'abcdef1234567890abcdef1234567890abcdef12',
        }),
      })
      return
    }

    if (pathname === `/pipelines/${encodeURIComponent(MOBILE_RUN_ID)}` && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          pipeline_id: MOBILE_RUN_ID,
          status: 'running',
          flow_name: MOBILE_FLOW_NAME,
          working_directory: MOBILE_PROJECT_PATH,
          model: 'gpt-5',
          completed_nodes: [],
          last_error: null,
        }),
      })
      return
    }

    await route.continue()
  })
}

test.beforeAll(() => {
  ensureScreenshotDir()
})

test('mobile and narrow viewport usability is preserved for core project and operational tasks', async ({ page }) => {
  await seedRouteState(page)
  await stubResponsiveSmokeApis(page)

  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/')

  await expect(page.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('project-register-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('project-register-button')).toBeVisible()
  await expect(page.getByTestId('favorite-toggle-button')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13a-mobile-projects-operations.png'), fullPage: true })

  await page.getByTestId('nav-mode-execution').click()
  await expect(page.getByTestId('execution-footer-controls')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('execution-footer-cancel-button')).toBeVisible()
  await expect(page.getByTestId('execution-footer-unsupported-controls-reason')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13b-mobile-execution-controls.png'), fullPage: true })
})

test('viewport regression baselines capture desktop shell layouts for projects and execution surfaces', async ({ page }) => {
  await seedRouteState(page)
  await stubResponsiveSmokeApis(page)

  await page.setViewportSize({ width: 1366, height: 900 })
  await page.goto('/')

  await expect(page.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'split')
  await expect(page.getByTestId('project-register-controls')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('project-register-button')).toBeVisible()
  await expect(page.getByTestId('favorite-toggle-button')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13c-desktop-projects-operations.png'), fullPage: true })

  await page.getByTestId('nav-mode-execution').click()
  await expect(page.getByTestId('execution-footer-controls')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('execution-footer-cancel-button')).toBeVisible()
  await expect(page.getByTestId('execution-footer-unsupported-controls-reason')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13d-desktop-execution-controls.png'), fullPage: true })
})
