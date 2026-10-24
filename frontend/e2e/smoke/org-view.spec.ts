import { mkdirSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { ensureScreenshotDir, gotoWithRegisteredProject, screenshotPath } from '../fixtures/smoke-helpers'

test.beforeAll(() => {
    ensureScreenshotDir()
})

test('the Org view lists bundled roles and flows read-only and drafts a question to the Coordinator', async ({ page }, testInfo) => {
    const project = testInfo.outputPath('org-project')
    mkdirSync(project, { recursive: true })
    await gotoWithRegisteredProject(page, project)

    await page.getByTestId('activity-org').click()
    await expect(page.getByTestId('org-view')).toBeVisible()
    const coordinator = page.locator('[data-testid="org-item"][data-kind="role"]', { hasText: 'coordinator' })
    await expect(coordinator).toContainText('bundled')
    await coordinator.click()
    await expect(page.getByTestId('org-role-standing')).toContainText('Standing in this project')
    await expect(page.getByRole('region', { name: 'History' })).toContainText('No commits in the org repo.')
    await page.screenshot({ path: screenshotPath('org-view.png') })

    await page.getByRole('button', { name: 'Ask the Coordinator' }).click()
    await expect(page.getByTestId('project-ai-conversation-input')).toHaveValue('About the role "coordinator" (bundled, roles/coordinator.md): ')
})
