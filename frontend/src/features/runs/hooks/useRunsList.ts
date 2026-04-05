import { useCallback, useEffect, useMemo, useRef, type SetStateAction } from 'react'
import { fetchRunsListValidated } from '@/lib/attractorClient'
import {
    computeRunMetadataFreshness,
} from '@/lib/runMetadataFreshness'
import { useStore } from '@/store'

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof Error && error.name === 'ApiHttpError') {
        return
    }
    console.error(error)
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
    const runRecordOverrides = useStore((state) => state.runRecordOverrides)
    const viewMode = useStore((state) => state.viewMode)
    const runsListSession = useStore((state) => state.runsListSession)
    const updateRunsListSession = useStore((state) => state.updateRunsListSession)
    const isFetchingRef = useRef(false)
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
            })
            return
        }
        if (isFetchingRef.current) {
            return
        }
        isFetchingRef.current = true
        updateRunsListSession({
            status: 'loading',
            error: null,
        })
        try {
            const data = await fetchRunsListValidated(usesActiveProjectScope ? activeProjectPath : null)
            updateRunsListSession({
                runs: data.runs,
                lastFetchedAtMs: Date.now(),
                status: 'ready',
                error: null,
            })
        } catch (err) {
            logUnexpectedRunError(err)
            updateRunsListSession({
                error: 'Unable to load runs',
                status: 'error',
            })
        } finally {
            isFetchingRef.current = false
        }
    }, [activeProjectPath, hasRunsSession, updateRunsListSession, usesActiveProjectScope])

    useEffect(() => {
        if (!manageSync || !hasRunsSession) {
            return
        }
        void fetchRuns()
    }, [fetchRuns, hasRunsSession, manageSync])

    useEffect(() => {
        if (!manageSync || !hasRunsSession) {
            return
        }
        const refreshInterval = window.setInterval(() => {
            void fetchRuns()
        }, 15_000)
        return () => window.clearInterval(refreshInterval)
    }, [fetchRuns, hasRunsSession, manageSync])

    useEffect(() => {
        if (!manageSync || !hasRunsSession) {
            return
        }
        const interval = window.setInterval(() => {
            updateRunsListSession({ nowMs: Date.now() })
        }, 1000)
        return () => window.clearInterval(interval)
    }, [hasRunsSession, manageSync, updateRunsListSession])

    const scopedRuns = useMemo(() => {
        if (Object.keys(runRecordOverrides).length === 0) {
            return runsListSession.runs
        }
        return runsListSession.runs.map((run) => {
            const override = runRecordOverrides[run.run_id]
            return override ? { ...run, ...override } : run
        })
    }, [runRecordOverrides, runsListSession.runs])

    const summary = useMemo(() => {
        const total = scopedRuns.length
        const running = scopedRuns.filter(
            (run) => run.status === 'running' || run.status === 'cancel_requested' || run.status === 'abort_requested',
        ).length
        return { total, running }
    }, [scopedRuns])

    const selectedRunSummary = useMemo(() => {
        if (!selectedRunId) {
            return null
        }
        return scopedRuns.find((run) => run.run_id === selectedRunId) || null
    }, [scopedRuns, selectedRunId])

    const metadataFreshness = computeRunMetadataFreshness({
        isLoading: runsListSession.status === 'loading',
        lastFetchedAtMs: runsListSession.lastFetchedAtMs,
        nowMs: runsListSession.nowMs,
        staleAfterMs: runsListSession.metadataStaleAfterMs,
    })

    return {
        error: runsListSession.error,
        fetchRuns,
        isLoading: runsListSession.status === 'loading',
        lastFetchedAtMs: runsListSession.lastFetchedAtMs,
        metadataFreshness,
        now: runsListSession.nowMs,
        scopedRuns,
        selectedRunSummary,
        setRuns: (next: SetStateAction<typeof runsListSession.runs>) => {
            updateRunsListSession({
                runs: typeof next === 'function' ? next(useStore.getState().runsListSession.runs) : next,
            })
        },
        status: runsListSession.status,
        summary,
        usesActiveProjectScope,
    }
}
