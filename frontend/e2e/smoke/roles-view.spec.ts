import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, gotoWithRegisteredProject, screenshotPath } from '../fixtures/smoke-helpers'

// A started role's first turn runs on the fake Codex app-server in gated mode
// (see scripts/run-rust-smoke-server.mjs); this test opens its gates.
const tmpRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', `.tmp-ui-smoke-${process.env.SPARK_UI_SMOKE_PORT}`)
const openGate = (name: string) => {
    mkdirSync(path.join(tmpRoot, 'gates'), { recursive: true })
    writeFileSync(path.join(tmpRoot, 'gates', name), '')
}

test.beforeAll(() => {
    ensureScreenshotDir()
})

test('the Roles view lists roles, standing first, and starts a standing role into its conversation', async ({ page }) => {
    openGate('continue')
    openGate('finish')
    const project = path.join(tmpRoot, 'projects', 'roles-project')
    mkdirSync(project, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: project } })).ok()).toBe(true)
    await page.addInitScript(({ project }) => {
        localStorage.setItem('spark.ui_route_state', JSON.stringify({ viewMode: 'projects', activeProjectPath: project, activeFlow: null }))
    }, { project })
    await page.goto('/')

    await page.getByTestId('activity-roles').click()
    await expect(page.getByTestId('roles-view')).toBeVisible()
    await expect(page.getByRole('button', { name: 'roles-project' })).toBeVisible()
    await expect(page.getByTestId('roles-empty-state')).toBeVisible()
    await expect(page.locator('[data-testid="roles-item"][data-name="implement-change"]')).toHaveCount(0)
    await expect(page.getByTestId('definition-badge')).toHaveCount(0)

    const reviewer = page.getByRole('region', { name: 'Roles', exact: true }).locator('[data-testid="roles-item"][data-name="adversarial-reviewer"]')
    await expect(reviewer).toContainText(/used in \d+ steps? across \d+ flows?/)
    await reviewer.click()
    const detail = page.getByRole('region', { name: 'Role', exact: true })
    await expect(detail.getByRole('region', { name: 'Used in' })).toBeVisible()
    await expect(detail.getByRole('region', { name: 'Prompt' })).toContainText('Adversarial Reviewer')
    await expect(detail.getByRole('button', { name: /^(Start|Open conversation)$/ })).toHaveCount(0)

    // No role exists until it is started: every standing role offers Start.
    const standing = page.getByRole('region', { name: 'Standing' })
    await expect(standing.getByRole('button', { name: /^Open conversation with / })).toHaveCount(0)
    const row = standing.getByTestId('roles-item').first()
    const name = (await row.getAttribute('data-name'))!
    await row.click()
    await expect(detail.getByTestId('role-live-status')).toHaveCount(0)
    await detail.getByRole('button', { name: 'Start' }).click()
    await expect(page.getByTestId('project-ai-conversation-input')).toBeVisible()
    await expect(page.locator(`[data-testid="chats-chat-row"][data-standing-role="${name}"]`)).toHaveAttribute('aria-current', 'true')

    await page.getByTestId('activity-roles').click()
    const started = page.locator(`[data-testid="roles-item"][data-name="${name}"]`)
    await expect(started).toContainText(/(idle|working) · \d+\/\d+ concurrent/)
    await started.click()
    await expect(detail.getByTestId('role-live-status')).toHaveCount(1)
    await page.screenshot({ path: screenshotPath('roles-view.png') })
    await detail.getByRole('button', { name: 'Open conversation' }).click()
    await expect(page.locator(`[data-testid="chats-chat-row"][data-standing-role="${name}"]`)).toHaveAttribute('aria-current', 'true')
})

test('badges leave room for a role\'s title and meta, and for a flow\'s name', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('roles-badges-project')
    mkdirSync(project, { recursive: true })
    let flow = ''
    let standing = { title: '', description: '' }
    await page.route('**/workspace/api/definitions?*', async (route) => {
        const response = await route.fetch()
        const list = await response.json()
        const role = list.items.find((item: { kind: string; standing?: boolean }) => item.kind === 'role' && item.standing)
        role.layer = 'project'
        standing = { title: role.title ?? role.name, description: role.description }
        list.items.find((item: { kind: string; name: string }) => item.kind === 'role' && item.name === 'adversarial-reviewer').layer = 'project_copy'
        const first = list.items.find((item: { kind: string }) => item.kind === 'flow')
        flow = first.name
        first.layer = 'project'
        await route.fulfill({ response, json: list })
    })
    await gotoWithRegisteredProject(page, project)

    await page.getByTestId('activity-roles').click()
    const role = page.getByRole('region', { name: 'Standing' }).getByTestId('roles-item').first()
    const reviewer = page.locator('[data-testid="roles-item"][data-name="adversarial-reviewer"]')
    await expect(role.getByTestId('definition-badge')).toHaveText(['project'])
    await expect(reviewer.getByTestId('definition-badge')).toHaveText(['customized'])
    for (const [row, title, meta] of [[role, standing.title, standing.description], [reviewer, 'Adversarial Reviewer', /^Reviews or verifies/]] as const) {
        const text = row.locator('span.truncate').first()
        await expect(text).toHaveText(title)
        // Not truncated: the title's full text fits its box.
        expect(await text.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
        await expect(row.locator('span.block.truncate').first()).toHaveText(meta)
    }
    await page.screenshot({ path: screenshotPath('roles-view-badges.png') })

    await page.getByTestId('activity-flows').click()
    const flowRow = page.getByTestId('editor-flow-tree').getByRole('button', { name: flow, exact: true })
    await expect(flowRow.getByTestId('definition-badge')).toHaveText(['project'])
    const name = flowRow.locator('span.truncate').first()
    expect(await name.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: screenshotPath('flows-view-badges.png') })
})
