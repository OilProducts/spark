import { shallow } from 'zustand/shallow'
import { useCallback, useEffect, useMemo, useRef } from 'react'

import { fetchRunsListValidated, parseRunRecordPayload } from '@/lib/attractorClient'
import { useStore } from '@/store'

import type { RunRecord } from '../model/shared'
import { useRunsTransportReconnectSignal } from '../services/runsTransportReconnect'

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof Error && error.name === 'ApiHttpError') {
        return
    }
    console.error(error)
}

const sortRuns = (runs: RunRecord[]) => {
    return [...runs].sort((left, right) => {
        const leftKey = left.started_at || left.ended_at || ''
        const rightKey = right.started_at || right.ended_at || ''
        return rightKey.localeCompare(leftKey)
    })
}

const mergeRunUpsert = (currentRuns: RunRecord[], nextRun: RunRecord) => {
    const existingIndex = currentRuns.findIndex((run) => run.run_id === nextRun.run_id)
    if (existingIndex === -1) {
        return sortRuns([...currentRuns, nextRun])
    }
    const nextRuns = [...currentRuns]
    nextRuns[existingIndex] = nextRun
    return sortRuns(nextRuns)
}

/** Every project's runs; each run names its own project. */
export function useRunsList({
    selectedRunId,
    manageSync = true,
}: {
    selectedRunId: string | null
    manageSync?: boolean
}) {
    const sort = useStore((state) => state.clientRunPresentation.sort)
    const runsListSession = useStore((state) => state.runsListSession)
    const updateRunsListSession = useStore((state) => state.updateRunsListSession)
    const reconnectSignal = useRunsTransportReconnectSignal(manageSync)
    const requestRefresh = useRef<() => Promise<void>>(async () => {})
    const fetchRuns = useCallback(() => requestRefresh.current(), [])

    useEffect(() => {
        // The run list always loads: the activity bar shows what is running.
        if (!manageSync) {
            return
        }

        let activeRequest: AbortController | null = null
        let pendingRefresh = false
        // Live upserts that arrive while a fetch is out may be newer than its snapshot.
        let liveDuringRequest = new Map<string, RunRecord>()
        let closed = false

        const handleLiveRunUpsert = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            const nextRun = parseRunRecordPayload(detail?.run)
            if (!nextRun) {
                return
            }
            if (activeRequest) liveDuringRequest.set(nextRun.run_id, nextRun)
            useStore.getState().reconcileRunRecord(nextRun.run_id, 'live', nextRun)
            updateRunsListSession({
                runs: mergeRunUpsert(useStore.getState().runsListSession.runs, nextRun),
                status: 'ready',
                error: null,
                streamStatus: 'ready',
                streamError: null,
            }, 'live')
        }

        const refresh = async (): Promise<void> => {
            if (closed) return
            if (activeRequest) {
                pendingRefresh = true
                return
            }
            const request = new AbortController()
            activeRequest = request
            liveDuringRequest = new Map()
            updateRunsListSession({
                status: 'loading',
                error: null,
                streamStatus: 'loading',
                streamError: null,
            })
            let succeeded = false
            try {
                const data = await fetchRunsListValidated(null, request.signal)
                if (activeRequest !== request || request.signal.aborted) return
                updateRunsListSession({
                    runs: [...liveDuringRequest.values()].reduce(mergeRunUpsert, data.runs),
                    fetched: true,
                    status: 'ready',
                    error: null,
                    streamStatus: 'ready',
                    streamError: null,
                })
                succeeded = true
            } catch (err) {
                if (activeRequest !== request || request.signal.aborted) return
                logUnexpectedRunError(err)
                updateRunsListSession({
                    error: 'Unable to load runs',
                    status: 'error',
                    streamStatus: 'degraded',
                    streamError: 'Run history transport is unavailable. Reconnect to retry.',
                })
            } finally {
                if (activeRequest === request) {
                    activeRequest = null
                    const trailingRefresh = succeeded && pendingRefresh
                    pendingRefresh = false
                    if (trailingRefresh) void refresh()
                }
            }
        }
        requestRefresh.current = refresh
        const handleRecovery = () => {
            void refresh()
        }

        window.addEventListener('spark:run-upsert', handleLiveRunUpsert)
        window.addEventListener('spark:runs-overview-resync-required', handleRecovery)

        return () => {
            closed = true
            requestRefresh.current = async () => {}
            activeRequest?.abort()
            activeRequest = null
            pendingRefresh = false
            window.removeEventListener('spark:run-upsert', handleLiveRunUpsert)
            window.removeEventListener('spark:runs-overview-resync-required', handleRecovery)
        }
    }, [
        manageSync,
        updateRunsListSession,
    ])

    useEffect(() => {
        void fetchRuns()
    }, [fetchRuns, reconnectSignal, manageSync])

    const runs = useStore((state) => state.runsListSession.runs)
    const detailSessions = useStore((state) => state.runDetailSessionsByRunId)
    // A run's detail record is fresher than its list row; keep the array while the records are the same.
    const lastDisplayedRuns = useRef<RunRecord[]>([])
    const displayedRuns = useMemo(() => {
        const next = runs.map((run) => detailSessions[run.run_id]?.record ?? run)
        if (!shallow(next, lastDisplayedRuns.current)) lastDisplayedRuns.current = next
        return lastDisplayedRuns.current
    }, [runs, detailSessions])
    const summary = useMemo(() => {
        const total = displayedRuns.length
        const running = displayedRuns.filter(
            (run) => run.status === 'running' || run.status === 'cancel_requested' || run.status === 'abort_requested',
        ).length
        const queued = displayedRuns.filter((run) => run.status === 'queued').length
        return { total, running, queued }
    }, [displayedRuns])

    const selectedRunSummary = useMemo(() => {
        if (!selectedRunId) {
            return null
        }
        return runs.find((run) => run.run_id === selectedRunId) || null
    }, [runs, selectedRunId])

    return {
        error: runsListSession.error,
        fetchRuns,
        isLoading: runsListSession.status === 'loading',
        scopedRuns: sort === 'oldest' ? [...displayedRuns].reverse() : displayedRuns,
        selectedRunSummary,
        status: runsListSession.status,
        streamError: runsListSession.streamError,
        streamStatus: runsListSession.streamStatus,
        summary,
    }
}
