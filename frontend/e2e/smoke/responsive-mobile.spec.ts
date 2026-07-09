import { expect, test, type Page } from '@playwright/test'
import { ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

const MOBILE_PROJECT_PATH = '/tmp/ui-smoke-mobile-project'
const MOBILE_FLOW_NAME = 'mobile-smoke.yaml'
const MOBILE_RUN_ID = 'run-mobile-ops'
const MOBILE_FLOW_YAML = `schema_version: "1"
id: mobile_smoke
title: Mobile Smoke
nodes:
  start:
    kind: start
    label: Start
    config:
      kind: start
  task:
    kind: agent_task
    label: Task
    config:
      kind: agent_task
      prompt: Check the mobile smoke surface.
  done:
    kind: exit
    label: Done
    config:
      kind: exit
edges:
  - from: start
    to: task
  - from: task
    to: done
`

const seedRouteState = async (page: Page) => {
  await page.addInitScript(
    ({ projectPath, flowName }) => {
      window.localStorage.setItem(
        'spark.ui_route_state',
        JSON.stringify({
          viewMode: 'projects',
          activeProjectPath: projectPath,
          activeFlow: flowName,
        }),
      )
      window.localStorage.setItem(
        'spark.project_registry_state',
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
    },
  )
}

const stubResponsiveSmokeApis = async (page: Page) => {
  await page.route('**/*', async (route) => {
    const requestUrl = route.request().url()
    const requestMethod = route.request().method()
    const url = new URL(requestUrl)
    const pathname = url.pathname

    if (pathname === '/attractor/status') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'running',
          last_working_directory: MOBILE_PROJECT_PATH,
          last_flow_name: MOBILE_FLOW_NAME,
        }),
      })
      return
    }

    if (pathname === '/workspace/api/projects' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            project_id: 'ui-smoke-mobile-project',
            project_path: MOBILE_PROJECT_PATH,
            display_name: 'ui-smoke-mobile-project',
            created_at: '2026-03-11T09:00:00Z',
            last_opened_at: '2026-03-11T09:30:00Z',
            last_accessed_at: '2026-03-11T09:45:00Z',
            is_favorite: true,
            active_conversation_id: null,
          },
        ]),
      })
      return
    }

    if (pathname === '/workspace/api/projects/conversations' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([]),
      })
      return
    }

    if (pathname === '/attractor/api/flows' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([MOBILE_FLOW_NAME]),
      })
      return
    }

    if (pathname === `/attractor/api/flows/${encodeURIComponent(MOBILE_FLOW_NAME)}` && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          name: MOBILE_FLOW_NAME,
          content: MOBILE_FLOW_YAML,
        }),
      })
      return
    }

    if (pathname === '/attractor/preview' && requestMethod === 'POST') {
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

    if (pathname === '/workspace/api/projects/metadata' && requestMethod === 'GET') {
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

    if (pathname === `/attractor/pipelines/${encodeURIComponent(MOBILE_RUN_ID)}` && requestMethod === 'GET') {
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

    if (pathname === '/attractor/runs' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          runs: [
            {
              run_id: MOBILE_RUN_ID,
              status: 'running',
              outcome: null,
              flow_name: MOBILE_FLOW_NAME,
              started_at: '2026-03-11T10:00:00Z',
              ended_at: null,
              model: 'gpt-5',
              working_directory: MOBILE_PROJECT_PATH,
              project_path: MOBILE_PROJECT_PATH,
              git_branch: 'main',
              git_commit: 'abc123def456',
              last_error: null,
              token_usage: 1234,
              spec_id: null,
              plan_id: null,
            },
          ],
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
  await expect(page.getByTestId('top-nav-active-project')).toHaveAttribute('data-responsive-layout', 'stacked')
  await expect(page.getByTestId('top-nav-project-switcher')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-add-button')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-clear-button')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-remove-button')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13a-mobile-projects-operations.png'), fullPage: true })

  await page.getByTestId('nav-mode-runs').click()
  await expect(page.getByTestId('runs-panel')).toHaveAttribute('data-responsive-layout', 'stacked')
  await page.screenshot({ path: screenshotPath('13b-mobile-runs-panel.png'), fullPage: true })
})

test('viewport regression baselines capture desktop shell layouts for projects and runs surfaces', async ({ page }) => {
  await seedRouteState(page)
  await stubResponsiveSmokeApis(page)

  await page.setViewportSize({ width: 1366, height: 900 })
  await page.goto('/')

  await expect(page.getByTestId('top-nav')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('view-mode-tabs')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'split')
  await expect(page.getByTestId('top-nav-active-project')).toHaveAttribute('data-responsive-layout', 'inline')
  await expect(page.getByTestId('top-nav-project-switcher')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-add-button')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-clear-button')).toBeVisible()
  await expect(page.getByTestId('top-nav-project-remove-button')).toBeVisible()
  await page.screenshot({ path: screenshotPath('13c-desktop-projects-operations.png'), fullPage: true })

  await page.getByTestId('nav-mode-runs').click()
  await expect(page.getByTestId('runs-panel')).toHaveAttribute('data-responsive-layout', 'split')
  await page.screenshot({ path: screenshotPath('13d-desktop-runs-panel.png'), fullPage: true })
})
