import { expect, test, type Page } from '@playwright/test'

const SIDEBAR_PROJECT_PATH = '/tmp/ui-smoke-sidebar-project'

const seedSidebarRouteState = async (page: Page) => {
  await page.addInitScript(({ projectPath }) => {
    window.localStorage.setItem(
      'spark.ui_route_state',
      JSON.stringify({
        viewMode: 'projects',
        activeProjectPath: projectPath,
        activeFlow: null,
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
  }, { projectPath: SIDEBAR_PROJECT_PATH })
}

const stubSidebarApis = async (page: Page) => {
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
          status: 'idle',
          last_working_directory: SIDEBAR_PROJECT_PATH,
          last_flow_name: null,
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
            project_id: 'ui-smoke-sidebar-project',
            project_path: SIDEBAR_PROJECT_PATH,
            display_name: 'spark',
            created_at: '2026-04-17T17:00:00Z',
            last_opened_at: '2026-04-17T17:30:00Z',
            last_accessed_at: '2026-04-17T17:38:00Z',
            is_favorite: true,
            active_conversation_id: 'conversation-thread-b',
          },
        ]),
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

    if (pathname === '/workspace/api/projects/conversations' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify([
          {
            conversation_id: 'conversation-thread-a',
            project_path: SIDEBAR_PROJECT_PATH,
            title: "I'd like to change the default model on the global settings page and keep the rest intact",
            created_at: '2026-04-17T16:00:00Z',
            updated_at: '2026-04-17T16:30:00Z',
            revision: 1,
            last_message_preview: 'Default model change request.',
          },
          {
            conversation_id: 'conversation-thread-b',
            project_path: SIDEBAR_PROJECT_PATH,
            title: 'There is an issue with the UI where the thread card grows wider than the sidebar',
            created_at: '2026-04-17T16:10:00Z',
            updated_at: '2026-04-17T17:35:00Z',
            revision: 2,
            last_message_preview: 'Sidebar overflow investigation.',
          },
          {
            conversation_id: 'conversation-thread-c',
            project_path: SIDEBAR_PROJECT_PATH,
            title: 'Can you run the implement-change-request flow with the latest sidebar layout fix applied',
            created_at: '2026-04-16T09:00:00Z',
            updated_at: '2026-04-16T12:00:00Z',
            revision: 1,
            last_message_preview: 'Implementation flow check.',
          },
        ]),
      })
      return
    }

    if (pathname === '/workspace/api/conversations/conversation-thread-b' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          schema_version: 4,
          conversation_id: 'conversation-thread-b',
          project_path: SIDEBAR_PROJECT_PATH,
          title: 'There is an issue with the UI where the thread card grows wider than the sidebar',
          created_at: '2026-04-17T16:10:00Z',
          updated_at: '2026-04-17T17:35:00Z',
          revision: 2,
          turns: [],
          event_log: [],
        }),
      })
      return
    }

    if (pathname === '/attractor/runs' && requestMethod === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ runs: [] }),
      })
      return
    }

    await route.continue()
  })
}

test('home sidebar thread rows stay within the sidebar width for long titles', async ({ page }) => {
  await seedSidebarRouteState(page)
  await stubSidebarApis(page)

  await page.setViewportSize({ width: 1366, height: 900 })
  await page.goto('/')

  await expect(page.getByTestId('projects-panel')).toHaveAttribute('data-responsive-layout', 'split')
  await expect(page.getByTestId('project-thread-list')).toContainText(
    'There is an issue with the UI where the thread card grows wider than the sidebar',
  )

  const overflowingButtons = await page
    .getByTestId('project-thread-list')
    .getByRole('button')
    .evaluateAll((nodes) => nodes
      .map((node) => ({
        label: node.getAttribute('aria-label'),
        clientWidth: node.clientWidth,
        scrollWidth: node.scrollWidth,
      }))
      .filter(({ clientWidth, scrollWidth }) => scrollWidth > clientWidth + 1))

  expect(overflowingButtons).toEqual([])

  const scrollerMetrics = await page.getByTestId('project-thread-list').evaluate((node) => {
    const scroller = node.parentElement
    if (!scroller) {
      return { clientWidth: 0, scrollWidth: 0 }
    }
    return {
      clientWidth: scroller.clientWidth,
      scrollWidth: scroller.scrollWidth,
    }
  })

  expect(scrollerMetrics.scrollWidth).toBeLessThanOrEqual(scrollerMetrics.clientWidth + 1)
})
