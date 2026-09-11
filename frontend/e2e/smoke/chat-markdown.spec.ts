import { expect, test } from '@playwright/test'

const projectPath = '/tmp/markdown-smoke'
const timestamp = '2026-09-10T00:00:00Z'
const header = '$x^2$\n\n$$\n\\frac{1}{2}\n$$\n\n| Wide |\n| --- |\n| ' + 'wide'.repeat(100) + ' |\n\n```text\n' + 'long code '.repeat(100) + '\n```\n\n'
const content = header + '```mermaid\ngraph TD; A['
const segment = { id: 'segment', turn_id: 'assistant', order: 1, kind: 'assistant_message', role: 'assistant', status: 'streaming', timestamp, updated_at: timestamp, content }

test('real chat math and Mermaid render while streaming and contain wide content', async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 900 })
    await page.addInitScript(({ projectPath }) => {
        localStorage.setItem('spark.ui_route_state', JSON.stringify({ viewMode: 'projects', activeProjectPath: projectPath, activeFlow: null }))
    }, { projectPath })
    await page.route('**/workspace/api/projects', (route) => route.fulfill({ json: [{
        project_id: 'markdown', project_path: projectPath, display_name: 'Markdown', created_at: timestamp,
        last_opened_at: timestamp, last_accessed_at: timestamp, is_favorite: false, active_conversation_id: 'markdown',
    }] }))
    await page.route('**/workspace/api/projects/metadata**', (route) => route.fulfill({ json: { name: 'Markdown', directory: projectPath, branch: 'main', commit: 'smoke' } }))
    await page.route('**/workspace/api/projects/conversations**', (route) => route.fulfill({ json: [{
        conversation_id: 'markdown', project_path: projectPath, title: 'Markdown smoke', created_at: timestamp, updated_at: timestamp, revision: 1,
    }] }))
    await page.route('**/workspace/api/conversations/markdown?**', (route) => route.fulfill({ json: {
        schema_version: 4, revision: 1, conversation_id: 'markdown', project_path: projectPath, title: 'Markdown smoke',
        created_at: timestamp, updated_at: timestamp, chat_mode: 'chat',
        turns: [{ id: 'assistant', role: 'assistant', content: '', timestamp, status: 'streaming', kind: 'message' }],
        segments: [segment], event_log: [], flow_run_requests: [], flow_launches: [],
    } }))
    await page.route('**/workspace/api/live/events**', (route) => route.fulfill({ contentType: 'text/event-stream', body: ': smoke\n\n' }))
    await page.goto('/')
    const history = page.getByTestId('project-ai-conversation-history-list')
    await expect(history.locator('.katex').first()).toBeVisible()
    await expect(history.locator('.katex-display')).toBeVisible()
    await expect(history.locator('code.language-mermaid')).toContainText('graph TD; A[')
    await expect(history.getByRole('img', { name: 'Mermaid diagram' })).toHaveCount(0)
    await page.evaluate(({ projectPath, segment, header }) => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', { detail: {
            conversationId: 'markdown', projectPath, type: 'conversation.stream_delta', payload: {
                type: 'stream_delta', conversation_id: 'markdown', turn_id: 'assistant', stream_sequence: 1, base_revision: 1,
                delta_kind: 'segment_delta', segment: { ...segment, content: header + '```mermaid\ngraph TD; A[Streaming]-->B[Rendered]' },
            },
        } }))
    }, { projectPath, segment, header })
    await expect(history.getByRole('img', { name: 'Mermaid diagram' })).toBeVisible({ timeout: 20_000 })
    await expect(history.locator('.markdown-diagram svg')).toContainText('Rendered')
    await expect(history.getByRole('button', { name: 'Copy code' })).toHaveCount(0)
    await history.getByText('Diagram source').click()
    await expect(history.locator('code.language-mermaid')).toBeVisible()
    expect(await history.evaluate((element) => {
        const boundary = element.getBoundingClientRect()
        return [...element.querySelectorAll('.conversation-markdown, pre, .markdown-table, .katex-display, .markdown-diagram')].every((child) => {
            const rect = child.getBoundingClientRect()
            return rect.left >= boundary.left - 1 && rect.right <= boundary.right + 1 && child.clientWidth <= boundary.width
        })
    })).toBe(true)
    expect(await history.locator('pre').first().evaluate((element) => element.scrollWidth > element.clientWidth)).toBe(true)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
