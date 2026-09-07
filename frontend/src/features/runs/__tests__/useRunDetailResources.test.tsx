import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useStore } from '@/store'
import { useRunDetailResources } from '../hooks/useRunDetailResources'

const pending: { resolve: (response: Response) => void }[] = []

beforeEach(() => {
    pending.length = 0
    useStore.setState({ runDetailSessionsByRunId: {} })
    vi.stubGlobal('fetch', vi.fn(() => new Promise<Response>((resolve) => {
        pending.push({ resolve })
    })))
})
afterEach(() => vi.unstubAllGlobals())

it.each([
    ['b.txt', 200],
    ['b.txt', 500],
    ['a.txt', 200],
    ['a.txt', 500],
])('keeps the latest preview of %s when an older request finishes with %s', async (path, status) => {
    const { result, unmount } = renderHook(() => useRunDetailResources({ selectedRunId: 'run', manageSync: false }))
    let first!: Promise<void>
    act(() => { first = result.current.viewArtifact({ path: 'a.txt', viewable: true }) })
    unmount()
    const latest = renderHook(() => useRunDetailResources({ selectedRunId: 'run', manageSync: false }))
    let second!: Promise<void>
    act(() => { second = latest.result.current.viewArtifact({ path, viewable: true }) })
    await act(async () => {
        pending[1].resolve(new Response('Latest contents'))
        await second
    })
    await act(async () => {
        pending[0].resolve(new Response('Old response', { status }))
        await first
    })
    expect(latest.result.current.selectedArtifactPath).toBe(path)
    expect(latest.result.current.artifactViewerPayload).toBe('Latest contents')
    expect(latest.result.current.artifactViewerStatus).toBe('ready')
    expect(latest.result.current.artifactViewerError).toBeNull()
})
