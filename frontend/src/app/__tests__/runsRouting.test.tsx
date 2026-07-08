import { act, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { RunsHashRoutingController } from '@/app/AppSessionControllers'
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

describe('RunsHashRoutingController', () => {
    beforeEach(() => {
        resetHash()
        useStore.setState({
            viewMode: 'home',
            selectedRunId: null,
            runDetailSessionsByRunId: {},
        })
    })

    afterEach(() => {
        resetHash()
    })

    it('applies a deep link hash to the store on mount', async () => {
        window.history.replaceState(null, '', buildRunsHash('run-deep-link', 'deploy'))

        render(<RunsHashRoutingController />)

        await waitFor(() => {
            expect(useStore.getState().viewMode).toBe('runs')
        })
        expect(useStore.getState().selectedRunId).toBe('run-deep-link')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-deep-link']?.selectedNodeId,
        ).toBe('deploy')
    })

    it('applies hashchange navigation to the store', async () => {
        render(<RunsHashRoutingController />)

        act(() => {
            window.location.hash = buildRunsHash('run-live', 'verify')
            window.dispatchEvent(new HashChangeEvent('hashchange'))
        })

        await waitFor(() => {
            expect(useStore.getState().selectedRunId).toBe('run-live')
        })
        expect(useStore.getState().viewMode).toBe('runs')
        expect(
            useStore.getState().runDetailSessionsByRunId['run-live']?.selectedNodeId,
        ).toBe('verify')
    })

    it('reflects store selection into the hash while on the runs tab', async () => {
        render(<RunsHashRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setSelectedRunId('run-store')
            state.updateRunDetailSession('run-store', { selectedNodeId: 'lint' })
        })

        await waitFor(() => {
            expect(window.location.hash).toBe(buildRunsHash('run-store', 'lint'))
        })
    })

    it('clears a stale runs hash when leaving the runs tab', async () => {
        render(<RunsHashRoutingController />)

        act(() => {
            const state = useStore.getState()
            state.setViewMode('runs')
            state.setSelectedRunId('run-leaving')
        })
        await waitFor(() => {
            expect(isRunsHash(window.location.hash)).toBe(true)
        })

        act(() => {
            useStore.getState().setViewMode('home')
        })

        await waitFor(() => {
            expect(isRunsHash(window.location.hash)).toBe(false)
        })
    })
})
