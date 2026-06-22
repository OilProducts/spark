import { useCallback, useEffect, useMemo, type SetStateAction } from 'react'

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

const mergeSelectedRunLiveUpsert = (currentRun: RunRecord, nextRun: RunRecord): RunRecord => ({
    ...currentRun,
    status: nextRun.status,
    outcome: nextRun.outcome !== undefined ? nextRun.outcome : currentRun.outcome,
    outcome_reason_code: nextRun.outcome_reason_code !== undefined
        ? nextRun.outcome_reason_code
        : currentRun.outcome_reason_code,
    outcome_reason_message: nextRun.outcome_reason_message !== undefined
        ? nextRun.outcome_reason_message
        : currentRun.outcome_reason_message,
    ended_at: nextRun.ended_at !== undefined ? nextRun.ended_at : currentRun.ended_at,
    last_error: nextRun.last_error !== undefined ? nextRun.last_error : currentRun.last_error,
    token_usage: nextRun.token_usage !== undefined ? nextRun.token_usage : currentRun.token_usage,
    token_usage_breakdown: nextRun.token_usage_breakdown !== undefined
        ? nextRun.token_usage_breakdown
        : currentRun.token_usage_breakdown,
    estimated_model_cost: nextRun.estimated_model_cost !== undefined
        ? nextRun.estimated_model_cost
        : currentRun.estimated_model_cost,
})

const applySelectedRunLiveUpsert = (nextRun: RunRecord) => {
    const state = useStore.getState()
    const currentRun = state.selectedRunRecord
    if (state.selectedRunId !== nextRun.run_id || currentRun?.run_id !== nextRun.run_id) {
        return
    }
    state.setSelectedRunSnapshot({
        record: mergeSelectedRunLiveUpsert(currentRun, nextRun),
        completedNodes: state.selectedRunCompletedNodes,
        fetchedAtMs: state.selectedRunStatusFetchedAtMs,
    })
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

    const fetchRuns = useCallback(async () => {
        if (!hasRunsSession) {
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
        updateRunsListSession({
            status: 'loading',
            error: null,
        })
        try {
            const data = await fetchRunsListValidated(usesActiveProjectScope ? activeProjectPath : null)
            updateRunsListSession({
                runs: data.runs,
                status: 'ready',
                error: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunsListSession({
                error: 'Unable to load runs',
                status: 'error',
            })
        }
    }, [activeProjectPath, hasRunsSession, updateRunsListSession, usesActiveProjectScope])

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
            applySelectedRunLiveUpsert(nextRun)
            updateRunsListSession({
                runs: mergeRunUpsert(useStore.getState().runsListSession.runs, nextRun),
                status: 'ready',
                error: null,
                streamStatus: 'ready',
                streamError: null,
            })
        }

        const startScopedSync = async () => {
            updateRunsListSession({
                status: 'loading',
                error: null,
                streamStatus: 'loading',
                streamError: null,
            })
            try {
                const data = await fetchRunsListValidated(usesActiveProjectScope ? activeProjectPath : null)
                if (closed) {
                    return
                }
                updateRunsListSession({
                    runs: data.runs,
                    status: 'ready',
                    error: null,
                    streamStatus: 'ready',
                    streamError: null,
                })
            } catch (err) {
                if (closed) {
                    return
                }
                logUnexpectedRunError(err)
                updateRunsListSession({
                    error: 'Unable to load runs',
                    status: 'error',
                    streamStatus: 'degraded',
                    streamError: 'Run history transport is unavailable. Reconnect to retry.',
                })
            }
        }

        void startScopedSync()
        window.addEventListener('spark:run-upsert', handleLiveRunUpsert)
        window.addEventListener('spark:runs-overview-resync-required', fetchRuns)

        return () => {
            closed = true
            window.removeEventListener('spark:run-upsert', handleLiveRunUpsert)
            window.removeEventListener('spark:runs-overview-resync-required', fetchRuns)
        }
    }, [
        activeProjectPath,
        hasRunsSession,
        manageSync,
        reconnectSignal,
        updateRunsListSession,
        usesActiveProjectScope,
    ])

    const summary = useMemo(() => {
        const total = runsListSession.runs.length
        const running = runsListSession.runs.filter(
            (run) => run.status === 'running' || run.status === 'cancel_requested' || run.status === 'abort_requested',
        ).length
        const queued = runsListSession.runs.filter((run) => run.status === 'queued').length
        return { total, running, queued }
    }, [runsListSession.runs])

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
        scopedRuns: runsListSession.runs,
        selectedRunSummary,
        setRuns: (next: SetStateAction<typeof runsListSession.runs>) => {
            updateRunsListSession({
                runs: typeof next === 'function' ? next(useStore.getState().runsListSession.runs) : next,
            })
        },
        status: runsListSession.status,
        streamError: runsListSession.streamError,
        streamStatus: runsListSession.streamStatus,
        summary,
        usesActiveProjectScope,
    }
}
