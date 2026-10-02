import { mkdirSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

const run = (runId: string, projectPath: string, status: string, endedAt: string | null, title: string) => ({
    run_id: runId, title, flow_name: 'smoke/overview-review.yaml', status, outcome: status === 'completed' ? 'success' : status === 'failed' ? 'failure' : null,
    working_directory: projectPath, project_path: projectPath, model: 'gpt-5', started_at: new Date(Date.now() - 3_600_000).toISOString(),
    ended_at: endedAt, last_error: status === 'failed' ? 'boom' : '', token_usage: null,
})

test.beforeAll(() => {
    ensureScreenshotDir()
})

test('Spark opens on the Overview: last chat, what needs you, what finished and where it came from, and an unread dot', async ({ page }, testInfo) => {
    const stamp = Date.now()
    const project = testInfo.outputPath('overview-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    const lastChat = `conversation-overview-last-${stamp}`
    const otherChat = `conversation-overview-other-${stamp}`
    // The other chat has the newer message; the last chat wins on the run it launched, which ends later still.
    for (const id of [lastChat, otherChat]) {
        expect((await page.request.put(`/workspace/api/conversations/${id}/settings`, { data: { project_path: project, expected_revision: '0' } })).ok()).toBeTruthy()
    }
    const created = await page.request.post(`/workspace/api/missions?project_path=${encodeURIComponent(project)}`, {
        data: { fields: { title: `Overview mission ${stamp}`, description: 'Overview smoke', archived: false }, actor: 'human' },
    })
    expect(created.ok(), await created.text()).toBeTruthy()
    const missionId = (await created.json()).id as string

    const doneRun = `run-overview-done-${stamp}`
    const failedRun = `run-overview-failed-${stamp}`
    const waitingRun = `run-overview-waiting-${stamp}`
    await page.waitForTimeout(20)
    const doneAt = new Date().toISOString()
    await page.route('**/attractor/runs', (route) => route.fulfill({ json: { runs: [
        run(doneRun, project, 'completed', doneAt, 'Reviewed the change'),
        run(failedRun, project, 'failed', new Date(Date.now() - 60_000).toISOString(), 'Mission build'),
        run(waitingRun, project, 'waiting', null, 'Waiting for approval'),
    ] } }))
    // The chat launched the completed run; the mission's roster holds the failed one.
    await page.route((url) => url.pathname === '/workspace/api/projects/conversations', async (route) => {
        const response = await route.fetch()
        const chats = (await response.json()) as { conversation_id: string; launched_run_ids?: string[] }[]
        await route.fulfill({ response, json: chats.map((chat) => chat.conversation_id === lastChat ? { ...chat, launched_run_ids: [doneRun] } : chat) })
    })
    await page.route('**/workspace/api/missions', async (route) => {
        const response = await route.fetch()
        const board = (await response.json()) as { missions: { id: string }[] }
        await route.fulfill({ response, json: { ...board, missions: board.missions.map((mission) => mission.id === missionId
            ? { ...mission, runs: [{ run_id: failedRun, flow_name: 'smoke/overview-review.yaml', summary: '', launched_at: '', status: 'failed' }] }
            : mission) } })
    })
    let attention = [{ kind: 'run_gate', id: `gate-${stamp}`, title: 'Approve the deploy?', project_path: project, run_id: waitingRun, updated_at: new Date(Date.now() - 120_000).toISOString() }]
    await page.route('**/workspace/api/attention', (route) => route.fulfill({ json: { items: attention } }))

    await page.goto('/')
    await expect(page.getByTestId('overview-view')).toBeVisible()
    await expect(page.getByTestId('activity-overview')).toHaveAttribute('aria-current', 'page')

    // Needs you: the pending question.
    const needsYou = page.getByTestId('overview-needs-you-item')
    await expect(needsYou).toHaveCount(1)
    await expect(needsYou).toContainText('Approve the deploy?')

    // Finished since: the completed and failed runs, each with where it came from, and the failure counted.
    const finished = page.getByTestId('overview-finished')
    const done = finished.locator(`[data-item-id="${doneRun}"]`)
    const failed = finished.locator(`[data-item-id="${failedRun}"]`)
    await expect(done.locator('[data-mark="completed"]')).toBeVisible()
    await expect(failed.locator('[data-mark="failed"]')).toBeVisible()
    await expect(done.getByTestId('overview-source-link')).toBeVisible()
    await expect(failed.getByTestId('overview-source-link')).toHaveText(`Overview mission ${stamp}`)
    await expect(page.getByTestId('overview-failure-count')).toContainText('1 failed')
    const doneIndex = await finished.getByTestId('overview-finished-item').evaluateAll((rows, id) => rows.findIndex((row) => row.getAttribute('data-item-id') === id), doneRun)
    const failedIndex = await finished.getByTestId('overview-finished-item').evaluateAll((rows, id) => rows.findIndex((row) => row.getAttribute('data-item-id') === id), failedRun)
    expect(doneIndex).toBeLessThan(failedIndex)

    // You were last in: the chat that launched the latest run, with its tally and runs.
    await expect(page.getByTestId('overview-tally')).toContainText('1')
    await page.getByTestId('overview-show-runs').click()
    await expect(page.getByTestId('overview-started-item')).toHaveCount(1)
    await expect(page.getByTestId('overview-started-item')).toContainText('Reviewed the change')
    await page.screenshot({ path: screenshotPath('30a-overview.png'), fullPage: true })

    // Continue opens that chat in the Chats view.
    await page.getByTestId('overview-continue').click()
    await expect(page.getByTestId('activity-chats')).toHaveAttribute('aria-current', 'page')
    await expect(page.locator(`[data-testid="chats-chat-row"][data-conversation-id="${lastChat}"]`)).toHaveAttribute('aria-current', 'true')

    // Something new needs you while you are away: the Overview icon shows a dot until you look.
    await expect(page.getByTestId('activity-overview-dot')).toHaveCount(0)
    attention = [...attention, { kind: 'run_gate', id: `gate-new-${stamp}`, title: 'Another question', project_path: project, run_id: waitingRun, updated_at: new Date().toISOString() }]
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect(page.getByTestId('activity-overview-dot')).toBeVisible()
    await page.getByTestId('activity-overview').click()
    await expect(page.getByTestId('activity-overview-dot')).toHaveCount(0)
    await page.getByTestId('activity-runs').click()
    await expect(page.getByTestId('activity-overview-dot')).toHaveCount(0)

    // While you are away a seen question is answered, then the same run asks again: that is new, so the dot returns.
    const repeated = attention[1]
    attention = [attention[0]]
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect.poll(() => page.evaluate(() => window.localStorage.getItem('spark.overview_seen_attention') ?? '')).not.toContain(repeated.id)
    await expect(page.getByTestId('activity-overview-dot')).toHaveCount(0)
    attention = [attention[0], repeated]
    await page.evaluate(() => window.dispatchEvent(new Event('focus')))
    await expect(page.getByTestId('activity-overview-dot')).toBeVisible()

    // A pending question opens where it can be answered: its run.
    await page.getByTestId('activity-overview').click()
    // The mocked run has no record behind it, so its opening shows as the Runs view fetching it.
    const opened = page.waitForRequest((request) => request.url().includes(`/attractor/pipelines/${waitingRun}`))
    await page.getByTestId('overview-needs-you-item').first().click()
    await expect(page.getByTestId('activity-runs')).toHaveAttribute('aria-current', 'page')
    await opened
})
