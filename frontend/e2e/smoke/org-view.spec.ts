import { mkdirSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, gotoWithRegisteredProject, screenshotPath } from '../fixtures/smoke-helpers'

test.beforeAll(() => {
    ensureScreenshotDir()
})

test('the Org view lists roles only, standing first, read-only, and drafts a question to the Coordinator', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('org-project')
    mkdirSync(project, { recursive: true })
    await gotoWithRegisteredProject(page, project)

    await page.getByTestId('activity-org').click()
    await expect(page.getByTestId('org-view')).toBeVisible()
    await expect(page.getByTestId('org-status')).toHaveText('No project definitions: all roles are bundled defaults.')
    await expect(page.getByRole('button', { name: 'org-project' })).toBeVisible()
    await expect(page.getByTestId('org-empty-state')).toBeVisible()
    await expect(page.locator('[data-testid="org-item"]').first()).toHaveAttribute('data-name', 'coordinator')
    await expect(page.locator('[data-testid="org-item"][data-name="implement-change"]')).toHaveCount(0)
    await expect(page.getByTestId('org-badge')).toHaveCount(0)

    const reviewer = page.locator('[data-testid="org-item"][data-name="adversarial-reviewer"]')
    await expect(reviewer).toContainText(/used in \d+ steps? across \d+ flows?/)
    await reviewer.click()
    await expect(page.getByRole('region', { name: 'Used in' })).toBeVisible()
    await expect(page.getByRole('region', { name: 'Prompt' })).toContainText('Adversarial Reviewer')
    await page.getByRole('button', { name: 'Ask the Coordinator' }).click()
    await expect(page.getByTestId('project-ai-conversation-input')).toHaveValue('About the role "adversarial-reviewer" (bundled, roles/adversarial-reviewer.md): ')

    await page.getByTestId('activity-org').click()
    await page.locator('[data-testid="org-item"][data-name="coordinator"]').click()
    await expect(page.getByTestId('org-live-status')).toHaveCount(1)
    await expect(page.getByRole('region', { name: 'Prompt' })).toContainText('Coordinator')
    await expect(page.getByRole('button', { name: 'Ask the Coordinator' })).toHaveCount(0)
    await page.screenshot({ path: screenshotPath('org-view.png') })
})

test('badges leave room for a role\'s title and meta, and for a flow\'s name', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('org-badges-project')
    mkdirSync(project, { recursive: true })
    let flow = ''
    await page.route('**/workspace/api/org?*', async (route) => {
        const response = await route.fetch()
        const list = await response.json()
        for (const item of list.items) {
            if (item.kind === 'role' && item.name === 'coordinator') item.layer = 'project'
            if (item.kind === 'role' && item.name === 'adversarial-reviewer') item.layer = 'project_copy'
        }
        const first = list.items.find((item: { kind: string }) => item.kind === 'flow')
        flow = first.name
        first.layer = 'project'
        await route.fulfill({ response, json: { ...list, org_exists: true } })
    })
    await gotoWithRegisteredProject(page, project)

    await page.getByTestId('activity-org').click()
    const coordinator = page.locator('[data-testid="org-item"][data-name="coordinator"]')
    const reviewer = page.locator('[data-testid="org-item"][data-name="adversarial-reviewer"]')
    await expect(coordinator.getByTestId('org-badge')).toHaveText(['customized'])
    await expect(reviewer.getByTestId('org-badge')).toHaveText(['project copy'])
    await expect(page.getByTestId('org-status')).toHaveText('1 customized role · 1 project copy')
    for (const [row, title, meta] of [[coordinator, 'Coordinator', /^idle · 0\/4 concurrent/], [reviewer, 'Adversarial Reviewer', /^Reviews or verifies/]] as const) {
        const text = row.locator('span.truncate').first()
        await expect(text).toHaveText(title)
        // Not truncated: the title's full text fits its box.
        expect(await text.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
        await expect(row.locator('span.block.truncate').first()).toHaveText(meta)
    }
    await page.screenshot({ path: screenshotPath('org-view-badges.png') })

    await page.getByTestId('activity-flows').click()
    const flowRow = page.getByTestId('editor-flow-tree').getByRole('button', { name: flow, exact: true })
    await expect(flowRow.getByTestId('org-badge')).toHaveText(['customized'])
    const name = flowRow.locator('span.truncate').first()
    expect(await name.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true)
    await page.screenshot({ path: screenshotPath('flows-view-badges.png') })
})
