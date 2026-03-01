import { useMemo } from 'react'
import { OctagonX } from 'lucide-react'
import { useStore, type RuntimeStatus } from '@/store'

const STATUS_LABELS: Record<string, string> = {
    running: 'Running',
    abort_requested: 'Canceling…',
    cancel_requested: 'Canceling…',
    aborted: 'Canceled',
    canceled: 'Canceled',
    failed: 'Failed',
    validation_error: 'Validation Error',
    success: 'Complete',
    idle: 'Idle',
}

const CANCEL_ACTION_LABELS: Record<string, string> = {
    running: 'Cancel',
    abort_requested: 'Canceling…',
    cancel_requested: 'Canceling…',
    aborted: 'Canceled',
    canceled: 'Canceled',
    failed: 'Cancel',
    validation_error: 'Cancel',
    success: 'Cancel',
    idle: 'Cancel',
}

const TRANSITION_HINTS: Record<string, string> = {
    abort_requested: 'Cancel requested. Waiting for active node to finish.',
    cancel_requested: 'Cancel requested. Waiting for active node to finish.',
    aborted: 'Run canceled.',
    canceled: 'Run canceled.',
}

const CANCEL_DISABLED_REASONS: Record<string, string> = {
    cancel_requested: 'Cancel already requested for this run.',
    abort_requested: 'Cancel already requested for this run.',
    canceled: 'This run is already canceled.',
    aborted: 'This run is already canceled.',
}

const DEFAULT_CANCEL_DISABLED_REASON = 'Cancel is only available while the run is active.'

const UNSUPPORTED_CONTROL_REASON = 'Pause/Resume is unavailable: backend runtime control API does not expose pause/resume.'
const ACTIVE_RUNTIME_STATUSES = new Set<RuntimeStatus>([
    'running',
    'cancel_requested',
    'abort_requested',
])

export function ExecutionControls() {
    const viewMode = useStore((state) => state.viewMode)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const selectedRunId = useStore((state) => state.selectedRunId)

    const runIsActive = ACTIVE_RUNTIME_STATUSES.has(runtimeStatus)
    const shouldShowFooter = viewMode === 'execution' && (runIsActive || Boolean(selectedRunId))
    const canCancel = runtimeStatus === 'running' && Boolean(selectedRunId)
    const statusLabel = useMemo(
        () => STATUS_LABELS[runtimeStatus] || runtimeStatus,
        [runtimeStatus]
    )
    const cancelActionLabel = CANCEL_ACTION_LABELS[runtimeStatus] || 'Cancel'
    const transitionHint = TRANSITION_HINTS[runtimeStatus] || null
    const cancelDisabledReason = !selectedRunId
        ? 'Run id is still loading.'
        : CANCEL_DISABLED_REASONS[runtimeStatus] || transitionHint || DEFAULT_CANCEL_DISABLED_REASON

    if (!shouldShowFooter) return null

    const requestCancel = async () => {
        if (!selectedRunId) {
            window.alert('Run id is still loading. Please try cancel again in a moment.')
            return
        }
        if (!window.confirm('Cancel this run? It will stop after the active node finishes.')) {
            return
        }
        setRuntimeStatus('cancel_requested')
        try {
            const response = await fetch(`/pipelines/${encodeURIComponent(selectedRunId)}/cancel`, { method: 'POST' })
            if (!response.ok) {
                throw new Error(`cancel failed with HTTP ${response.status}`)
            }
        } catch (error) {
            console.error(error)
            setRuntimeStatus('running')
            window.alert('Failed to request cancel. Check backend logs for details.')
        }
    }

    return (
        <div
            data-testid="execution-footer-controls"
            className="absolute bottom-4 left-1/2 z-20 flex -translate-x-1/2 items-center gap-3 rounded-md border border-border bg-background/90 px-3 py-2 shadow-lg"
        >
            <span data-testid="execution-footer-run-status" className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                {statusLabel}
            </span>
            <div className="h-4 w-px bg-border" />
            {transitionHint && (
                <span className="text-xs text-muted-foreground">{transitionHint}</span>
            )}
            <button
                data-testid="execution-footer-cancel-button"
                onClick={requestCancel}
                disabled={!canCancel}
                title={canCancel ? undefined : cancelDisabledReason}
                className="inline-flex h-8 items-center gap-2 rounded-md bg-destructive px-2 text-xs font-semibold uppercase tracking-wide text-destructive-foreground transition-colors hover:bg-destructive/90 disabled:pointer-events-none disabled:opacity-50"
            >
                <OctagonX className="h-3.5 w-3.5" />
                {cancelActionLabel}
            </button>
            <button
                data-testid="execution-footer-pause-button"
                disabled={true}
                title={UNSUPPORTED_CONTROL_REASON}
                className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground disabled:pointer-events-none disabled:opacity-50"
            >
                Pause
            </button>
            <button
                data-testid="execution-footer-resume-button"
                disabled={true}
                title={UNSUPPORTED_CONTROL_REASON}
                className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground disabled:pointer-events-none disabled:opacity-50"
            >
                Resume
            </button>
            <span
                data-testid="execution-footer-unsupported-controls-reason"
                className="max-w-xs text-xs text-muted-foreground"
            >
                {UNSUPPORTED_CONTROL_REASON}
            </span>
        </div>
    )
}
