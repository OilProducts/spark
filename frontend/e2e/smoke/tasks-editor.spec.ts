import { expect, test } from '@playwright/test'
import { gotoWithRegisteredProject, stubProjectMetadata } from '../fixtures/smoke-helpers'

for (const width of [1440, 1024, 390]) {
  test(`task editor keeps actions and focus reachable at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    await page.route('**/workspace/api/live/events**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' }))
    await stubProjectMetadata(page)
    await page.route('**/workspace/api/tasks?**', route => route.fulfill({ json: {
      tasks: Array.from({ length: 36 }, (_, i) => ({
        id: `task-${i}`, revision: 1,
        fields: { title: `Task ${i}`, description: 'Outcome for a populated board', acceptance_criteria: '', next_action: '', stage: ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done'][i % 6], priority: 2, blocked: i === 0 ? 'Review needed' : '', needs_input: '', archived: false, conversations: [], artifacts: [], runs: [] },
        activity: [],
      })), runs: [], attention: [],
    } }))
    await gotoWithRegisteredProject(page, '/tmp/tasks-editor-smoke')
    await page.getByTestId('nav-mode-tasks').click()
    const card = page.getByRole('button', { name: /Task 0.*Normal.*Blocked: Review needed/ })
    await card.click()
    const editor = page.getByRole('region', { name: 'Task details' })
    await expect(editor.getByRole('heading', { name: 'Task 0', exact: true })).toBeFocused()
    await expect(page.getByTestId('top-nav')).toBeVisible()
    if (width <= 1024) await expect(card).toBeHidden()
    else {
      await expect(card).toBeVisible()
      expect((await editor.boundingBox())!.width).toBeCloseTo(448, 0)
    }
    await expect(editor.getByLabel('Blocked — explanation (empty to clear)')).toBeVisible()
    for (const section of ['Acceptance criteria', 'Archival', 'Relationships', 'Note / completion evidence']) {
      await editor.locator('summary').filter({ hasText: section }).click()
    }
    await editor.getByRole('textbox', { name: 'Note / completion evidence', exact: true }).fill('Session note')
    const footer = editor.locator('footer')
    await expect(footer.getByRole('button', { name: 'Save task' })).toBeInViewport()
    await expect(editor.getByRole('heading', { name: 'Task 0', exact: true })).toBeInViewport()
    const body = editor.locator('form > div')
    expect(await body.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true)
    await page.screenshot({ path: test.info().outputPath('expanded-editor.png') })
    await page.keyboard.press('Escape')
    await expect(editor).toBeHidden()
    await expect(card).toBeFocused()
    await card.click()
    await expect(editor.getByRole('textbox', { name: 'Note / completion evidence', exact: true })).toHaveValue('Session note')
    await editor.getByRole('button', { name: 'Close', exact: true }).click()
    await page.getByRole('button', { name: 'Create task', exact: true }).click()
    await expect(editor.getByLabel('Title', { exact: true })).toBeFocused()
    await editor.getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Create task', exact: true })).toBeFocused()
  })
}
