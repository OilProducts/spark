import { OctagonX } from 'lucide-react'

interface ExecutionRunStatusStripProps {
    isNarrowViewport: boolean
    humanGatePrompt: string | null
    statusLabel: string
    runIdentityLabel: string
    runtimeOutcome: string | null
    outcomeLabel: string
    terminalStateLabel: string | null
    runtimeOutcomeReasonCode: string | null
    runtimeOutcomeReasonMessage: string | null
    transitionHint: string | null
    canCancel: boolean
    cancelDisabledReason: string
    cancelActionLabel: string
    unsupportedControlReason: string
    onCancel: () => void
}

export function ExecutionRunStatusStrip({
    isNarrowViewport,
    humanGatePrompt,
    statusLabel,
    runIdentityLabel,
    runtimeOutcome,
    outcomeLabel,
    terminalStateLabel,
    runtimeOutcomeReasonCode,
    runtimeOutcomeReasonMessage,
    transitionHint,
    canCancel,
    cancelDisabledReason,
    cancelActionLabel,
    unsupportedControlReason,
    onCancel,
}: ExecutionRunStatusStripProps) {
    return (
        <div className={`flex ${isNarrowViewport ? 'flex-col items-stretch gap-2' : 'flex-wrap items-center gap-3'}`}>
            {humanGatePrompt && (
                <div
                    data-testid="execution-pending-human-gate-banner"
                    className="inline-flex items-center rounded-md border border-amber-500/40 bg-amber-500/10 px-2 py-1 text-[11px] font-semibold text-amber-800"
                >
                    Pending human gate: {humanGatePrompt}
                </div>
            )}
            <span data-testid="execution-footer-run-status" className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                {statusLabel}
            </span>
            <span data-testid="execution-footer-run-identity" className="text-xs font-mono text-muted-foreground">
                {runIdentityLabel}
            </span>
            {runtimeOutcome ? (
                <span data-testid="execution-footer-run-outcome" className="text-xs font-medium text-muted-foreground">
                    Outcome: {outcomeLabel}
                </span>
            ) : null}
            {terminalStateLabel ? (
                <span data-testid="execution-footer-terminal-state" className="text-xs font-medium text-muted-foreground">
                    {terminalStateLabel}
                </span>
            ) : null}
            {runtimeOutcomeReasonCode ? (
                <span data-testid="execution-footer-outcome-reason-code" className="text-xs text-muted-foreground">
                    Reason: {runtimeOutcomeReasonCode}
                </span>
            ) : null}
            {runtimeOutcomeReasonMessage ? (
                <span data-testid="execution-footer-outcome-reason-message" className="text-xs text-muted-foreground">
                    {runtimeOutcomeReasonMessage}
                </span>
            ) : null}
            {transitionHint ? (
                <span className="text-xs text-muted-foreground">{transitionHint}</span>
            ) : null}
            <button
                data-testid="execution-footer-cancel-button"
                onClick={onCancel}
                disabled={!canCancel}
                title={canCancel ? undefined : cancelDisabledReason}
                className="inline-flex h-8 items-center gap-2 rounded-md bg-destructive px-2 text-xs font-semibold uppercase tracking-wide text-destructive-foreground transition-colors hover:bg-destructive/90 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
            >
                <OctagonX className="h-3.5 w-3.5" />
                {cancelActionLabel}
            </button>
            <button
                data-testid="execution-footer-pause-button"
                disabled={true}
                title={unsupportedControlReason}
                className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
            >
                Pause
            </button>
            <button
                data-testid="execution-footer-resume-button"
                disabled={true}
                title={unsupportedControlReason}
                className="inline-flex h-8 items-center rounded-md border border-border px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
            >
                Resume
            </button>
            <span
                data-testid="execution-footer-unsupported-controls-reason"
                className={`text-xs text-muted-foreground ${isNarrowViewport ? 'max-w-none' : 'max-w-xs'}`}
            >
                {unsupportedControlReason}
            </span>
        </div>
    )
}
