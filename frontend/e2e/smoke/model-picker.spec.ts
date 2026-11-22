import { expect, test } from '@playwright/test'
import { createFlowForSmokeTest, ensureScreenshotDir, gotoWithRegisteredProject, openActivity, screenshotPath, stubProjectMetadata } from '../fixtures/smoke-helpers'

for (const theme of ['light', 'dark']) for (const width of [1440, 390]) {
    test(`model picker chat and inspector ${theme} ${width}`, async ({ page }) => {
        ensureScreenshotDir()
        await page.setViewportSize({ width, height: 900 })
        await stubProjectMetadata(page)
        const flow = await createFlowForSmokeTest(page, 'model-picker', '/tmp/model-picker-smoke')
        await gotoWithRegisteredProject(page, '/tmp/model-picker-smoke')
        await page.route('**/workspace/api/projects/chat-models**', route => route.fulfill({ json: {
            providers: { codex: { status: 'available', error: null } },
            models: [
                { provider: 'codex', id: 'gpt-5.5', display: 'GPT-5.5', is_default: true, default_reasoning_effort: 'medium', supported_reasoning_efforts: ['low', 'medium', 'high', 'xhigh'] },
                { provider: 'claude-code', id: 'opus', display: 'Opus', is_default: true, default_reasoning_effort: 'high', supported_reasoning_efforts: ['low', 'medium', 'high'] },
                { provider: 'openai', id: 'gpt-5.3-codex-spark', display: 'GPT-5.3 Codex Spark', is_default: true, default_reasoning_effort: 'high', supported_reasoning_efforts: ['low', 'medium', 'high'] },
            ],
        } }))
        // Discovery refetches on a settings live event; the stub above replaces the helper's.
        await page.evaluate(() => window.dispatchEvent(new Event('spark:settings-live-event')))
        await expect(page.getByRole('button', { name: /^Model:/ })).toContainText('GPT-5.5')
        await expect(page.getByRole('button', { name: /^Reasoning effort:/ })).toContainText('Medium')
        await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme)
        const composer = page.getByTestId('project-ai-conversation-surface')
        const card = await page.getByTestId('project-ai-conversation-composer').boundingBox()
        expect(card).not.toBeNull()
        const modeTrigger = composer.getByTestId('project-active-chat-mode-badge')
        const trigger = composer.getByRole('button', { name: /^Model:/ })
        const effortTrigger = composer.getByRole('button', { name: /^Reasoning effort:/ })
        const attachButton = composer.getByTestId('project-ai-conversation-attach-button')
        const sendButton = composer.getByTestId('project-ai-conversation-send-button')
        for (const control of [modeTrigger, trigger, effortTrigger, attachButton, sendButton]) {
            const box = await control.boundingBox()
            expect(box).not.toBeNull()
            expect(box!.x).toBeGreaterThanOrEqual(card!.x)
            expect(box!.x + box!.width).toBeLessThanOrEqual(card!.x + card!.width)
        }
        await effortTrigger.click()
        const effortMenu = page.getByRole('menu', { name: 'Reasoning effort' })
        await expect(effortMenu).toBeVisible()
        const effortBounds = await effortMenu.boundingBox()
        expect(effortBounds).not.toBeNull()
        expect(effortBounds!.y).toBeGreaterThanOrEqual(0)
        expect(effortBounds!.y + effortBounds!.height).toBeLessThanOrEqual(900)
        expect(effortBounds!.x).toBeGreaterThanOrEqual(0)
        expect(effortBounds!.x + effortBounds!.width).toBeLessThanOrEqual(width)
        await page.keyboard.press('Escape')
        await trigger.click()
        await expect(page.getByRole('combobox', { name: 'Search models' })).toBeFocused()
        await page.screenshot({ path: screenshotPath(`model-picker-chat-${theme}-${width}.png`) })
        await page.keyboard.press('Escape')
        await expect(trigger).toBeFocused()
        await openActivity(page, 'activity-flows')
        await page.getByTestId('editor-flow-tree').locator(`button[data-flow-name="${flow}"]`).click()
        await page.locator('.react-flow__node').filter({ hasText: 'Ingest Spec' }).click()
        const inspector = page.locator('[data-inspector-scope="node"]')
        await expect(inspector).toBeVisible()
        const advanced = inspector.getByRole('button', { name: 'Show advanced' })
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
