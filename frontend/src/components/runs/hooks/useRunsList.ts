import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchRunsListValidated } from '@/lib/attractorClient'
import {
    computeRunMetadataFreshness,
    formatRunMetadataLastUpdated,
    RUN_METADATA_STALE_AFTER_MS,
} from '@/lib/runMetadataFreshness'
import type { RunRecord } from '../shared'

const normalizeScopePath = (value: string) => value.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()

const runBelongsToProjectScope = (run: RunRecord, projectPath: string) => {
    const normalizedProjectPath = normalizeScopePath(projectPath)
    const runProjectPath = normalizeScopePath(run.project_path || '')
    const runWorkingDirectory = normalizeScopePath(run.working_directory || '')
    return runProjectPath === normalizedProjectPath || runWorkingDirectory === normalizedProjectPath
}

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof Error && error.name === 'ApiHttpError') {
        return
    }
    console.error(error)
}

export function useRunsList({
    activeProjectPath,
    selectedRunId,
    viewMode,
}: {
    activeProjectPath: string | null
    selectedRunId: string | null
    viewMode: string
}) {
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

    const fetchRuns = useCallback(async () => {
        if (isFetchingRef.current) return
        isFetchingRef.current = true
        setIsLoading(true)
        setError(null)
        try {
            const data = await fetchRunsListValidated()
            setRuns(data.runs)
            setLastFetchedAtMs(Date.now())
        } catch (err) {
            logUnexpectedRunError(err)
            setError('Unable to load runs')
        } finally {
            isFetchingRef.current = false
            setIsLoading(false)
        }
    }, [])

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
        if (!activeProjectPath) return []
        return runs.filter((run) => runBelongsToProjectScope(run, activeProjectPath))
    }, [runs, activeProjectPath])

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
    const metadataFreshnessLabel =
        metadataFreshness === 'refreshing'
            ? 'Refreshing'
            : metadataFreshness === 'stale'
                ? 'Stale'
                : metadataFreshness === 'fresh'
                    ? 'Fresh'
                    : 'Never'
    const metadataFreshnessStyle =
        metadataFreshness === 'stale'
            ? 'border-amber-500/40 bg-amber-500/10 text-amber-800'
            : metadataFreshness === 'fresh'
                ? 'border-green-500/40 bg-green-500/10 text-green-800'
                : 'border-border bg-muted text-muted-foreground'

    return {
        error,
        fetchRuns,
        isLoading,
        lastFetchedAtMs,
        metadataFreshness,
        metadataFreshnessLabel,
        metadataFreshnessStyle,
        now,
        scopedRuns,
        selectedRunSummary,
        setRuns,
        summary,
        updatedAtLabel: formatRunMetadataLastUpdated({ lastFetchedAtMs, nowMs: now }),
    }
}
