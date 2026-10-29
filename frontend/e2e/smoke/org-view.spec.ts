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
    await expect(page.getByTestId('org-status')).toHaveText('No org repo: all roles are bundled defaults.')
    await expect(page.getByRole('button', { name: 'org-project' })).toBeVisible()
    await expect(page.getByTestId('org-empty-state')).toBeVisible()
    await expect(page.locator('[data-testid="org-item"]').first()).toHaveAttribute('data-name', 'coordinator')
    await expect(page.locator('[data-testid="org-item"][data-name="implement-change"]')).toHaveCount(0)
    await expect(page.getByTestId('org-badge')).toHaveCount(0)

    const reviewer = page.locator('[data-testid="org-item"][data-name="adversarial-reviewer"]')
    await expect(reviewer).toContainText(/used in \d+ steps? across \d+ flows?/)
    await reviewer.click()
    await expect(page.getByRole('region', { name: 'Used in' })).toBeVisible()
    await expect(page.getByText('History')).toBeVisible()
    await page.getByRole('button', { name: 'Ask the Coordinator' }).click()
    await expect(page.getByTestId('project-ai-conversation-input')).toHaveValue('About the role "adversarial-reviewer" (bundled, roles/adversarial-reviewer.md): ')

    await page.getByTestId('activity-org').click()
    await page.locator('[data-testid="org-item"][data-name="coordinator"]').click()
    await expect(page.getByTestId('org-live-status')).toHaveCount(1)
    await expect(page.getByRole('region', { name: 'Prompt' })).toContainText('Coordinator')
    await expect(page.getByRole('button', { name: 'Ask the Coordinator' })).toHaveCount(0)
    await page.screenshot({ path: screenshotPath('org-view.png') })
})
