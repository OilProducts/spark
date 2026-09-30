import { expect, test } from '@playwright/test'

const projectPath = '/tmp/live-turn-smoke'
const timestamp = '2026-09-29T00:00:00Z'
const segment = (id: string, order: number, content: string, overrides: Record<string, unknown> = {}) => ({
    id, turn_id: 'assistant', order, kind: 'assistant_message', role: 'assistant', status: 'streaming',
    timestamp, updated_at: timestamp, content, ...overrides,
})

test('a long chat turn keeps rendering live updates after it launches a flow partway through', async ({ page }) => {
    let snapshotFetches = 0
    await page.addInitScript(({ projectPath }) => {
        localStorage.setItem('spark.ui_route_state', JSON.stringify({ viewMode: 'projects', activeProjectPath: projectPath, activeFlow: null }))
    }, { projectPath })
    await page.route('**/workspace/api/projects', (route) => route.fulfill({ json: [{
        project_id: 'live', project_path: projectPath, display_name: 'Live', created_at: timestamp,
        last_opened_at: timestamp, last_accessed_at: timestamp, is_favorite: false, active_conversation_id: 'live',
    }] }))
    await page.route('**/workspace/api/projects/metadata**', (route) => route.fulfill({ json: { name: 'Live', directory: projectPath, branch: 'main', commit: 'smoke' } }))
    await page.route('**/workspace/api/projects/conversations**', (route) => route.fulfill({ json: [{
        conversation_id: 'live', project_path: projectPath, title: 'New thread', created_at: timestamp, updated_at: timestamp, revision: 1,
    }] }))
    await page.route('**/workspace/api/conversations/live?**', (route) => {
        snapshotFetches += 1
        return route.fulfill({ json: {
            schema_version: 4, revision: 1, conversation_id: 'live', project_path: projectPath, title: 'New thread',
            created_at: timestamp, updated_at: timestamp, chat_mode: 'chat',
            turns: [{ id: 'assistant', role: 'assistant', content: '', timestamp, status: 'streaming', kind: 'message' }],
            segments: [segment('segment-one', 1, 'Planning the work')], event_log: [], flow_run_requests: [], flow_launches: [],
        } })
    })
    await page.route('**/workspace/api/live/events**', (route) => route.fulfill({ contentType: 'text/event-stream', body: ': smoke\n\n' }))
    await page.goto('/')
    const history = page.getByTestId('project-ai-conversation-history-list')
    await expect(history).toContainText('Planning the work')

    const dispatch = (type: string, payload: Record<string, unknown>) => page.evaluate(({ projectPath, type, payload }) => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', {
            detail: { conversationId: 'live', projectPath, type, payload },
        }))
    }, { projectPath, type, payload })
    const delta = (stream_sequence: number, streamed: ReturnType<typeof segment>) => dispatch('conversation.stream_delta', {
        type: 'stream_delta', conversation_id: 'live', turn_id: 'assistant', stream_sequence, base_revision: 1,
        delta_kind: 'segment_delta', segment: streamed,
    })

    await delta(1, segment('segment-one', 1, 'Planning the work'))
    await delta(2, segment('segment-one', 1, 'Planning the work, then launching.'))
    await expect(history).toContainText('Planning the work, then launching.')

    // Mid-turn commit: a flow is launched and the thread title is stored.
    await dispatch('conversation.segment_upsert', {
        type: 'segment_upsert', revision: 3, conversation_id: 'live', project_path: projectPath, title: 'Build the index',
        updated_at: timestamp,
        segment: segment('segment-launch', 2, '', { kind: 'flow_launch', role: 'system', status: 'complete', artifact_id: 'launch-1' }),
        flow_launches: [{
            id: 'launch-1', created_at: timestamp, updated_at: timestamp, flow_name: 'work/build.yaml', summary: 'Build the index',
            project_path: projectPath, conversation_id: 'live', source_turn_id: 'assistant', source_segment_id: 'segment-launch',
            status: 'launched', run_id: 'run-launch',
        }],
    })
    await expect(history).toContainText('Build the index')

    // A replayed older update is still dropped after the commit.
    await delta(1, segment('segment-one', 1, 'Stale text'))

    // Live updates keep flowing even though the conversation revision moved past their base.
    await delta(3, segment('segment-two', 3, 'Flow started; continuing the turn.'))
    await expect(history).toContainText('Flow started; continuing the turn.')
    await delta(4, segment('segment-two', 3, 'Flow started; continuing the turn with more detail.'))
    await expect(history).toContainText('Flow started; continuing the turn with more detail.')
    await expect(history).toContainText('Planning the work, then launching.')
    await expect(history).not.toContainText('Stale text')

    expect(snapshotFetches).toBe(1)
})
