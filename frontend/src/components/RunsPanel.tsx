import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Eye, OctagonX, RefreshCcw } from 'lucide-react'
import { useStore } from '@/store'
import {
    computeRunMetadataFreshness,
    formatRunMetadataLastUpdated,
    RUN_METADATA_STALE_AFTER_MS,
} from '@/lib/runMetadataFreshness'

interface RunRecord {
    run_id: string
    flow_name: string
    status: string
    result?: string | null
    working_directory: string
    project_path?: string
    git_branch?: string | null
    git_commit?: string | null
    model: string
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
}

interface CheckpointResponse {
    pipeline_id: string
    checkpoint: Record<string, unknown>
}

interface ContextResponse {
    pipeline_id: string
    context: Record<string, unknown>
}

interface CheckpointErrorState {
    message: string
    help: string
}

interface ContextErrorState {
    message: string
    help: string
}

interface FormattedContextValue {
    renderedValue: string
    valueType: string
    renderKind: 'scalar' | 'structured'
}

const asRecord = (value: unknown): Record<string, unknown> | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    return value as Record<string, unknown>
}

const asErrorDetail = (value: unknown): string | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    const detail = (value as Record<string, unknown>).detail
    if (typeof detail !== 'string') {
        return null
    }
    const trimmed = detail.trim()
    return trimmed.length > 0 ? trimmed : null
}

const checkpointErrorFromResponse = (status: number, detail: string | null): CheckpointErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'checkpoint unavailable') {
        return {
            message: 'Checkpoint unavailable for this run.',
            help: 'Run may still be in progress or did not persist checkpoint data yet.',
        }
    }
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    return {
        message: `Unable to load checkpoint (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
}

const contextErrorFromResponse = (status: number, detail: string | null): ContextErrorState => {
    const normalizedDetail = detail?.toLowerCase()
    if (status === 404 && normalizedDetail === 'context unavailable') {
        return {
            message: 'Context unavailable for this run.',
            help: 'Run may still be in progress or did not persist context data yet.',
        }
    }
    if (status === 404 && normalizedDetail === 'unknown pipeline') {
        return {
            message: 'Run is no longer available.',
            help: 'The selected run could not be found. Refresh run history and pick a different run.',
        }
    }
    return {
        message: `Unable to load context (HTTP ${status}).`,
        help: detail ? `Backend returned: ${detail}.` : 'Retry, and check backend availability if this keeps failing.',
    }
}

const formatContextValue = (value: unknown): FormattedContextValue => {
    if (value === null) {
        return {
            renderedValue: 'null',
            valueType: 'null',
            renderKind: 'scalar',
        }
    }

    if (typeof value === 'string') {
        return {
            renderedValue: JSON.stringify(value),
            valueType: 'string',
            renderKind: 'scalar',
        }
    }

    if (typeof value === 'number' || typeof value === 'boolean') {
        return {
            renderedValue: String(value),
            valueType: typeof value,
            renderKind: 'scalar',
        }
    }

    const valueType = Array.isArray(value) ? 'array' : 'object'
    let renderedValue = ''
    try {
        renderedValue = JSON.stringify(value, null, 2) ?? String(value)
    } catch {
        renderedValue = String(value)
    }

    return {
        renderedValue,
        valueType,
        renderKind: 'structured',
    }
}

const STATUS_STYLES: Record<string, string> = {
    running: 'bg-sky-500/15 text-sky-700',
    success: 'bg-green-500/15 text-green-700',
    failed: 'bg-destructive/15 text-destructive',
    fail: 'bg-destructive/15 text-destructive',
    aborted: 'bg-amber-500/15 text-amber-700',
    canceled: 'bg-amber-500/15 text-amber-700',
    paused: 'bg-amber-500/15 text-amber-700',
    pause_requested: 'bg-amber-500/15 text-amber-700',
    abort_requested: 'bg-amber-500/15 text-amber-700',
    cancel_requested: 'bg-amber-500/15 text-amber-700',
    validation_error: 'bg-destructive/15 text-destructive',
}

const STATUS_LABELS: Record<string, string> = {
    pause_requested: 'Pausing',
    abort_requested: 'Canceling',
    cancel_requested: 'Canceling',
    aborted: 'Canceled',
    canceled: 'Canceled',
}

const formatTimestamp = (value?: string | null) => {
    if (!value) return '—'
    const date = new Date(value)
    if (Number.isNaN(date.getTime())) return '—'
    return date.toLocaleString()
}

const formatDuration = (start?: string, end?: string | null, status?: string, now?: number) => {
    if (!start) return '—'
    const startMs = Date.parse(start)
    if (!Number.isFinite(startMs)) return '—'
    let endMs: number | null = null
    if (end) {
        const parsed = Date.parse(end)
        if (Number.isFinite(parsed)) endMs = parsed
    } else if (status === 'running' || status === 'pause_requested' || status === 'abort_requested' || status === 'cancel_requested') {
        endMs = now ?? Date.now()
    }
    if (endMs === null) return '—'
    const delta = Math.max(0, endMs - startMs)
    const seconds = Math.floor(delta / 1000)
    const minutes = Math.floor(seconds / 60)
    const hours = Math.floor(minutes / 60)
    const remSeconds = seconds % 60
    const remMinutes = minutes % 60
    if (hours > 0) return `${hours}h ${remMinutes}m`
    if (minutes > 0) return `${minutes}m`
    return `${remSeconds}s`
}

const normalizeScopePath = (value: string) => {
    const trimmed = value.trim()
    if (!trimmed) return ''
    const slashNormalized = trimmed.replace(/\\/g, '/').replace(/\/{2,}/g, '/')
    const prefix = slashNormalized.startsWith('/') ? '/' : ''
    const rawBody = prefix ? slashNormalized.slice(1) : slashNormalized
    const parts = rawBody.split('/').filter((part) => part.length > 0)
    const segments: string[] = []
    for (const part of parts) {
        if (part === '.') {
            continue
        }
        if (part === '..') {
            segments.pop()
            continue
        }
        segments.push(part)
    }
    const normalizedBody = segments.join('/')
    if (!normalizedBody && prefix) {
        return prefix
    }
    return `${prefix}${normalizedBody}`
}

const runBelongsToProjectScope = (run: RunRecord, projectPath: string) => {
    const normalizedProjectPath = normalizeScopePath(projectPath)
    if (!normalizedProjectPath) return false

    const runWorkingDirectory = normalizeScopePath(run.working_directory || '')
    if (!runWorkingDirectory) return false
    if (runWorkingDirectory === normalizedProjectPath) return true

    return runWorkingDirectory.startsWith(`${normalizedProjectPath}/`)
}

export function RunsPanel() {
    const viewMode = useStore((state) => state.viewMode)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const selectedRunId = useStore((state) => state.selectedRunId)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const setViewMode = useStore((state) => state.setViewMode)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const [runs, setRuns] = useState<RunRecord[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [now, setNow] = useState(() => Date.now())
    const [lastFetchedAtMs, setLastFetchedAtMs] = useState<number | null>(null)
    const [checkpointData, setCheckpointData] = useState<CheckpointResponse | null>(null)
    const [isCheckpointLoading, setIsCheckpointLoading] = useState(false)
    const [checkpointError, setCheckpointError] = useState<CheckpointErrorState | null>(null)
    const [contextData, setContextData] = useState<ContextResponse | null>(null)
    const [isContextLoading, setIsContextLoading] = useState(false)
    const [contextError, setContextError] = useState<ContextErrorState | null>(null)
    const [contextSearchQuery, setContextSearchQuery] = useState('')
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
            const res = await fetch('/runs')
            if (!res.ok) {
                throw new Error('Failed to load runs')
            }
            const data = await res.json()
            setRuns(Array.isArray(data?.runs) ? data.runs : [])
            setLastFetchedAtMs(Date.now())
        } catch (err) {
            console.error(err)
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
        if (scopedRuns.length === 0) return null
        return scopedRuns.find((run) => run.run_id === selectedRunId) || scopedRuns[0]
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
            ? 'border-amber-500/40 bg-amber-500/10 text-amber-700'
            : metadataFreshness === 'fresh'
                ? 'border-green-500/40 bg-green-500/10 text-green-700'
                : 'border-border bg-muted text-muted-foreground'

    const fetchCheckpoint = useCallback(async () => {
        if (!selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        setIsCheckpointLoading(true)
        setCheckpointError(null)
        try {
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/checkpoint`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setCheckpointData(null)
                setCheckpointError(checkpointErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.json() as CheckpointResponse
            setCheckpointData(payload)
        } catch (err) {
            console.error(err)
            setCheckpointData(null)
            setCheckpointError({
                message: 'Unable to load checkpoint.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsCheckpointLoading(false)
        }
    }, [selectedRunSummary])

    const fetchContext = useCallback(async () => {
        if (!selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setIsContextLoading(false)
            return
        }
        setIsContextLoading(true)
        setContextError(null)
        try {
            const res = await fetch(`/pipelines/${encodeURIComponent(selectedRunSummary.run_id)}/context`)
            if (!res.ok) {
                let detail: string | null = null
                try {
                    const errorBody = await res.json()
                    detail = asErrorDetail(errorBody)
                } catch {
                    detail = null
                }
                setContextData(null)
                setContextError(contextErrorFromResponse(res.status, detail))
                return
            }
            const payload = await res.json() as ContextResponse
            setContextData(payload)
        } catch (err) {
            console.error(err)
            setContextData(null)
            setContextError({
                message: 'Unable to load context.',
                help: 'Check your network/backend connection and retry.',
            })
        } finally {
            setIsContextLoading(false)
        }
    }, [selectedRunSummary])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setCheckpointData(null)
            setCheckpointError(null)
            setIsCheckpointLoading(false)
            return
        }
        void fetchCheckpoint()
    }, [viewMode, selectedRunSummary, fetchCheckpoint])

    useEffect(() => {
        if (viewMode !== 'runs' || !selectedRunSummary) {
            setContextData(null)
            setContextError(null)
            setIsContextLoading(false)
            setContextSearchQuery('')
            return
        }
        setContextSearchQuery('')
        void fetchContext()
    }, [viewMode, selectedRunSummary, fetchContext])

    const checkpointSnapshot = useMemo(() => asRecord(checkpointData?.checkpoint), [checkpointData])
    const checkpointCurrentNode = useMemo(() => {
        const currentNode = checkpointSnapshot?.current_node
        return typeof currentNode === 'string' && currentNode.trim().length > 0 ? currentNode : '—'
    }, [checkpointSnapshot])
    const checkpointCompletedNodes = useMemo(() => {
        const completedNodes = checkpointSnapshot?.completed_nodes
        if (!Array.isArray(completedNodes)) return '—'
        const normalized = completedNodes
            .map((value) => (typeof value === 'string' ? value.trim() : ''))
            .filter((value) => value.length > 0)
        return normalized.length > 0 ? normalized.join(', ') : '—'
    }, [checkpointSnapshot])
    const checkpointRetryCounters = useMemo(() => {
        const retryCounts = asRecord(checkpointSnapshot?.retry_counts)
        if (!retryCounts) return '—'
        const pairs = Object.entries(retryCounts)
            .filter(([key]) => key.trim().length > 0)
            .map(([key, value]) => {
                if (typeof value === 'number' && Number.isFinite(value)) return `${key}: ${value}`
                if (typeof value === 'string' || typeof value === 'boolean') return `${key}: ${String(value)}`
                return `${key}: ${JSON.stringify(value)}`
            })
        return pairs.length > 0 ? pairs.join(', ') : '—'
    }, [checkpointSnapshot])
    const contextSnapshot = useMemo(() => asRecord(contextData?.context), [contextData])
    const contextRows = useMemo(() => {
        if (!contextSnapshot) return []
        return Object.entries(contextSnapshot)
            .map(([key, value]) => {
                const formatted = formatContextValue(value)
                return { key, ...formatted }
            })
            .sort((a, b) => a.key.localeCompare(b.key))
    }, [contextSnapshot])
    const filteredContextRows = useMemo(() => {
        const normalizedQuery = contextSearchQuery.trim().toLowerCase()
        if (!normalizedQuery) return contextRows
        return contextRows.filter((row) => (
            row.key.toLowerCase().includes(normalizedQuery)
            || row.renderedValue.toLowerCase().includes(normalizedQuery)
        ))
    }, [contextRows, contextSearchQuery])

    const openRun = (run: RunRecord) => {
        setSelectedRunId(run.run_id)
        if (run.flow_name) {
            setActiveFlow(run.flow_name)
        }
        setViewMode('execution')
    }

    const requestCancel = async (runId: string, currentStatus: string) => {
        if (currentStatus !== 'running') {
            return
        }
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        setRuns((current) =>
            current.map((run) => (
                run.run_id === runId
                    ? { ...run, status: 'cancel_requested' }
                    : run
            ))
        )
        try {
            const response = await fetch(`/pipelines/${encodeURIComponent(runId)}/cancel`, { method: 'POST' })
            if (!response.ok) {
                throw new Error(`cancel failed with HTTP ${response.status}`)
            }
            fetchRuns()
        } catch (err) {
            console.error(err)
            setRuns((current) =>
                current.map((run) => (
                    run.run_id === runId
                        ? { ...run, status: currentStatus }
                        : run
                ))
            )
            window.alert('Failed to cancel run')
        }
    }

    return (
        <div data-testid="runs-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-6xl space-y-6">
                <div className="flex items-center justify-between">
                    <div className="space-y-1">
                        <h2 className="text-lg font-semibold">Run History</h2>
                        <p className="text-sm text-muted-foreground">
                            {summary.total} total runs · {summary.running} running
                        </p>
                    </div>
                    <button
                        onClick={() => void fetchRuns()}
                        data-testid="runs-refresh-button"
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-border px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                    >
                        <RefreshCcw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
                        Refresh
                    </button>
                </div>
                <div className="flex flex-wrap items-center gap-2 text-xs">
                    <span
                        data-testid="run-metadata-freshness-indicator"
                        className={`inline-flex items-center rounded-md border px-2 py-1 font-semibold uppercase tracking-wide ${metadataFreshnessStyle}`}
                    >
                        {metadataFreshnessLabel}
                    </span>
                    <span data-testid="run-metadata-last-updated" className="text-muted-foreground">
                        {formatRunMetadataLastUpdated({ lastFetchedAtMs, nowMs: now })}
                    </span>
                </div>
                {metadataFreshness === 'stale' && (
                    <div data-testid="run-metadata-stale-indicator" className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-700">
                        Run metadata may be stale. Refresh to load the latest run status.
                    </div>
                )}

                {error && (
                    <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                        {error}
                    </div>
                )}
                {!activeProjectPath && (
                    <div className="rounded-md border border-border bg-card px-3 py-2 text-sm text-muted-foreground">
                        Select an active project to view run history for that project.
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-summary-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Run Summary</h3>
                            <span className="text-xs text-muted-foreground">{selectedRunSummary.run_id}</span>
                        </div>
                        <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-2">
                            <div data-testid="run-summary-status"><span className="font-medium">Status:</span> {STATUS_LABELS[selectedRunSummary.status] || selectedRunSummary.status}</div>
                            <div data-testid="run-summary-result"><span className="font-medium">Result:</span> {selectedRunSummary.result || '—'}</div>
                            <div data-testid="run-summary-flow-name"><span className="font-medium">Flow:</span> {selectedRunSummary.flow_name || 'Untitled'}</div>
                            <div data-testid="run-summary-started-at"><span className="font-medium">Started:</span> {formatTimestamp(selectedRunSummary.started_at)}</div>
                            <div data-testid="run-summary-ended-at"><span className="font-medium">Ended:</span> {formatTimestamp(selectedRunSummary.ended_at)}</div>
                            <div data-testid="run-summary-duration"><span className="font-medium">Duration:</span> {formatDuration(selectedRunSummary.started_at, selectedRunSummary.ended_at, selectedRunSummary.status, now)}</div>
                            <div data-testid="run-summary-model"><span className="font-medium">Model:</span> {selectedRunSummary.model || 'default model'}</div>
                            <div data-testid="run-summary-working-directory" className="break-all"><span className="font-medium">Working Dir:</span> {selectedRunSummary.working_directory || '—'}</div>
                            <div data-testid="run-summary-project-path" className="break-all"><span className="font-medium">Project Path:</span> {selectedRunSummary.project_path || activeProjectPath || '—'}</div>
                            <div data-testid="run-summary-git-branch"><span className="font-medium">Git Branch:</span> {selectedRunSummary.git_branch || '—'}</div>
                            <div data-testid="run-summary-git-commit"><span className="font-medium">Git Commit:</span> {selectedRunSummary.git_commit || '—'}</div>
                            <div data-testid="run-summary-last-error" className="break-all"><span className="font-medium">Last Error:</span> {selectedRunSummary.last_error || '—'}</div>
                            <div data-testid="run-summary-token-usage"><span className="font-medium">Tokens:</span> {typeof selectedRunSummary.token_usage === 'number' ? selectedRunSummary.token_usage.toLocaleString() : '—'}</div>
                        </div>
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-checkpoint-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Checkpoint</h3>
                            <button
                                onClick={() => void fetchCheckpoint()}
                                data-testid="run-checkpoint-refresh-button"
                                className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                            >
                                {isCheckpointLoading ? 'Refreshing…' : 'Refresh'}
                            </button>
                        </div>
                        {checkpointError && (
                            <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                <div data-testid="run-checkpoint-error">{checkpointError.message}</div>
                                <div data-testid="run-checkpoint-error-help" className="text-xs text-destructive/90">
                                    {checkpointError.help}
                                </div>
                            </div>
                        )}
                        {!checkpointError && checkpointData && (
                            <div className="space-y-3">
                                <div className="grid gap-x-6 gap-y-2 text-sm md:grid-cols-3">
                                    <div data-testid="run-checkpoint-current-node">
                                        <span className="font-medium">Current Node:</span> {checkpointCurrentNode}
                                    </div>
                                    <div data-testid="run-checkpoint-completed-nodes">
                                        <span className="font-medium">Completed Nodes:</span> {checkpointCompletedNodes}
                                    </div>
                                    <div data-testid="run-checkpoint-retry-counters">
                                        <span className="font-medium">Retry Counters:</span> {checkpointRetryCounters}
                                    </div>
                                </div>
                                <pre
                                    data-testid="run-checkpoint-payload"
                                    className="max-h-60 overflow-auto rounded-md border border-border/80 bg-muted/40 p-3 text-xs text-foreground"
                                >
                                    {JSON.stringify(checkpointData, null, 2)}
                                </pre>
                            </div>
                        )}
                    </div>
                )}
                {selectedRunSummary && (
                    <div data-testid="run-context-panel" className="rounded-md border border-border bg-card p-4 shadow-sm">
                        <div className="mb-3 flex items-center justify-between gap-3">
                            <h3 className="text-sm font-semibold uppercase tracking-wide text-muted-foreground">Context</h3>
                            <button
                                onClick={() => void fetchContext()}
                                data-testid="run-context-refresh-button"
                                className="inline-flex h-7 items-center rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                            >
                                {isContextLoading ? 'Refreshing…' : 'Refresh'}
                            </button>
                        </div>
                        <div className="mb-3">
                            <input
                                value={contextSearchQuery}
                                onChange={(event) => setContextSearchQuery(event.target.value)}
                                placeholder="Search context key or value..."
                                data-testid="run-context-search-input"
                                className="h-9 w-full rounded-md border border-border bg-background px-3 text-sm text-foreground shadow-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/30"
                            />
                        </div>
                        {contextError && (
                            <div className="space-y-1 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                                <div data-testid="run-context-error">{contextError.message}</div>
                                <div data-testid="run-context-error-help" className="text-xs text-destructive/90">
                                    {contextError.help}
                                </div>
                            </div>
                        )}
                        {!contextError && (
                            <div className="overflow-hidden rounded-md border border-border/80">
                                <table data-testid="run-context-table" className="w-full table-fixed border-collapse text-sm">
                                    <thead className="bg-muted/50 text-left text-xs uppercase tracking-wide text-muted-foreground">
                                        <tr>
                                            <th className="w-2/5 px-3 py-2 font-semibold">Key</th>
                                            <th className="px-3 py-2 font-semibold">Value</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {filteredContextRows.length > 0 ? (
                                            filteredContextRows.map((row) => (
                                                <tr key={row.key} data-testid="run-context-row" className="border-t border-border/70 align-top">
                                                    <td className="px-3 py-2 font-mono text-xs text-foreground">{row.key}</td>
                                                    <td className="space-x-2 px-3 py-2 font-mono text-xs text-foreground break-all">
                                                        <span
                                                            data-testid="run-context-row-type"
                                                            className="inline-flex rounded border border-border/80 bg-muted/50 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground"
                                                        >
                                                            {row.valueType}
                                                        </span>
                                                        {row.renderKind === 'structured' ? (
                                                            <div data-testid="run-context-row-value">
                                                                <pre
                                                                    data-testid="run-context-row-value-structured"
                                                                    className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-border/70 bg-muted/40 px-2 py-1"
                                                                >
                                                                    {row.renderedValue}
                                                                </pre>
                                                            </div>
                                                        ) : (
                                                            <span data-testid="run-context-row-value">
                                                                <span data-testid="run-context-row-value-scalar">{row.renderedValue}</span>
                                                            </span>
                                                        )}
                                                    </td>
                                                </tr>
                                            ))
                                        ) : (
                                            <tr>
                                                <td data-testid="run-context-empty" colSpan={2} className="px-3 py-4 text-sm text-muted-foreground">
                                                    No context entries match the current search.
                                                </td>
                                            </tr>
                                        )}
                                    </tbody>
                                </table>
                            </div>
                        )}
                    </div>
                )}

                <div className="rounded-md border border-border bg-card shadow-sm">
                    <div className="grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px_170px] gap-2 border-b px-4 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        <span>Status</span>
                        <span>Result</span>
                        <span>Flow</span>
                        <span>Started</span>
                        <span>Ended</span>
                        <span>Duration</span>
                        <span>Tokens</span>
                        <span>Actions</span>
                    </div>
                    {scopedRuns.length === 0 ? (
                        <div className="px-4 py-8 text-center text-sm text-muted-foreground">
                            {activeProjectPath ? 'No runs for the active project yet.' : 'No runs yet.'}
                        </div>
                    ) : (
                        <div className="divide-y">
                            {scopedRuns.map((run) => (
                                (() => {
                                    const canCancel = run.status === 'running'
                                    const cancelActionLabel = canCancel ? 'Cancel' : (
                                        run.status === 'cancel_requested' || run.status === 'abort_requested'
                                            ? 'Canceling…'
                                            : run.status === 'canceled' || run.status === 'aborted'
                                                ? 'Canceled'
                                                : 'Cancel'
                                    )
                                    const cancelDisabledReason =
                                        run.status === 'cancel_requested' || run.status === 'abort_requested'
                                            ? 'Cancel already requested for this run.'
                                            : run.status === 'canceled' || run.status === 'aborted'
                                                ? 'This run is already canceled.'
                                                : 'Cancel is only available while the run is active.'

                                    return (
                                        <div
                                            key={run.run_id}
                                            className={`grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px_170px] gap-2 px-4 py-3 text-sm ${
                                                selectedRunId === run.run_id ? 'bg-muted/40' : ''
                                            }`}
                                        >
                                            <span
                                                className={`inline-flex h-6 items-center rounded-md px-2 text-[11px] font-semibold uppercase tracking-wide ${
                                                    STATUS_STYLES[run.status] || 'bg-muted text-muted-foreground'
                                                }`}
                                            >
                                                {STATUS_LABELS[run.status] || run.status}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {run.result || '—'}
                                            </span>
                                            <div>
                                                <div className="font-medium text-foreground">
                                                    {run.flow_name || 'Untitled'}
                                                </div>
                                                <div className="text-[11px] text-muted-foreground">
                                                    {run.model || 'default model'} · {run.run_id}
                                                </div>
                                            </div>
                                            <span className="text-xs text-muted-foreground">
                                                {formatTimestamp(run.started_at)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {formatTimestamp(run.ended_at)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {formatDuration(run.started_at, run.ended_at, run.status, now)}
                                            </span>
                                            <span className="text-xs text-muted-foreground">
                                                {typeof run.token_usage === 'number' ? run.token_usage.toLocaleString() : '—'}
                                            </span>
                                            <div className="flex items-center gap-2">
                                                <button
                                                    onClick={() => openRun(run)}
                                                    className="inline-flex h-7 items-center gap-1.5 rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                                                >
                                                    <Eye className="h-3.5 w-3.5" />
                                                    Open
                                                </button>
                                                <button
                                                    onClick={() => requestCancel(run.run_id, run.status)}
                                                    disabled={!canCancel}
                                                    title={canCancel ? undefined : cancelDisabledReason}
                                                    className="inline-flex h-7 items-center gap-1.5 rounded-md bg-destructive px-2 text-[11px] font-semibold text-destructive-foreground hover:bg-destructive/90 disabled:pointer-events-none disabled:opacity-50"
                                                >
                                                    <OctagonX className="h-3.5 w-3.5" />
                                                    {cancelActionLabel}
                                                </button>
                                            </div>
                                        </div>
                                    )
                                })()
                            ))}
                        </div>
                    )}
                </div>
            </div>
        </div>
    )
}
