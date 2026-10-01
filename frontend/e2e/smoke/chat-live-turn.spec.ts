import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test } from '@playwright/test'

// The smoke server runs chat turns on the fake Codex app-server in gated mode
// (see scripts/run-rust-smoke-server.mjs); this test opens its gates.
const tmpRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '.tmp-ui-smoke')
const openGate = (name: string) => {
    mkdirSync(path.join(tmpRoot, 'gates'), { recursive: true })
    writeFileSync(path.join(tmpRoot, 'gates', name), '')
}
const flowYaml = `schema_version: "1"
id: live-turn-smoke
title: Live turn smoke
nodes:
  start:
    kind: start
    label: Start
    config:
      kind: start
  done:
    kind: exit
    label: Done
    config:
      kind: exit
edges:
  - from: start
    to: done
`

test('a long chat turn keeps rendering live updates after it launches a flow partway through', async ({ page }) => {
    test.setTimeout(90_000)
    const projectPath = path.join(tmpRoot, 'projects', 'live-turn')
    mkdirSync(projectPath, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: projectPath } })).ok()).toBe(true)
    // A second project, to leave the chat for mid-turn.
    const otherProject = path.join(tmpRoot, 'projects', 'live-turn-other')
    mkdirSync(otherProject, { recursive: true })
    expect((await page.request.post('/workspace/api/projects/register', { data: { project_path: otherProject } })).ok()).toBe(true)
    expect((await page.request.post('/attractor/api/flows', { data: { name: 'live-turn-smoke.yaml', content: flowYaml } })).ok()).toBe(true)
    await page.addInitScript(({ projectPath }) => {
        localStorage.setItem('spark.ui_route_state', JSON.stringify({ viewMode: 'projects', activeProjectPath: projectPath, activeFlow: null }))
    }, { projectPath })
    await page.goto('/')

    const turnPosted = page.waitForRequest((request) => request.method() === 'POST' && /\/workspace\/api\/conversations\/[^/]+\/turns$/.test(request.url()))
    await page.getByTestId('project-ai-conversation-input').fill('Take your time and launch the flow.')
    await page.getByTestId('project-ai-conversation-send-button').click()
    const conversationId = (await turnPosted).url().split('/').at(-2)!
    const history = page.getByTestId('project-ai-conversation-history-list')
    await expect(history).toContainText('Planning the work.')
    await expect(history).toContainText('Half done.')
    // The Chats icon shows the turn running, and clears when it ends.
    await expect(page.getByTestId('activity-chats-dot')).toHaveAttribute('data-dot', 'running')

    // From here on, content must arrive over the live stream, not by refetching the snapshot.
    let snapshotFetches = 0
    page.on('request', (request) => {
        if (request.method() === 'GET' && request.url().includes(`/workspace/api/conversations/${conversationId}?`)) {
            snapshotFetches += 1
        }
    })

    // The agent launches a flow mid-turn, as the spark CLI does: a real artifact commit that publishes a full snapshot.
    const snapshot = await (await page.request.get(`/workspace/api/conversations/${conversationId}?project_path=${encodeURIComponent(projectPath)}`)).json()
    const launched = await page.request.post('/workspace/api/runs/launch', {
        data: { flow_name: 'live-turn-smoke.yaml', summary: 'Launch the live turn smoke flow.', conversation_handle: snapshot.conversation_handle },
    })
    expect(launched.ok()).toBe(true)
    await expect(history).toContainText('live-turn-smoke.yaml')
    await expect(history).toContainText('Planning the work.')
    await expect(history).toContainText('Half done.')

    openGate('continue')
    await expect(history).toContainText('Half done. Still going.')
    await expect(history).toContainText('Planning the work.')
    await expect(history).toContainText('live-turn-smoke.yaml')
    expect(snapshotFetches).toBe(0)

    // Leaving the chat for one in another project keeps the dot until the turn ends there.
    const otherGroup = page.locator(`[data-testid="chats-project-group"][data-project-path="${otherProject}"]`)
    await otherGroup.hover()
    await otherGroup.getByTestId('chats-project-new-chat').click()
    await expect(page.getByTestId('chat-composer-project')).toContainText('live-turn-other')
    await page.waitForTimeout(3_000)
    await expect(page.getByTestId('activity-chats-dot')).toHaveAttribute('data-dot', 'running')
    await page.getByTestId('activity-runs').click()
    await expect(page.getByTestId('activity-chats-dot')).toHaveAttribute('data-dot', 'running')

    openGate('finish')
    await expect(page.getByTestId('activity-chats-dot')).toHaveCount(0)
    await page.getByTestId('activity-chats').click()
    const liveGroup = page.locator(`[data-testid="chats-project-group"][data-project-path="${projectPath}"]`)
    await liveGroup.getByTestId('chats-chat-row').first().click()
    await expect(history).toContainText('Half done. Still going. All done.')
    await expect(page.getByTestId('project-chat-stop')).toHaveCount(0)
    await expect(page.getByTestId('activity-chats-dot')).toHaveCount(0)
})
