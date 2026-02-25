import { useEffect, useMemo, useState } from 'react'
import { RefreshCcw } from 'lucide-react'
import { useStore } from '@/store'

interface RunRecord {
    run_id: string
    flow_name: string
    status: string
    result?: string | null
    working_directory: string
    model: string
    started_at: string
    ended_at?: string | null
    last_error?: string
    token_usage?: number | null
}

const STATUS_STYLES: Record<string, string> = {
    running: 'bg-sky-500/15 text-sky-700',
    success: 'bg-green-500/15 text-green-700',
    failed: 'bg-destructive/15 text-destructive',
    aborted: 'bg-amber-500/15 text-amber-700',
    paused: 'bg-amber-500/15 text-amber-700',
    pause_requested: 'bg-amber-500/15 text-amber-700',
    abort_requested: 'bg-amber-500/15 text-amber-700',
    validation_error: 'bg-destructive/15 text-destructive',
}

const STATUS_LABELS: Record<string, string> = {
    pause_requested: 'Pausing',
    abort_requested: 'Aborting',
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
    } else if (status === 'running' || status === 'pause_requested' || status === 'abort_requested') {
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

export function RunsPanel() {
    const viewMode = useStore((state) => state.viewMode)
    const [runs, setRuns] = useState<RunRecord[]>([])
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [now, setNow] = useState(() => Date.now())

    const fetchRuns = async () => {
        setIsLoading(true)
        setError(null)
        try {
            const res = await fetch('/runs')
            if (!res.ok) {
                throw new Error('Failed to load runs')
            }
            const data = await res.json()
            setRuns(Array.isArray(data?.runs) ? data.runs : [])
        } catch (err) {
            console.error(err)
            setError('Unable to load runs')
        } finally {
            setIsLoading(false)
        }
    }

    useEffect(() => {
        if (viewMode !== 'runs') return
        fetchRuns()
    }, [viewMode])

    useEffect(() => {
        if (viewMode !== 'runs') return
        const interval = window.setInterval(() => setNow(Date.now()), 1000)
        return () => window.clearInterval(interval)
    }, [viewMode])

    const summary = useMemo(() => {
        const total = runs.length
        const running = runs.filter((run) => run.status === 'running').length
        return { total, running }
    }, [runs])

    return (
        <div className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-6xl space-y-6">
                <div className="flex items-center justify-between">
                    <div className="space-y-1">
                        <h2 className="text-lg font-semibold">Run History</h2>
                        <p className="text-sm text-muted-foreground">
                            {summary.total} total runs · {summary.running} running
                        </p>
                    </div>
                    <button
                        onClick={fetchRuns}
                        className="inline-flex h-8 items-center gap-2 rounded-md border border-border px-3 text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                    >
                        <RefreshCcw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
                        Refresh
                    </button>
                </div>

                {error && (
                    <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                        {error}
                    </div>
                )}

                <div className="rounded-md border border-border bg-card shadow-sm">
                    <div className="grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px] gap-2 border-b px-4 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        <span>Status</span>
                        <span>Result</span>
                        <span>Flow</span>
                        <span>Started</span>
                        <span>Ended</span>
                        <span>Duration</span>
                        <span>Tokens</span>
                    </div>
                    {runs.length === 0 ? (
                        <div className="px-4 py-8 text-center text-sm text-muted-foreground">
                            No runs yet.
                        </div>
                    ) : (
                        <div className="divide-y">
                            {runs.map((run) => (
                                <div
                                    key={run.run_id}
                                    className="grid grid-cols-[120px_120px_1.5fr_160px_160px_110px_120px] gap-2 px-4 py-3 text-sm"
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
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </div>
        </div>
    )
}
