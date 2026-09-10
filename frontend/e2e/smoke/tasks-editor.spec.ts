import { expect, test } from '@playwright/test'
import { gotoWithRegisteredProject, stubProjectMetadata } from '../fixtures/smoke-helpers'

for (const theme of ['light', 'dark']) for (const width of [1440, 1024, 390]) {
  test(`task states and Runs comparison ${theme} ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    await page.route('**/workspace/api/live/events**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' }))
    await stubProjectMetadata(page)
    const run = { run_id: 'run-comparison', flow_name: 'Review work', status: 'completed', outcome: 'success', project_path: '/tmp/tasks-editor-smoke', working_directory: '/tmp/tasks-editor-smoke', model: 'gpt-5', started_at: '2026-09-10T12:00:00Z', ended_at: '2026-09-10T12:02:00Z', token_usage: 42, last_error: '', git_branch: 'main', git_commit: 'abc1234' }
    await page.route('**/attractor/runs**', route => route.fulfill({ json: { runs: [run] } }))
    await page.route('**/attractor/pipelines/run-comparison', route => route.fulfill({ json: { ...run, pipeline_id: run.run_id, completed_nodes: [], progress: { current_node: null, completed_count: 0 } } }))
    let populated = false
    let failure = false
    let revision = 1
    let release!: () => void
    const loading = new Promise<void>(resolve => { release = resolve })
    await page.route('**/workspace/api/tasks?**', async route => {
      await loading
      if (failure) return route.fulfill({ status: 500, json: { detail: 'Tasks are unavailable. Try Refresh.' } })
      return route.fulfill({ json: { tasks: populated ? Array.from({ length: 72 }, (_, i) => ({
        id: `task-${i}`, revision,
        fields: { title: i === 1 ? 'A long task title that wraps naturally without obscuring adjacent cards or the editor actions' : `Task ${i}`, description: revision > 1 ? 'A colleague updated the description.' : 'A readable description with enough detail to wrap over several lines.\n\nA second paragraph preserves the original formatting. '.repeat(12), stage: ['backlog', 'planning', 'ready', 'in_progress', 'review', 'done'][i % 5], archived: i === 2 },
        activity: [{ revision: 1, actor: 'assistant', at: '2026-09-10 12:00:00 +00:00:00', note: 'Captured issue', before: { title: '', description: '', stage: 'backlog', archived: false }, after: { title: `Task ${i}`, description: 'A readable description with enough detail to wrap over several lines.\n\nA second paragraph preserves the original formatting. '.repeat(12), stage: 'backlog', archived: false } }],
      })) : [] } })
    })
    await page.route('**/workspace/api/tasks/task-0?**', route => route.fulfill({ status: 409, json: { detail: 'Task changed' } }))
    await gotoWithRegisteredProject(page, '/tmp/tasks-editor-smoke')
    await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme)
    await page.getByTestId('nav-mode-runs').click()
    await page.getByTestId('run-history-row').first().click()
    await page.getByTestId('run-inspector-tab-details').click()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('runs.png') })
    await page.getByTestId('nav-mode-tasks').click()
    await expect(page.getByText('Loading…').first()).toBeVisible()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('loading.png') })
    release()
    await expect(page.getByText('No tasks')).toHaveCount(6)
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('empty.png') })
    failure = true
    await page.getByRole('button', { name: 'Refresh', exact: true }).click()
    await expect(page.getByRole('alert')).toContainText('Tasks are unavailable')
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('error.png') })
    failure = false
    populated = true
    await page.getByRole('button', { name: 'Refresh', exact: true }).click()
    const card = page.getByRole('button', { name: 'Task 0', exact: true })
    await expect(card).toBeVisible()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('populated.png') })
    const backlog = page.getByRole('region', { name: 'Backlog', exact: true })
    const board = backlog.locator('../..')
    expect((await backlog.boundingBox())!.width).toBeGreaterThanOrEqual(240)
    expect(await board.evaluate(node => node.scrollWidth > node.clientWidth)).toBe(true)
    const headerY = (await backlog.getByRole('heading').boundingBox())!.y
    const cards = backlog.locator('div').first()
    await cards.evaluate(node => { node.scrollTop = 120 })
    expect((await backlog.getByRole('heading').boundingBox())!.y).toBe(headerY)
    await page.getByLabel('Search titles').fill('MISSING')
    await expect(page.getByText('No matches')).toHaveCount(6)
    await page.getByRole('button', { name: 'Clear search' }).click()
    expect(await cards.evaluate(node => node.scrollTop)).toBe(120)
    await cards.evaluate(node => { node.scrollTop = 0 })
    await page.getByRole('button', { name: 'Show archived' }).click()
    await expect(page.getByRole('button', { name: 'Show archived' })).toHaveAttribute('aria-pressed', 'true')
    await page.getByRole('button', { name: 'Task 2', exact: true }).click()
    const archivedDetail = page.getByRole('region', { name: 'Task details' })
    await expect(archivedDetail.getByText('Archived', { exact: true })).toBeVisible()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('archived.png') })
    await archivedDetail.getByRole('button', { name: 'Close', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Task 2', exact: true })).toBeFocused()
    await board.evaluate(node => { node.scrollLeft = 0 })
    await page.keyboard.press('Tab')
    await card.focus()
    await board.evaluate(node => { node.scrollLeft = 40 })
    await cards.evaluate(node => { node.scrollTop = 20 })
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('board-focus.png') })
    await page.keyboard.press('Enter')
    const editor = page.getByRole('region', { name: 'Task details' })
    await expect(editor.getByRole('heading', { name: 'Task 0', exact: true })).toBeFocused()
    await expect(page.getByTestId('top-nav')).toBeVisible()
    if (width <= 1024) await expect(card).toBeHidden()
    else {
      await expect(card).toBeVisible()
      expect((await editor.boundingBox())!.width).toBeCloseTo(448, 0)
    }
    await expect(editor.locator('details')).not.toHaveAttribute('open')
    await expect(page.getByRole('button', { name: 'Task 0', exact: true, includeHidden: true })).toHaveAttribute('aria-pressed', 'true')
    await page.getByLabel('Search titles').fill('no visible cards')
    await expect(editor.getByRole('heading', { name: 'Task 0', exact: true })).toBeVisible()
    await page.getByRole('button', { name: 'Clear search' }).click()
    await expect(editor.getByRole('button', { name: 'Edit', exact: true })).toBeInViewport()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('read.png') })
    await editor.getByRole('button', { name: 'Edit', exact: true }).click()
    await expect(editor.getByLabel('Title', { exact: true })).toBeFocused()
    await editor.getByRole('textbox', { name: 'Description', exact: true }).fill('Session draft')
    await expect(editor.getByRole('button', { name: 'Save', exact: true })).toBeInViewport()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('editor.png') })
    await page.keyboard.press('Escape')
    await expect(editor).toBeHidden()
    await expect(card).toBeFocused()
    expect(await board.evaluate(node => node.scrollLeft)).toBe(40)
    expect(await cards.evaluate(node => node.scrollTop)).toBe(20)
    await card.click()
    await expect(editor.getByRole('textbox', { name: 'Description', exact: true })).toHaveValue('Session draft')
    revision = 2
    await editor.getByRole('button', { name: 'Save', exact: true }).click()
    await expect(editor.getByRole('status')).toContainText('newer revision')
    await expect(editor.locator('pre')).toHaveCount(0)
    await expect(editor.getByRole('button', { name: 'Save', exact: true })).toBeDisabled()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('conflict.png') })
    await editor.getByRole('button', { name: 'Reconcile with latest revision' }).click()
    await expect(editor.getByRole('button', { name: 'Save', exact: true })).toBeEnabled()
    await editor.locator('summary').click()
    await expect(editor.getByText('Title: Empty → Task 0')).toBeVisible()
    await editor.getByText('Title: Empty → Task 0').scrollIntoViewIfNeeded()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('activity.png') })
    await editor.getByRole('button', { name: 'Close', exact: true }).click()
    await page.getByRole('button', { name: 'Create task', exact: true }).click()
    await expect(editor.getByLabel('Title', { exact: true })).toBeFocused()
    await expect(editor.getByRole('combobox', { name: 'Stage', exact: true })).toHaveValue('backlog')
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('create.png') })
    await editor.getByRole('button', { name: 'Cancel', exact: true }).click()
    await expect(page.getByRole('button', { name: 'Create task', exact: true })).toBeFocused()
  })
}
