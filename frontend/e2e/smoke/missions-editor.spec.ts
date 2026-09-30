import { customModel } from '../fixtures/model-picker'
import { expect, test, type Page } from '@playwright/test'
import { gotoWithRegisteredProject, stubProjectMetadata } from '../fixtures/smoke-helpers'

const project = '/tmp/missions-editor-smoke'
const at = (day: number) => `2026-09-${String(day).padStart(2, "0")} 12:00:00.0 +00:00:00`
const longObjective = 'Search should return matching documents across every indexed project, ranked by relevance.\n\nKeep the diff small and cover the ranking with tests. '.repeat(4)
const closingSummary = 'Old index removed.\n- Dropped the legacy tables\n- Rebuilt the search cache\n- Verified every project still indexes\n- Cleaned up the migration scripts\n- Updated the docs'
const question = { flow_name: 'software-development/review-change.yaml', node_id: 'confirm', options: [{ key: 'S', label: 'Ship it', value: 'Ship it' }, { key: 'H', label: 'Hold', value: 'Hold' }], prompt: 'Ship the ranking change?', question_id: 'confirm-1', root_run_id: 'run-review', run_id: 'run-review' }
const mission = (id: string, title: string, status: string, day: number, extra: Record<string, unknown> = {}) => ({
  id, revision: 1, status, created_at: at(day), updated_at: at(day), started_at: status === 'draft' ? null : at(day), conversation_id: status === 'draft' ? null : id,
  fields: { title, description: longObjective, archived: false, budget: { concurrent_runs: 4, total_runs: 25 } },
  activity: [],
  ...extra,
})
const roster = (run_id: string, flow_name: string, summary: string, status: string) => ({ run_id, flow_name, summary, launched_at: at(12), status })
const initialMissions = () => [
  mission('mission-draft', 'Draft the importer', 'draft', 10),
  mission('mission-running', 'Ship search ranking with a title long enough to wrap onto a second line', 'running', 12, { runs: [roster('run-build', 'software-development/implement-change.yaml', 'Implement ranking', 'running')], playbook: { name: 'bug-report', title: 'Bug report', description: 'Fix a bug.', text: 'Reproduce the defect first.' } }),
  mission('mission-gate', 'Review the ranking change', 'needs_you', 11, { runs: [roster('run-build', 'software-development/implement-change.yaml', 'Implement ranking', 'completed'), roster('run-review', 'software-development/review-change.yaml', 'Review ranking', 'waiting')], question }),
  mission('mission-closed', 'Retire the old index', 'closed', 9, { closed: { status: 'done', reason: closingSummary, at: at(9), actor: 'assistant' } }),
  mission('mission-archived', 'Rename the index files', 'closed', 8, { closed: { status: 'failed', reason: 'Gave up', at: at(8), actor: 'assistant' }, fields: { title: 'Rename the index files', description: 'Rename them.', archived: true } }),
]
const run = (run_id: string, title: string, flow_name: string, status: string, token_usage: number) => ({
  run_id, title, flow_name, status, outcome: status === 'completed' ? 'success' : null, working_directory: project, project_path: project, model: 'gpt-5',
  started_at: '2026-09-12T12:00:00Z', ended_at: status === 'completed' ? '2026-09-12T12:14:00Z' : null, last_error: '', token_usage,
})
const turn = (id: string, role: string, content: string, kind = 'message') => ({ id, role, content, kind, status: 'complete', timestamp: '2026-09-12T12:00:00Z' })
const transcript = (id: string) => ({
  schema_version: 5, revision: 5, conversation_id: id, project_path: project, segments: [], event_log: [], flow_run_requests: [], flow_launches: [], proposed_plans: [],
  turns: [
    turn('u1', 'user', `Objective:\n${longObjective}\n\nBegin work on this mission.`),
    turn('a1', 'assistant', 'I launched **implement-change** for the ranking work and will report back when it finishes.'),
    turn('u2', 'user', 'Run run-build (software-development/implement-change.yaml, "Implement ranking") ended completed.\n\nUser: Keep the diff small.'),
    ...(id === 'mission-gate' ? [turn('u3', 'user', `Run question: ${JSON.stringify(question)}`), turn('a2', 'assistant', 'The review asks whether to ship. That is your call.')] : []),
    ...(id === 'mission-closed' ? [turn('n1', 'system', `Closed as done: ${closingSummary}`, 'mission_notice')] : []),
  ],
})

async function stubMissions(page: Page) {
  const missions = initialMissions()
  const posts: { url: string; body: unknown }[] = []
  await page.route('**/workspace/api/live/events**', route => route.fulfill({ status: 200, contentType: 'text/event-stream', body: '' }))
  await stubProjectMetadata(page)
  await page.route('**/attractor/runs**', route => route.fulfill({ json: { runs: [run('run-build', 'Rank search results by relevance', 'software-development/implement-change.yaml', 'completed', 193_000), run('run-review', 'Review the ranking diff', 'software-development/review-change.yaml', 'waiting', 12_000)] } }))
  await page.route('**/workspace/api/conversations/mission-*', route => route.request().method() === 'GET'
    ? route.fulfill({ json: transcript(new URL(route.request().url()).pathname.split('/').pop()!) })
    : route.fallback())
  await page.route('**/workspace/api/playbooks', route => route.fulfill({ json: [{ name: 'bug-report', title: 'Bug report', description: 'Fix a bug.' }] }))
  await page.route('**/workspace/api/missions**', route => {
    const request = route.request()
    const id = /\/missions\/([^/?]+)/.exec(request.url())?.[1]
    if (!id) return route.fulfill({ json: { missions } })
    const body = request.postDataJSON()
    posts.push({ url: new URL(request.url()).pathname, body })
    const index = missions.findIndex(item => item.id === id)
    if (request.method() === 'PATCH') missions[index] = { ...missions[index], revision: missions[index].revision + 1, fields: { ...missions[index].fields, ...body.fields } }
    return route.fulfill({ json: missions[index] })
  })
  return posts
}

for (const theme of ['light', 'dark']) test(`missions conversation view ${theme}`, async ({ page }) => {
  const posts = await stubMissions(page)
  await gotoWithRegisteredProject(page, project)
  await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme)
  await page.getByTestId('nav-mode-missions').click()

  // One list grouped by state, each row one short line; archived missions stay hidden.
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(['Needs you1', 'Running1', 'Drafts1', 'Closed1'])
  await expect(page.getByRole('button', { name: 'Review the ranking change' })).toHaveAccessibleDescription('Ship the ranking change?')
  await expect(page.getByRole('button', { name: 'Draft the importer' })).toHaveAccessibleDescription(/^Draft · created Sep 10/)
  const closedRow = page.getByRole('button', { name: 'Retire the old index', exact: true })
  await expect(closedRow).toHaveAccessibleDescription(/^Done · Sep 9/)
  const closedLine = closedRow.getByTestId('mission-status-line')
  expect((await closedLine.boundingBox())!.height).toBeLessThan(20)
  await expect(page.getByText(/Dropped the legacy tables/)).toHaveCount(0)
  await expect(page.getByRole('button', { name: 'Rename the index files' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /^(Archive|Restore)/ })).toHaveCount(0)
  await page.screenshot({ animations: 'disabled', path: test.info().outputPath('list.png') })

  // A mission that needs you ends its thread with the question; an option posts as your reply.
  await page.getByRole('button', { name: 'Review the ranking change' }).click()
  const detail = page.getByRole('region', { name: 'Mission details' })
  await expect(detail.getByRole('status')).toHaveText(/^Needs you · Ship the ranking change\?/)
  await expect(detail.getByRole('button', { name: 'Close mission' })).toBeVisible()
  await expect(detail.getByRole('button', { name: 'Cancel mission' })).toBeVisible()
  const events = detail.getByTestId('mission-event')
  await expect(events).toHaveCount(3)
  await expect(events.nth(0)).toContainText('Mission started')
  await expect(events.nth(1)).toContainText('✓Rank search results by relevance ended completed')
  await expect(events.nth(2)).toContainText('?Review the ranking diff asked “Ship the ranking change?”')
  await expect(detail.getByText('Keep the diff small.')).toBeVisible()
  const ask = detail.getByRole('region', { name: 'Waiting on you' })
  const thread = detail.getByTestId('mission-conversation')
  expect((await ask.boundingBox())!.y).toBeGreaterThan((await detail.getByText('That is your call.').boundingBox())!.y)
  await expect(ask.getByText('Ship the ranking change?')).toBeVisible()
  await expect(detail.getByLabel('Reply')).toBeVisible()
  // The objective appears once, in the rail.
  const rail = detail.getByRole('complementary', { name: 'Mission overview' })
  await expect(detail.getByText(/^Search should return matching documents/)).toHaveCount(1)
  await expect(rail.getByRole('region', { name: 'Objective' })).toContainText('Search should return matching documents')
  expect(await thread.getByText(/Search should return matching documents/).count()).toBe(0)
  // The rail lists the runs, joined with their facts, and budget, tokens and model.
  const runs = rail.getByRole('region', { name: 'Runs' })
  await expect(runs.getByRole('link')).toHaveText(['Rank search results by relevance', 'Review the ranking diff'])
  await expect(runs.getByRole('link', { name: 'Review the ranking diff' })).toHaveAttribute('href', '#/runs/run-review')
  await expect(runs).toContainText('Implement Change')
  await expect(runs).toContainText('14m')
  await expect(rail.getByRole('region', { name: 'Budget' })).toContainText('2 of 25 runs · up to 4 at once')
  await expect(rail.getByRole('region', { name: 'Spent' })).toContainText('205k tokens across runs')
  await expect(rail.getByRole('region', { name: 'Model' })).toContainText('Project default')
  await page.screenshot({ animations: 'disabled', path: test.info().outputPath('needs-you.png') })
  await ask.getByRole('button', { name: 'Ship it' }).click()
  await expect.poll(() => posts.at(-1)).toEqual({ url: '/workspace/api/missions/mission-gate/events', body: { kind: 'human.message', payload: { message: 'Ship it' } } })

  // Model and budget edit in place in the rail.
  await detail.getByRole('button', { name: 'Mission actions' }).click()
  await expect(page.getByRole('menuitem')).toHaveText(['Edit', 'Model', 'Budget', 'Archive'])
  await page.getByRole('menuitem', { name: 'Model', exact: true }).click()
  const modelForm = rail.getByRole('form', { name: 'Model', exact: true })
  await expect(modelForm.getByRole('button', { name: /^Model:/ })).toContainText('Default:')
  await customModel(page, 'mission-custom-model', modelForm)
  await modelForm.getByRole('button', { name: 'Cancel', exact: true }).click()
  await rail.getByRole('button', { name: 'Edit budget' }).click()
  await expect(rail.getByRole('form', { name: 'Budget' }).getByLabel('Total runs')).toHaveValue('25')
  await rail.getByRole('form', { name: 'Budget' }).getByRole('button', { name: 'Cancel' }).click()

  // Following a run link opens it on the Runs page.
  await runs.getByRole('link', { name: 'Rank search results by relevance' }).click()
  await expect(detail).toBeHidden()
  await page.getByTestId('nav-mode-missions').click()
  await detail.getByRole('button', { name: 'Close details' }).click()

  // A draft shows an empty thread, Start and Edit, and a disabled reply box.
  await page.getByRole('button', { name: 'Draft the importer' }).click()
  await expect(detail.getByText(/No conversation yet/)).toBeVisible()
  await expect(detail.getByRole('button', { name: 'Start' })).toBeVisible()
  await expect(detail.getByRole('button', { name: 'Edit', exact: true })).toBeVisible()
  await expect(detail.getByLabel('Reply')).toBeDisabled()
  await page.screenshot({ animations: 'disabled', path: test.info().outputPath('draft.png') })
  await detail.getByRole('button', { name: 'Edit', exact: true }).click()
  await expect(detail.getByLabel('Title', { exact: true })).toBeFocused()
  await expect(detail.getByLabel('Playbook').getByRole('option', { name: 'Bug report' })).toBeAttached()
  await page.keyboard.press('Escape')
  await expect(detail).toBeHidden()

  // A closed mission ends with its outcome and has no reply box.
  await closedRow.click()
  await expect(detail.getByRole('status')).toHaveText(/^Closed · Done · Sep 9/)
  const outcome = detail.getByRole('region', { name: 'Outcome' })
  await expect(outcome).toContainText('Closed as done · Sep 9')
  await expect(outcome).toContainText('Dropped the legacy tables')
  await expect(detail.getByText(/Dropped the legacy tables/)).toHaveCount(1)
  await expect(detail.getByLabel('Reply')).toHaveCount(0)
  await page.screenshot({ animations: 'disabled', path: test.info().outputPath('closed.png') })

  // Archive and Restore work from the actions; archived missions show only with Show archived.
  await detail.getByRole('button', { name: 'Archive', exact: true }).click()
  await expect(closedRow).toHaveCount(0)
  await expect(detail.getByRole('status')).toContainText('· Archived')
  await page.getByRole('button', { name: 'Show archived' }).click()
  await expect(closedRow).toHaveAccessibleDescription(/ · Archived$/)
  await expect(page.getByRole('button', { name: 'Rename the index files' })).toBeVisible()
  await detail.getByRole('button', { name: 'Restore', exact: true }).click()
  await expect(closedRow).toHaveAccessibleDescription(/^Done · Sep 9$/)
  await page.getByRole('button', { name: 'Hide archived' }).click()
  await expect(closedRow).toBeVisible()
  await expect(page.getByRole('button', { name: 'Rename the index files' })).toHaveCount(0)
  expect(posts.filter(post => post.url.endsWith('/mission-closed')).map(post => post.body)).toEqual([
    { revision: 1, fields: { archived: true }, actor: 'human' },
    { revision: 2, fields: { archived: false }, actor: 'human' },
  ])
})

test('missions rail stacks below the conversation at a narrow width', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 })
  await stubMissions(page)
  await gotoWithRegisteredProject(page, project)
  await page.getByTestId('nav-mode-missions').click()
  const row = page.getByRole('button', { name: 'Review the ranking change' })
  await row.click()
  const detail = page.getByRole('region', { name: 'Mission details' })
  await expect(row).toBeHidden()
  const conversation = await detail.getByTestId('mission-conversation').boundingBox()
  const rail = await detail.getByRole('complementary', { name: 'Mission overview' }).boundingBox()
  expect(rail!.y).toBeGreaterThanOrEqual(conversation!.y + conversation!.height - 1)
  expect(rail!.width).toBeGreaterThan(300)
  await expect(detail.getByRole('region', { name: 'Waiting on you' })).toBeVisible()
  await page.screenshot({ animations: 'disabled', fullPage: true, path: test.info().outputPath('narrow.png') })
})
