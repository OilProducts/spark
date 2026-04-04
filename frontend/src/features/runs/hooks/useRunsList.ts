import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchRunsListValidated } from '@/lib/attractorClient'
import {
    computeRunMetadataFreshness,
    RUN_METADATA_STALE_AFTER_MS,
} from '@/lib/runMetadataFreshness'
import { useStore } from '@/store'
import type { RunRecord } from '../model/shared'

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
    viewMode,
}: {
    activeProjectPath: string | null
    scopeMode: 'active' | 'all'
    selectedRunId: string | null
    viewMode: string
}) {
    const runRecordOverrides = useStore((state) => state.runRecordOverrides)
    const [runs, setRuns] = useState<RunRecord[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [now, setNow] = useState(() => Date.now())
    const [lastFetchedAtMs, setLastFetchedAtMs] = useState<number | null>(null)
    const [metadataStaleAfterMs] = useState(() => {
        const override = (globalThis as typeof globalThis & { __RUNS_METADATA_STALE_AFTER_MS__?: unknown })
            .__RUNS_METADATA_STALE_AFTER_MS__
        return typeof override === 'number' && Number.isFinite(override) && override > 0
            ? override
            : RUN_METADATA_STALE_AFTER_MS
    })
    const isFetchingRef = useRef(false)
    const usesActiveProjectScope = scopeMode === 'active'

    const fetchRuns = useCallback(async () => {
        if (usesActiveProjectScope && !activeProjectPath) {
            setRuns([])
            setError(null)
            setIsLoading(false)
            return
        }
        if (isFetchingRef.current) return
        isFetchingRef.current = true
        setIsLoading(true)
        setError(null)
        try {
            const data = await fetchRunsListValidated(usesActiveProjectScope ? activeProjectPath : null)
            setRuns(data.runs)
            setLastFetchedAtMs(Date.now())
        } catch (err) {
            logUnexpectedRunError(err)
            setError('Unable to load runs')
        } finally {
            isFetchingRef.current = false
            setIsLoading(false)
        }
    }, [activeProjectPath, usesActiveProjectScope])

    useEffect(() => {
        if (viewMode !== 'runs') return
        void fetchRuns()
    }, [viewMode, fetchRuns])

    useEffect(() => {
        if (viewMode !== 'runs') return
        const refreshInterval = window.setInterval(() => {
            void fetchRuns()
        }, 15_000)
        return () => window.clearInterval(refreshInterval)
    }, [viewMode, fetchRuns])

    useEffect(() => {
        if (viewMode !== 'runs') return
        const interval = window.setInterval(() => setNow(Date.now()), 1000)
        return () => window.clearInterval(interval)
    }, [viewMode])

    const scopedRuns = useMemo(() => {
        if (Object.keys(runRecordOverrides).length === 0) {
            return runs
        }
        return runs.map((run) => {
            const override = runRecordOverrides[run.run_id]
            return override ? { ...run, ...override } : run
        })
    }, [runRecordOverrides, runs])

    const summary = useMemo(() => {
        const total = scopedRuns.length
        const running = scopedRuns.filter(
            (run) => run.status === 'running' || run.status === 'cancel_requested' || run.status === 'abort_requested'
        ).length
        return { total, running }
    }, [scopedRuns])

    const selectedRunSummary = useMemo(() => {
        if (!selectedRunId) return null
        return scopedRuns.find((run) => run.run_id === selectedRunId) || null
    }, [scopedRuns, selectedRunId])

    const metadataFreshness = computeRunMetadataFreshness({
        isLoading,
        lastFetchedAtMs,
        nowMs: now,
        staleAfterMs: metadataStaleAfterMs,
    })

    return {
        error,
        fetchRuns,
        isLoading,
        lastFetchedAtMs,
        metadataFreshness,
        now,
        scopedRuns,
        selectedRunSummary,
        setRuns,
        summary,
        usesActiveProjectScope,
    }
}
