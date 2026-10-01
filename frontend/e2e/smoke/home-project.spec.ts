import { expect, test } from '@playwright/test'

test('a fresh session chats in the Home project without choosing one', async ({ page }) => {
    const projects = await (await page.request.get('/workspace/api/projects')).json()
    const home = projects.find((project: { is_default: boolean }) => project.is_default)
    expect(home.display_name).toBe('Home')

    await page.goto('/')
    await page.getByTestId('activity-chats').click()
    // Home leads the Chats panel, named Home, and the composer says chats go there.
    const homeGroup = page.getByTestId('chats-project-group').first()
    await expect(homeGroup).toHaveAttribute('data-project-path', home.project_path)
    await expect(homeGroup.getByTestId('chats-project-name')).toHaveText('Home')
    await expect(homeGroup.getByTestId('chats-project-name')).toHaveAttribute('title', home.project_path)
    await expect(page.getByTestId('chat-composer-project')).toContainText('Home')

    const turnPosted = page.waitForResponse((response) => response.request().method() === 'POST' && /\/workspace\/api\/conversations\/[^/]+\/turns$/.test(response.url()))
    await page.getByTestId('project-ai-conversation-input').fill('Hello from Home.')
    await page.getByTestId('project-ai-conversation-send-button').click()
    const turn = await turnPosted
    expect(turn.ok()).toBe(true)
    expect((await turn.json()).project_path).toBe(home.project_path)
    await expect(page.getByTestId('project-ai-conversation-history-list')).toContainText('Planning the work.')
    // The fake agent waits on gates this test leaves shut; interrupt its turn.
    const conversationId = turn.url().split('/').at(-2)
    expect((await page.request.post(`/workspace/api/conversations/${conversationId}/interrupt`, { data: { project_path: home.project_path } })).ok()).toBe(true)
})
