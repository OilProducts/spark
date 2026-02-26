import { useMemo } from 'react'
import { OctagonX } from 'lucide-react'
import { useStore } from '@/store'

const STATUS_LABELS: Record<string, string> = {
    running: 'Running',
    cancel_requested: 'Canceling…',
    aborted: 'Aborted',
    failed: 'Failed',
    validation_error: 'Validation Error',
    success: 'Complete',
    idle: 'Idle',
}

export function ExecutionControls() {
    const viewMode = useStore((state) => state.viewMode)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const selectedRunId = useStore((state) => state.selectedRunId)

    const canCancel = runtimeStatus === 'running'
    const statusLabel = useMemo(
        () => STATUS_LABELS[runtimeStatus] || runtimeStatus,
        [runtimeStatus]
    )

    if (viewMode !== 'execution' || !selectedRunId) return null

    const requestCancel = async () => {
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        try {
            await fetch(`/pipelines/${encodeURIComponent(selectedRunId)}/cancel`, { method: 'POST' })
        } catch (error) {
            console.error(error)
            window.alert('Failed to request cancel. Check backend logs for details.')
        }
    }

    return (
        <div className="absolute bottom-4 left-1/2 z-20 flex -translate-x-1/2 items-center gap-3 rounded-md border border-border bg-background/90 px-3 py-2 shadow-lg">
            <span className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                {statusLabel}
            </span>
            <div className="h-4 w-px bg-border" />
            <button
                onClick={requestCancel}
                disabled={!canCancel}
                className="inline-flex h-8 items-center gap-2 rounded-md bg-destructive px-2 text-xs font-semibold uppercase tracking-wide text-destructive-foreground transition-colors hover:bg-destructive/90 disabled:pointer-events-none disabled:opacity-50"
            >
                <OctagonX className="h-3.5 w-3.5" />
                Cancel
            </button>
        </div>
    )
}
