import { selectSelectedRunId } from '@/state/runsSessionSelectors'
import { act, cleanup, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { HistoryRoutingController } from '@/app/AppSessionControllers'
import { buildRunsHash, isRunsHash, parseRunsHash } from '@/app/runsRouting'
import { useStore } from '@/store'

const resetHash = () => {
    window.history.replaceState(null, '', window.location.pathname + window.location.search)
}

describe('runsRouting helpers', () => {
    it('round-trips run and node ids through the hash, encoding special characters', () => {
        expect(parseRunsHash(buildRunsHash('run-123'))).toEqual({ runId: 'run-123', nodeId: null })
        expect(parseRunsHash(buildRunsHash('run-123', 'build-image'))).toEqual({
            runId: 'run-123',
            nodeId: 'build-image',
        })
        expect(parseRunsHash(buildRunsHash('run/slash', 'node space'))).toEqual({
            runId: 'run/slash',
            nodeId: 'node space',
        })
    })

    it('rejects non-runs hashes', () => {
        expect(parseRunsHash('')).toBeNull()
        expect(parseRunsHash('#/settings')).toBeNull()
        expect(parseRunsHash('#/runs/')).toBeNull()
        expect(isRunsHash('#/runs/run-1')).toBe(true)
        expect(isRunsHash('#/home')).toBe(false)
    })
})

describe('HistoryRoutingController', () => {
    beforeEach(() => {
        resetHash()
        useStore.setState({ viewMode: 'home', projectPagePath: null, runDetailSessionsByRunId: {} })
        useStore.getState().setRunsSelectedRunId(null)
    })

    afterEach(() => {
        cleanup()
        resetHash()
    })

    it('applies a deep link hash to the store on mount', async () => {
        window.history.replaceState(null, '', buildRunsHash('run-deep-link', 'deploy'))

        render(<HistoryRoutingController />)

        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('runs')
        })
        expect(selectSelectedRunId(useStore.getState())).toBe('run-deep-link')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-deep-link']?.selectedNodeId,
        ).toBe('deploy')
    })

    it('applies hashchange navigation to the store', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            window.location.hash = buildRunsHash('run-live', 'verify')
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(selectSelectedRunId(useStore.getState())).toBe('run-live')
        })
        expect(useStore.getState().viewMode).toBe('runs')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-live']?.selectedNodeId,
        ).toBe('verify')
    })

    it('opens a view named by a bare hash link', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            window.location.hash = '#/triggers'
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('triggers')
        })
    })

    it('reflects store selection into the hash while on the runs tab', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setRunsSelectedRunId('run-store')
            state.updateRunDetailSession('run-store', { selectedNodeId: 'lint' })
        })

        await waitFor(() => {
            expect(window.location.hash).toBe(buildRunsHash('run-store', 'lint'))
        })
    })

    it('names the view in the hash after leaving the runs tab', async () => {
        render(<HistoryRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setRunsSelectedRunId('run-leaving')
        })
        await waitFor(() => {
            expect(isRunsHash(window.location.hash)).toBe(true)
        })

        act(() => {
            useStore.getState().setViewMode('home')
        })

        await waitFor(() => {
            expect(window.location.hash).toBe('#/home')
        })
    })

    it('adds one history entry per place, so Back returns to the view a run was opened from', async () => {
        useStore.setState({ viewMode: 'overview' })
        render(<HistoryRoutingController />)
        const start = window.history.length

        // The view and its run change in separate updates, as when a run opens from the Overview.
        await act(async () => {
            useStore.getState().setRunsSelectedRunId('run-opened')
            useStore.getState().setViewMode('runs')
        })
        expect(window.history.length).toBe(start + 1)
        // Picking a node rewrites the entry rather than adding one.
        await act(async () => {
            useStore.getState().updateRunDetailSession('run-opened', { selectedNodeId: 'build' })
        })
        expect(window.history.length).toBe(start + 1)
        expect(window.location.hash).toBe(buildRunsHash('run-opened', 'build'))

        act(() => {
            window.history.back()
        })
        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('overview')
        })
        expect(window.location.hash).toBe('#/overview')

        act(() => {
            window.history.forward()
        })
        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('runs')
        })
        expect(selectSelectedRunId(useStore.getState())).toBe('run-opened')
        expect(useStore.getState().runDetailSessionsByRunId['run-opened']?.selectedNodeId).toBe('build')
    })
})
