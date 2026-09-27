import { expect, test } from '@playwright/test'
import { createFlowForSmokeTest, ensureScreenshotDir, gotoWithRegisteredProject, screenshotPath, stubProjectMetadata } from '../fixtures/smoke-helpers'

for (const theme of ['light', 'dark']) for (const width of [1440, 390]) {
    test(`model picker chat and inspector ${theme} ${width}`, async ({ page }) => {
        ensureScreenshotDir()
        await page.setViewportSize({ width, height: 900 })
        await stubProjectMetadata(page)
        const flow = await createFlowForSmokeTest(page, 'model-picker')
        await gotoWithRegisteredProject(page, '/tmp/model-picker-smoke')
        await page.route('**/workspace/api/projects/chat-models**', route => route.fulfill({ json: {
            providers: { codex: { status: 'available', error: null } },
            models: [
                { provider: 'codex', id: 'gpt-5.5', display: 'GPT-5.5', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh'] },
                { provider: 'claude-code', id: 'opus', display: 'Opus', is_default: true, default_reasoning_effort: 'high', supported_reasoning_efforts: ['low', 'medium', 'high'] },
                { provider: 'openai', id: 'gpt-5.3-codex-spark', display: 'GPT-5.3 Codex Spark', is_default: true, default_reasoning_effort: 'high', supported_reasoning_efforts: ['low', 'medium', 'high'] },
            ],
        } }))
        await page.evaluate(() => window.dispatchEvent(new Event('spark:codex-connected')))
        await expect(page.getByRole('button', { name: /^Model:/ })).toContainText('GPT-5.5 · Medium')
        await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme)
        const composer = page.getByTestId('project-ai-conversation-surface')
        const trigger = composer.getByRole('button', { name: /^Model:/ })
        await trigger.click()
        await expect(page.getByRole('combobox', { name: 'Search models' })).toBeFocused()
        await page.screenshot({ path: screenshotPath(`model-picker-chat-${theme}-${width}.png`) })
        await page.keyboard.press('Escape')
        await expect(trigger).toBeFocused()
        await page.getByTestId('nav-mode-editor').click()
        await page.getByRole('button', { name: flow, exact: true }).click()
        await page.locator('.react-flow__node').filter({ hasText: 'Ingest Spec' }).click()
        const inspector = page.locator('[data-inspector-scope="node"]')
        await expect(inspector).toBeVisible()
        const advanced = inspector.getByRole('button', { name: 'Show Advanced' })
        if (await advanced.isVisible()) await advanced.click()
        await inspector.getByRole('button', { name: /^Model:/ }).click()
        await expect(page.getByRole('combobox', { name: 'Search models' })).toBeFocused()
        await page.screenshot({ path: screenshotPath(`model-picker-inspector-${theme}-${width}.png`) })
        const picker = page.getByRole('dialog', { name: 'Choose model' })
        const bounds = await picker.boundingBox()
        expect(bounds!.y).toBeGreaterThanOrEqual(0)
        expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(900)
        expect(bounds!.x).toBeGreaterThanOrEqual(0)
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width)
    })
}
