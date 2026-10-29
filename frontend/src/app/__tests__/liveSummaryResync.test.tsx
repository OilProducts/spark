import { act, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { WorkspaceLiveEventsController } from '../AppSessionControllers'

class MockEventSource {
    static instances: MockEventSource[] = []
    onopen: (() => void) | null = null
    onmessage: ((event: MessageEvent) => void) | null = null
    onerror: (() => void) | null = null
    constructor(public url: string) { MockEventSource.instances.push(this) }
    close() {}
}

const summary = (state: 'idle' | 'working' | 'stopped') => ({
    conversation_id: 'role-p-coordinator', project_path: '/p', title: 'Coordinator',
    created_at: '2026-10-28T00:00:00Z', updated_at: '2026-10-28T00:00:00Z', revision: 1,
    standing_role: { name: 'coordinator', title: 'Coordinator', state },
})
const roleState = () => useStore.getState().homeConversationCache.summariesByProjectPath['/p']?.[0]?.standing_role?.state
let responses: ((value: unknown) => void)[]

beforeEach(() => {
    MockEventSource.instances = []
    responses = []
    vi.stubGlobal('EventSource', MockEventSource)
    vi.stubGlobal('fetch', vi.fn((url: string) => url.includes('/workspace/api/projects/conversations')
        ? new Promise((resolve) => responses.push((value) => resolve({ ok: true, status: 200, json: async () => value })))
        : Promise.resolve({ ok: true, status: 200, json: async () => ({}) })))
    useStore.setState({
        homeConversationCache: { ...useStore.getState().homeConversationCache, summariesByProjectPath: { '/p': [summary('idle')] } },
    })
})
afterEach(() => { vi.unstubAllGlobals() })

it('refetches held summaries when the stream opens, so a change made while it was closed shows', async () => {
    render(<WorkspaceLiveEventsController />)
    act(() => MockEventSource.instances.at(-1)!.onopen!())
    await waitFor(() => expect(responses).toHaveLength(1))
    // The role stopped while no stream was subscribed; only the refetch carries it.
    await act(async () => responses[0]([summary('stopped')]))
    await waitFor(() => expect(roleState()).toBe('stopped'))
})

it('fetches again when a live upsert overtakes the refetch, instead of applying the older list', async () => {
    render(<WorkspaceLiveEventsController />)
    const source = MockEventSource.instances.at(-1)!
    act(() => source.onopen!())
    await waitFor(() => expect(responses).toHaveLength(1))
    act(() => source.onmessage!({ data: JSON.stringify({
        type: 'conversation.summary_upsert', project_path: '/p',
        resource: { kind: 'conversation_summary', id: 'role-p-coordinator' }, payload: { conversation: summary('working') },
    }) } as MessageEvent))
    expect(roleState()).toBe('working')
    await act(async () => responses[0]([summary('idle')]))
    await waitFor(() => expect(responses).toHaveLength(2))
    expect(roleState()).toBe('working')
    await act(async () => responses[1]([summary('working')]))
    expect(roleState()).toBe('working')
})
