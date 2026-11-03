import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { conversationBatch, conversationPage, turnRow } from '@/test/conversationWire'
import { MissionTranscript } from '../../MissionTranscript'
import type { Mission } from '../../MissionsPanel'
import { useMissionConversation } from '../useMissionConversation'

const at = '2026-03-10T10:00:00Z'
const turn = (index: number) => ({
    id: `turn-${index}`, role: index % 2 ? 'assistant' : 'user', content: `Message ${index}`,
    timestamp: at, kind: 'message', status: 'complete', position: index,
})
const page = (from: number, to: number, revision: number, olderCursor: string | null) => conversationPage({
    conversation_id: 'mission-1', project_path: '/project', title: 'Mission', created_at: at, updated_at: at, revision,
    turns: Array.from({ length: to - from }, (_, offset) => turn(from + offset)), segments: [],
}, { older_cursor: olderCursor })
const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const mission = { id: 'mission-1', project_path: '/project', revision: 1, fields: { title: 'M', description: '', archived: false }, activity: [], status: 'draft' } as unknown as Mission

function Log() {
    const conversation = useMissionConversation(mission, '/project')
    return <MissionTranscript conversation={conversation} project="/project" runs={new Map()} />
}

let fetchMock: ReturnType<typeof vi.fn>
beforeEach(() => {
    fetchMock = vi.fn()
    vi.stubGlobal('fetch', fetchMock)
})
afterEach(() => vi.unstubAllGlobals())

it('loads earlier mission messages through the older-page control and keeps them across reloads', async () => {
    // 60 turns: the newest window holds 50 of them.
    fetchMock.mockImplementation(async (url: string) => {
        if (url.includes('before=cursor-older')) return json(200, page(0, 10, 3, null))
        return json(200, page(10, 60, 3, 'cursor-older'))
    })
    render(<Log />)
    await screen.findByText('Message 59')
    expect(screen.queryByText('Message 1')).toBeNull()
    const control = screen.getByRole('button', { name: 'Load older messages' })

    await userEvent.click(control)
    await screen.findByText('Message 1')
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes('before=cursor-older'))).toBe(true)
    expect(screen.queryByRole('button', { name: 'Load older messages' })).toBeNull()

    // An update catches up on the rows changed since the window; the earlier messages stay.
    fetchMock.mockImplementation(async (url: string) => {
        expect(url).toContain('/changes?')
        expect(url).toContain('since=3')
        return json(200, conversationBatch([turnRow('mission-1', turn(60), '4')], { from_revision: '3', watermark: '4' }))
    })
    act(() => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', { detail: { conversationId: 'mission-1' } }))
    })
    await screen.findByText('Message 60')
    expect(screen.getByText('Message 1')).toBeInTheDocument()
    expect(screen.getByText('Message 10')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Load older messages' })).toBeNull()
})

it('reports a failed older page with Retry, which loads it', async () => {
    let failOlder = true
    fetchMock.mockImplementation(async (url: string) => {
        if (url.includes('before=cursor-older')) {
            return failOlder ? json(500, { detail: 'older page unavailable' }) : json(200, page(0, 10, 3, null))
        }
        return json(200, page(10, 60, 3, 'cursor-older'))
    })
    render(<Log />)
    await userEvent.click(await screen.findByRole('button', { name: 'Load older messages' }))
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('older page unavailable')
    failOlder = false
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }))
    await waitFor(() => expect(screen.getByText('Message 1')).toBeInTheDocument())
    expect(screen.queryByRole('alert')).toBeNull()
})

it('drops an older page whose window a reload replaced while it was in flight', async () => {
    let releaseOlder!: () => void
    let reopened = false
    fetchMock.mockImplementation(async (url: string) => {
        if (url.includes('before=cursor-older')) {
            await new Promise<void>((resolve) => { releaseOlder = resolve })
            return json(200, page(0, 10, 3, null))
        }
        if (url.includes('/changes?')) {
            reopened = true
            return json(409, { code: 'window_reset', reason: 'generation_changed', generation: 'generation-2', watermark: '0' })
        }
        return json(200, reopened ? page(20, 70, 8, 'cursor-new') : page(10, 60, 3, 'cursor-older'))
    })
    render(<Log />)
    await userEvent.click(await screen.findByRole('button', { name: 'Load older messages' }))
    act(() => {
        window.dispatchEvent(new CustomEvent('spark:conversation-live-event', { detail: { conversationId: 'mission-1' } }))
    })
    await screen.findByText('Message 69')
    act(() => releaseOlder())
    await screen.findByRole('button', { name: 'Load older messages' })
    expect(screen.queryByText('Message 1')).toBeNull()
})
