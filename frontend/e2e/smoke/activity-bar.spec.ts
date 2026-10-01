import { mkdirSync, rmSync } from 'node:fs'
import { expect, test, type Page, type TestInfo } from '@playwright/test'
import { createFlowForSmokeTest, deleteFlowAfterSmoke, ensureScreenshotDir, screenshotPath } from '../fixtures/smoke-helpers'

const group = (page: Page, projectPath: string) => page.locator(`[data-testid="chats-project-group"][data-project-path="${projectPath}"]`)

async function seedProjects(page: Page, testInfo: TestInfo) {
    const alpha = testInfo.outputPath('alpha-project')
    const beta = testInfo.outputPath('beta-project')
    for (const project of [alpha, beta]) {
        mkdirSync(project, { recursive: true })
        expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBeTruthy()
    }
    const home = (await (await page.request.get('/workspace/api/projects')).json()).find((entry: { is_default: boolean }) => entry.is_default)
    return { alpha, beta, home: home.project_path as string }
}

const createChat = async (page: Page, projectPath: string, id: string) => {
    expect((await page.request.put(`/workspace/api/conversations/${id}/settings`, { data: { project_path: projectPath, expected_revision: '0' } })).ok()).toBeTruthy()
}

const createMission = async (page: Page, projectPath: string, title: string) => {
    const response = await page.request.post(`/workspace/api/missions?project_path=${encodeURIComponent(projectPath)}`, {
        data: { fields: { title, description: 'Activity bar smoke', archived: false }, actor: 'human' },
    })
    expect(response.ok(), await response.text()).toBeTruthy()
}

const run = (runId: string, projectPath: string, title: string) => ({
    run_id: runId, title, flow_name: 'smoke/activity.yaml', status: 'completed', outcome: 'success',
    working_directory: projectPath, project_path: projectPath, model: 'gpt-5', started_at: '2026-10-01T10:00:00Z',
    ended_at: '2026-10-01T10:05:00Z', last_error: '', token_usage: null,
})

test.beforeAll(() => {
    ensureScreenshotDir()
})

test('every activity-bar view opens its panel with items from more than one project, and no top tabs or project selector remain', async ({ page }, testInfo) => {
    const { alpha, beta, home } = await seedProjects(page, testInfo)
    const stamp = Date.now()
    const flowName = await createFlowForSmokeTest(page, 'activity-bar')
    await createChat(page, home, `conversation-home-${stamp}`)
    await createChat(page, alpha, `conversation-alpha-${stamp}`)
    await createMission(page, alpha, `Alpha mission ${stamp}`)
    await createMission(page, beta, `Beta mission ${stamp}`)
    const trigger = await page.request.post('/workspace/api/triggers', { data: {
        name: `Unregistered target ${stamp}`, enabled: false, source_type: 'schedule',
        action: { flow_name: flowName, project_path: '/tmp/not-a-registered-project', static_context: {} },
        source: { kind: 'interval', interval_seconds: 3600 },
    } })
    expect(trigger.ok(), await trigger.text()).toBeTruthy()
    const alphaTrigger = await page.request.post('/workspace/api/triggers', { data: {
        name: `Alpha target ${stamp}`, enabled: false, source_type: 'schedule',
        action: { flow_name: flowName, project_path: alpha, static_context: {} },
        source: { kind: 'interval', interval_seconds: 3600 },
    } })
    expect(alphaTrigger.ok(), await alphaTrigger.text()).toBeTruthy()
    await page.route('**/attractor/runs', (route) => route.fulfill({ json: { runs: [
        run(`run-alpha-${stamp}`, alpha, 'Alpha run'),
        run(`run-beta-${stamp}`, beta, 'Beta run'),
    ] } }))

    try {
        await page.goto('/')
        await expect(page.getByTestId('activity-bar')).toBeVisible()
        await expect(page.getByTestId('view-mode-tabs')).toHaveCount(0)
        await expect(page.getByTestId('top-nav-project-switcher')).toHaveCount(0)
        await expect(page.getByTestId('top-nav-project-settings-button')).toHaveCount(0)

        // Chats: grouped by project, Home first.
        await expect(page.getByTestId('chats-project-group').first()).toHaveAttribute('data-project-path', home)
        await expect(group(page, home).getByTestId('chats-chat-row')).not.toHaveCount(0)
        await group(page, alpha).getByTestId('chats-project-toggle').click()
        await expect(group(page, alpha).getByTestId('chats-chat-row')).toHaveCount(1)
        await page.screenshot({ path: screenshotPath('20a-activity-chats.png'), fullPage: true })

        // Missions: every project's missions, each naming its project.
        await page.getByTestId('activity-missions').click()
        const missions = page.getByTestId('missions-view').getByTestId('side-panel')
        await expect(missions.getByRole('button', { name: `Alpha mission ${stamp}` })).toBeVisible()
        await expect(missions.getByRole('button', { name: `Beta mission ${stamp}` })).toBeVisible()
        await expect(missions.getByRole('button', { name: `Alpha mission ${stamp}` })).toContainText('alpha-project')
        await expect(missions.getByRole('button', { name: `Beta mission ${stamp}` })).toContainText('beta-project')

        // Runs: every project's runs, with the project on each row and a project filter.
        await page.getByTestId('activity-runs').click()
        const runs = page.getByTestId('runs-view').getByTestId('side-panel')
        await expect(runs.locator(`[data-run-id="run-alpha-${stamp}"]`)).toContainText('alpha-project')
        await expect(runs.locator(`[data-run-id="run-beta-${stamp}"]`)).toContainText('beta-project')
        await page.getByTestId('runs-project-filter').selectOption(beta)
        await expect(runs.locator(`[data-run-id="run-alpha-${stamp}"]`)).toHaveCount(0)
        await expect(runs.locator(`[data-run-id="run-beta-${stamp}"]`)).toBeVisible()
        await page.getByTestId('runs-project-filter').selectOption('')

        // Triggers: each names the project it targets; an unregistered target is marked.
        await page.getByTestId('activity-triggers').click()
        const triggers = page.getByTestId('triggers-view').getByTestId('side-panel')
        await expect(triggers.getByRole('button', { name: new RegExp(`Alpha target ${stamp}`) })).toContainText('Project · alpha-project')
        const unregistered = triggers.getByRole('button', { name: new RegExp(`Unregistered target ${stamp}`) })
        await expect(unregistered).toHaveAttribute('data-unregistered-target', 'true')
        await expect(unregistered).toContainText('Unregistered project · not-a-registered-project')
        // The selected trigger survives a reload, whichever row it is.
        const alphaRow = triggers.getByRole('button', { name: new RegExp(`Alpha target ${stamp}`) })
        for (const row of [unregistered, alphaRow]) {
            await row.click()
            await expect(row).toHaveAttribute('aria-current', 'true')
            await page.reload()
            await page.getByTestId('activity-triggers').click()
            await expect(row).toHaveAttribute('aria-current', 'true')
        }

        // Flows: the installed flows, grouped by folder, with the editor in the main area.
        await page.getByTestId('activity-flows').click()
        await page.getByTestId('flows-view').getByRole('button', { name: flowName }).click()
        await expect(page.getByTestId('flows-view').getByTestId('view-main').getByTestId('editor-run-button')).toBeVisible()
        await page.screenshot({ path: screenshotPath('20b-activity-flows.png'), fullPage: true })
    } finally {
        await deleteFlowAfterSmoke(page, flowName)
        for (const created of [trigger, alphaTrigger]) {
            const { id, revision } = await created.json()
            await page.request.delete(`/workspace/api/triggers/${id}?expected_revision=${encodeURIComponent(revision)}`).catch(() => undefined)
        }
    }
})

test('starting a chat in a project places it under that project and its composer names it', async ({ page }, testInfo) => {
    const { alpha } = await seedProjects(page, testInfo)
    await page.goto('/')
    const alphaGroup = group(page, alpha)
    await alphaGroup.hover()
    await alphaGroup.getByTestId('chats-project-new-chat').click()
    await expect(alphaGroup.getByRole('button', { name: 'Open thread New thread' })).toHaveAttribute('aria-current', 'true')
    await expect(page.getByTestId('chat-composer-project')).toHaveText('Runs and missions started here go to alpha-project')
    await expect(page.getByTestId('chat-project-link')).toHaveText('alpha-project')

    // "+ New" asks which project, Home first then recent projects.
    await page.getByTestId('project-thread-new-button').click()
    const picker = page.getByTestId('new-chat-project-picker')
    await expect(picker).toBeVisible()
    await expect(picker.getByTestId('project-picker-item').first()).toContainText('Home')
    await page.keyboard.press('Escape')
})

test('a new mission and a flow run ask which project, suggesting the last used and the chat\'s project', async ({ page }, testInfo) => {
    const { alpha, beta } = await seedProjects(page, testInfo)
    const flowName = await createFlowForSmokeTest(page, 'activity-run-picker')
    try {
        await page.goto('/')
        // Opening a chat in beta makes it the chat you came from.
        await group(page, beta).hover()
        await group(page, beta).getByTestId('chats-project-new-chat').click()
        await expect(page.getByTestId('chat-composer-project')).toContainText('beta-project')

        await page.getByTestId('activity-missions').click()
        await page.getByRole('button', { name: 'New mission' }).click()
        const missionPicker = page.getByTestId('new-mission-project-picker')
        await expect(missionPicker.getByTestId('project-picker-item').first()).toHaveAttribute('data-project-path', beta)
        await missionPicker.locator(`[data-project-path="${alpha}"]`).click()
        await expect(page.getByTestId('new-mission-project')).toHaveText('New mission in alpha-project')
        await page.getByLabel('Title', { exact: true }).fill('Mission in alpha')
        await page.getByRole('button', { name: 'Create mission' }).click()
        await expect.poll(async () => {
            const missions = await (await page.request.get(`/workspace/api/missions?project_path=${encodeURIComponent(alpha)}`)).json()
            return missions.missions.map((mission: { fields: { title: string } }) => mission.fields.title)
        }).toContain('Mission in alpha')
        await expect(page.getByTestId('missions-view').getByTestId('side-panel').getByRole('button', { name: 'Mission in alpha' })).toContainText('alpha-project')
        // Creating it made alpha the last-used project, for the next mission and after a reload.
        for (const reload of [false, true]) {
            if (reload) {
                await page.reload()
                await page.getByTestId('activity-missions').click()
            }
            await page.getByRole('button', { name: 'New mission' }).click()
            await expect(missionPicker.getByTestId('project-picker-item').first()).toHaveAttribute('data-project-path', alpha)
            await page.keyboard.press('Escape')
        }

        await page.getByTestId('activity-flows').click()
        await page.getByRole('button', { name: flowName }).click()
        await page.getByTestId('editor-run-button').click()
        const runPicker = page.getByTestId('run-flow-project-picker')
        await expect(runPicker.getByTestId('project-picker-item').first()).toHaveAttribute('data-project-path', beta)
        await runPicker.locator(`[data-project-path="${alpha}"]`).click()
        await expect(page.getByTestId('editor-run-panel')).toBeVisible()
        await expect(page.getByTestId('launch-panel-start-button')).toHaveText('Run in alpha-project')
        await page.screenshot({ path: screenshotPath('20c-run-asks-project.png'), fullPage: true })
    } finally {
        await deleteFlowAfterSmoke(page, flowName)
    }
})

test('the project page shows its chats and settings, marks a missing folder, and refuses to remove Home', async ({ page }, testInfo) => {
    const { alpha, beta, home } = await seedProjects(page, testInfo)
    await createChat(page, alpha, `conversation-page-${Date.now()}`)
    rmSync(beta, { recursive: true, force: true })
    await page.goto('/')

    await expect(group(page, beta)).toHaveAttribute('data-folder-missing', 'true')
    await expect(group(page, beta).getByTestId('chats-project-missing')).toHaveText('folder missing')

    await group(page, alpha).getByTestId('chats-project-name').click()
    const projectPage = page.getByTestId('project-page')
    await expect(projectPage.getByTestId('project-page-path')).toHaveText(alpha)
    await expect(projectPage.getByTestId('project-page-chat')).toHaveCount(1)
    await expect(projectPage.getByTestId('project-default-execution-profile')).toBeVisible()
    await expect(projectPage.getByRole('heading', { name: 'Project model defaults' })).toBeVisible()
    await expect(projectPage.getByTestId('project-page-remove')).toBeEnabled()
    // Project model defaults left Settings.
    await page.getByTestId('activity-settings').click()
    await expect(page.getByRole('heading', { name: 'Project model defaults' })).toHaveCount(0)

    await page.getByTestId('activity-chats').click()
    await group(page, home).getByTestId('chats-project-name').click()
    await expect(projectPage.getByTestId('project-page-title')).toHaveText('Home')
    await expect(projectPage.getByTestId('project-page-remove')).toBeDisabled()
    await expect(projectPage.getByTestId('project-page-remove-refused')).toHaveText("Home is Spark's default project and can't be removed.")
    // The server refuses too.
    expect((await page.request.delete(`/workspace/api/projects?project_path=${encodeURIComponent(home)}`)).ok()).toBe(false)
    await page.screenshot({ path: screenshotPath('20d-home-project-page.png'), fullPage: true })

    // Removing an ordinary project works from its page.
    await page.getByTestId('activity-chats').click()
    await group(page, beta).getByTestId('chats-project-name').click()
    await projectPage.getByTestId('project-page-remove').click()
    await page.getByTestId('shared-dialog-confirm').click()
    await expect(group(page, beta)).toHaveCount(0)
})

test('a new mission draft belongs to the project it was started in', async ({ page }, testInfo) => {
    const { alpha, home } = await seedProjects(page, testInfo)
    const stamp = Date.now()
    await page.goto('/')
    await page.getByTestId('activity-missions').click()
    const picker = page.getByTestId('new-mission-project-picker')
    const title = page.getByLabel('Title', { exact: true })
    const startIn = async (projectPath: string) => {
        await page.getByRole('button', { name: 'New mission' }).click()
        await picker.locator(`[data-project-path="${projectPath}"]`).click()
    }

    await startIn(alpha)
    await title.fill(`Alpha draft ${stamp}`)
    // Starting one in Home shows a fresh draft, not alpha's.
    await startIn(home)
    await expect(page.getByTestId('new-mission-project')).toHaveText('New mission in Home')
    await expect(title).toHaveValue('')
    await title.fill(`Home mission ${stamp}`)
    await page.getByRole('button', { name: 'Create mission' }).click()
    await expect(page.getByTestId('missions-view').getByTestId('side-panel').getByRole('button', { name: `Home mission ${stamp}` })).toContainText('Home')

    // Alpha's draft is still there, and saves to alpha.
    await startIn(alpha)
    await expect(title).toHaveValue(`Alpha draft ${stamp}`)
    await page.getByRole('button', { name: 'Create mission' }).click()
    const titles = async (projectPath: string) => (await (await page.request.get(`/workspace/api/missions?project_path=${encodeURIComponent(projectPath)}`)).json())
        .missions.map((mission: { fields: { title: string } }) => mission.fields.title)
    await expect.poll(() => titles(alpha)).toContain(`Alpha draft ${stamp}`)
    expect(await titles(alpha)).not.toContain(`Home mission ${stamp}`)
    expect(await titles(home)).toContain(`Home mission ${stamp}`)
    expect(await titles(home)).not.toContain(`Alpha draft ${stamp}`)
})

test('Settings reopens the category you left it on, across views and reloads', async ({ page }) => {
    await page.goto('/')
    await page.getByTestId('activity-settings').click()
    await page.getByRole('tab', { name: 'Execution' }).click()
    await expect(page.getByRole('tab', { name: 'Execution' })).toHaveAttribute('aria-selected', 'true')
    await page.getByTestId('activity-runs').click()
    await page.getByTestId('activity-settings').click()
    await expect(page.getByRole('tab', { name: 'Execution' })).toHaveAttribute('aria-selected', 'true')
    await page.reload()
    await expect(page.getByRole('tab', { name: 'Execution' })).toHaveAttribute('aria-selected', 'true')
    await page.getByRole('tab', { name: 'Models & accounts' }).click()
})
