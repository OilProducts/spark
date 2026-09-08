import { useShallow } from 'zustand/react/shallow'
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

export function useRunsList({
    activeProjectPath,
    scopeMode,
    selectedRunId,
    manageSync = true,
}: {
    activeProjectPath: string | null
    scopeMode: 'active' | 'all'
    selectedRunId: string | null
    manageSync?: boolean
}) {
    const viewMode = useStore((state) => state.viewMode)
    const runsListSession = useStore((state) => state.runsListSession)
    const updateRunsListSession = useStore((state) => state.updateRunsListSession)
    const reconnectSignal = useRunsTransportReconnectSignal(manageSync)
    const usesActiveProjectScope = scopeMode === 'active'
    const hasRunsSession =
        viewMode === 'runs'
        || selectedRunId !== null
        || runsListSession.status !== 'idle'
        || runsListSession.runs.length > 0
        || runsListSession.scopeMode !== 'active'

    const requestRefresh = useRef<() => Promise<void>>(async () => {})
    const fetchRuns = useCallback(() => requestRefresh.current(), [])

    useEffect(() => {
        if (!manageSync || !hasRunsSession) {
            return
        }

        if (usesActiveProjectScope && !activeProjectPath) {
            updateRunsListSession({
                runs: [],
                error: null,
                status: 'ready',
                streamStatus: 'idle',
                streamError: null,
            })
            return
        }

        let activeRequest: AbortController | null = null
        let pendingRefresh = false
        let closed = false

        const handleLiveRunUpsert = (event: Event) => {
            const detail = event instanceof CustomEvent ? event.detail : null
            const nextRun = parseRunRecordPayload(detail?.run)
            if (!nextRun) {
                return
            }
            if (usesActiveProjectScope && activeProjectPath && nextRun.project_path !== activeProjectPath) {
                return
            }
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
            updateRunsListSession({
                status: 'loading',
                error: null,
                streamStatus: 'loading',
                streamError: null,
            })
            let succeeded = false
            try {
                const data = await fetchRunsListValidated(
                    usesActiveProjectScope ? activeProjectPath : null,
                    request.signal,
                )
                if (activeRequest !== request || request.signal.aborted) return
                updateRunsListSession({
                    runs: data.runs,
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
        const handleRecovery = (event: Event) => {
            const projectPath = event instanceof CustomEvent ? event.detail?.projectPath : null
            if (usesActiveProjectScope && projectPath && projectPath !== activeProjectPath) return
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
        activeProjectPath,
        hasRunsSession,
        manageSync,
        updateRunsListSession,
        usesActiveProjectScope,
    ])

    useEffect(() => {
        void fetchRuns()
    }, [fetchRuns, reconnectSignal, activeProjectPath, usesActiveProjectScope, manageSync, hasRunsSession])

    const displayedRuns = useStore(useShallow((state) => state.runsListSession.runs.map((run) => state.runDetailSessionsByRunId[run.run_id]?.record ?? run)))
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
        return runsListSession.runs.find((run) => run.run_id === selectedRunId) || null
    }, [runsListSession.runs, selectedRunId])

    return {
        error: runsListSession.error,
        fetchRuns,
        isLoading: runsListSession.status === 'loading',
        scopedRuns: displayedRuns,
        selectedRunSummary,
        status: runsListSession.status,
        streamError: runsListSession.streamError,
        streamStatus: runsListSession.streamStatus,
        summary,
        usesActiveProjectScope,
    }
}
