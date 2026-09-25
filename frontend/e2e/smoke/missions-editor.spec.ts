import { expect, test } from '@playwright/test'
import { gotoWithRegisteredProject, stubProjectMetadata } from '../fixtures/smoke-helpers'

const project = '/tmp/missions-editor-smoke'
const at = (day: number) => `2026-09-${String(day).padStart(2, "0")} 12:00:00.0 +00:00:00`
const longObjective = 'Search should return matching documents across every indexed project, ranked by relevance.\n\nKeep the diff small and cover the ranking with tests. '.repeat(4)
const mission = (id: string, title: string, status: string, day: number, extra: Record<string, unknown> = {}) => ({
  id, revision: 1, status, updated_at: at(day), conversation_id: status === 'draft' ? null : id,
  fields: { title, description: longObjective, archived: false, budget: { concurrent_runs: 4, total_runs: 25 } },
  activity: [{ revision: 1, actor: 'assistant', at: at(day), note: 'Captured issue', before: { title: '', description: '', archived: false }, after: { title, description: longObjective, archived: false } }],
  ...extra,
})
const missions = [
  mission('mission-draft', 'Draft the importer', 'draft', 10),
  mission('mission-running', 'Ship search ranking with a title long enough to wrap onto a second line', 'running', 12, { runs: [{ run_id: 'run-build', flow_name: 'software-development/implement-change.yaml', summary: 'Implement ranking', launched_at: 't', status: 'running' }] }),
  mission('mission-gate', 'Review the ranking change', 'needs_you', 11, { runs: [{ run_id: 'run-review', flow_name: 'software-development/review-change.yaml', summary: 'Review ranking', launched_at: 't', status: 'waiting' }] }),
  mission('mission-closed', 'Retire the old index', 'closed', 9, { closed: { status: 'done', reason: 'Old index removed', at: at(9), actor: 'assistant' } }),
]
const turn = (id: string, role: string, content: string, kind = 'message') => ({ id, role, content, kind, status: 'complete', timestamp: '2026-09-12T12:00:00Z' })
const transcript = {
  schema_version: 5, revision: 5, conversation_id: 'mission-running', project_path: project, segments: [], event_log: [], flow_run_requests: [], flow_launches: [], proposed_plans: [],
  turns: [
    turn('u1', 'user', `Objective:\n${longObjective}\n\nBegin work on this mission.`),
    turn('a1', 'assistant', 'I launched **implement-change** for the ranking work and will report back when it finishes.'),
    turn('u2', 'user', 'Run run-build (software-development/implement-change.yaml, "Implement ranking") ended completed.\n\nUser: Keep the diff small.'),
    turn('a2', 'assistant', 'The build completed. Should I launch a review next?'),
    turn('n1', 'system', 'Closed as done: Old index removed', 'mission_notice'),
  ],
}

for (const theme of ['light', 'dark']) for (const width of [1440, 390]) {
  test(`missions list and transcript ${theme} ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 })
    await page.route('**/workspace/api/live/events**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' }))
    await stubProjectMetadata(page)
    let release!: () => void
    const loading = new Promise<void>(resolve => { release = resolve })
    await page.route('**/workspace/api/missions?**', async route => { await loading; return route.fulfill({ json: { missions } }) })
    await page.route('**/workspace/api/conversations/mission-*?**', route => route.fulfill({ json: transcript }))
    await gotoWithRegisteredProject(page, project)
    await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme)
    await page.getByTestId('nav-mode-missions').click()
    await expect(page.getByText('Loading…').first()).toBeVisible()
    release()

    // One list grouped by status, newest first within a group.
    const groups = page.getByRole('heading', { level: 2 })
    await expect(groups).toHaveText(['Needs you1', 'Running1', 'Drafts1', 'Closed1'])
    await expect(page.getByRole('button', { name: 'Review the ranking change' })).toHaveAccessibleDescription('Review ranking is waiting on a human gate')
    await expect(page.getByRole('button', { name: 'Archive Retire the old index' })).toBeVisible()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('list.png') })

    // A running mission reads as its transcript with the objective pinned.
    const row = page.getByRole('button', { name: /^Ship search ranking/ })
    await row.click()
    const detail = page.getByRole('region', { name: 'Mission details' })
    await expect(detail.getByRole('status')).toContainText('Running · 1 run in flight')
    await expect(detail.getByText('The build completed. Should I launch a review next?')).toBeVisible()
    await expect(detail.getByLabel('Mission events')).toContainText('User: Keep the diff small.')
    await expect(detail.getByRole('button', { name: 'Open run run-build' })).toBeVisible()
    await expect(detail.getByText('Closed as done: Old index removed')).toBeVisible()
    await expect(detail.getByLabel('Reply')).toBeInViewport()
    const objective = detail.getByRole('region', { name: 'Objective' })
    const pinnedY = (await objective.boundingBox())!.y
    await detail.getByText('The build completed. Should I launch a review next?').scrollIntoViewIfNeeded()
    expect((await objective.boundingBox())!.y).toBe(pinnedY)
    if (width < 1024) await expect(row).toBeHidden()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('transcript.png') })
    await detail.getByRole('button', { name: 'Mission actions' }).click()
    await expect(page.getByRole('menuitem')).toHaveText(['Edit', 'Budget', 'Cancel mission', 'Close mission', 'Archive'])
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('menu.png') })
    await page.getByRole('menuitem', { name: 'Budget' }).click()
    await expect(detail.getByRole('form', { name: 'Budget' }).getByLabel('Total runs')).toHaveValue('25')
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('budget.png') })
    await detail.getByRole('form', { name: 'Budget' }).getByRole('button', { name: 'Cancel' }).click()
    await detail.getByRole('button', { name: 'Close details' }).click()

    // A draft offers Start in place of the reply box.
    await page.getByRole('button', { name: 'Draft the importer' }).click()
    await expect(detail.getByRole('button', { name: 'Start' })).toBeInViewport()
    await expect(detail.getByLabel('Reply')).toHaveCount(0)
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('draft.png') })
    await detail.getByRole('button', { name: 'Mission actions' }).click()
    await page.getByRole('menuitem', { name: 'Edit' }).click()
    await expect(detail.getByLabel('Title', { exact: true })).toBeFocused()
    await expect(detail.getByRole('button', { name: 'Save mission', exact: true })).toBeInViewport()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('editor.png') })
    await page.keyboard.press('Escape')
    await expect(detail).toBeHidden()

    // A closed mission shows its outcome and no reply box.
    await page.getByRole('button', { name: 'Retire the old index', exact: true }).click()
    await expect(detail.getByRole('status')).toContainText('Closed · Closed as done: Old index removed')
    await expect(detail.getByText('This mission is closed.')).toBeVisible()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('closed.png') })
    await detail.getByRole('button', { name: 'Close details' }).click()

    // A mission waiting on a gate reads as Needs you and keeps the reply box.
    await page.getByRole('button', { name: 'Review the ranking change' }).click()
    await expect(detail.getByRole('status')).toContainText('Needs you · Review ranking is waiting on a human gate')
    await expect(detail.getByLabel('Reply')).toBeInViewport()
    await page.screenshot({ animations: 'disabled', path: test.info().outputPath('needs-you.png') })
  })
}
