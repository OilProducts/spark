import { Button } from '@/components/ui/button'
import type { RunRecord } from '../model/shared'
import {
    STATUS_LABELS,
    canCancelRun,
    canContinueRun,
    canRetryRun,
    cancelRunActionLabel,
    cancelRunDisabledReason,
    formatDuration,
    formatTimestamp,
} from '../model/shared'
import {
    formatEstimatedModelCostLabel,
    formatOutcomeReason,
    formatTokenCount,
} from '../model/runSummaryFormat'

// The run's masthead: identity, state, and actions on one line; ambient facts
// on a second. Reference detail lives behind the inspector's Details tab.

const STATUS_CHIP_STYLES: Record<string, string> = {
    running: 'border-sky-500/40 bg-sky-500/10 text-sky-700',
    waiting: 'border-amber-500/40 bg-amber-500/10 text-amber-800',
    completed: 'border-green-500/40 bg-green-500/10 text-green-800',
    failed: 'border-destructive/40 bg-destructive/10 text-destructive',
    canceled: 'border-border bg-muted text-muted-foreground',
    aborted: 'border-border bg-muted text-muted-foreground',
    queued: 'border-border bg-muted text-muted-foreground',
}

export interface RunHeaderBarProps {
    run: RunRecord
    now: number
    currentNodeId: string | null
    onRequestCancel: (runId: string, currentStatus: string) => void
    onRequestRetry: (runId: string, currentStatus: string) => void
    onContinueFromRun: (run: RunRecord) => void
    onRerunRun: (run: RunRecord) => void
    onFocusPendingQuestions: (() => void) | null
}

export function RunHeaderBar({
    run,
    now,
    currentNodeId,
    onRequestCancel,
    onRequestRetry,
    onContinueFromRun,
    onRerunRun,
    onFocusPendingQuestions,
}: RunHeaderBarProps) {
    const cancelAvailable = canCancelRun(run.status)
    const continueAvailable = canContinueRun(run.status)
    const rerunAvailable = canContinueRun(run.status)
    const retryAvailable = canRetryRun(run.status)
    const statusChipClass = STATUS_CHIP_STYLES[run.status] ?? 'border-border bg-muted text-muted-foreground'
    const outcomeReason = run.status === 'failed' ? formatOutcomeReason(run) : null
    const facts: Array<{ id: string; label: string; value: string }> = [
        ...(currentNodeId ? [{ id: 'node', label: 'Node', value: currentNodeId }] : []),
        {
            id: 'duration',
            label: 'Duration',
            value: formatDuration(run.started_at, run.ended_at, run.status, now),
        },
        { id: 'tokens', label: 'Tokens', value: formatTokenCount(run.token_usage_breakdown?.total_tokens ?? run.token_usage) },
        { id: 'cost', label: 'Est. cost', value: formatEstimatedModelCostLabel(run) },
        { id: 'started', label: 'Started', value: formatTimestamp(run.started_at) },
    ]

    return (
        <header
            data-testid="run-summary-panel"
            className="space-y-2 rounded-lg border border-border bg-card px-4 py-3"
        >
            <div className="flex flex-wrap items-center gap-3">
                <h3
                    data-testid="run-header-title"
                    className="min-w-0 truncate text-base font-semibold text-foreground"
                    title={run.flow_name || run.run_id}
                >
                    {run.flow_name || run.run_id.slice(0, 8)}
                </h3>
                <span
                    data-testid="run-header-status"
                    className={`inline-flex rounded border px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide ${statusChipClass}`}
                >
                    {STATUS_LABELS[run.status] || run.status}
                </span>
                {run.status === 'waiting' && onFocusPendingQuestions ? (
                    <button
                        type="button"
                        data-testid="run-header-waiting-chip"
                        onClick={onFocusPendingQuestions}
                        className="inline-flex rounded border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-800 hover:bg-amber-500/20"
                    >
                        Waiting for input{currentNodeId ? ` at ${currentNodeId}` : ''} — answer below
                    </button>
                ) : null}
                <span className="font-mono text-xs text-muted-foreground" title={run.run_id}>
                    {run.run_id}
                </span>
                <div className="ml-auto flex flex-wrap items-center gap-2">
                    {rerunAvailable ? (
                        <Button
                            type="button"
                            data-testid="run-summary-rerun-button"
                            onClick={() => onRerunRun(run)}
                            title="Launch a new run of this flow with the same inputs"
                            variant="outline"
                            size="xs"
                        >
                            Re-run
                        </Button>
                    ) : null}
                    {continueAvailable ? (
                        <Button
                            type="button"
                            data-testid="run-summary-continue-button"
                            onClick={() => onContinueFromRun(run)}
                            variant="outline"
                            size="xs"
                        >
                            Continue from node
                        </Button>
                    ) : null}
                    {retryAvailable ? (
                        <Button
                            type="button"
                            data-testid="run-summary-retry-button"
                            onClick={() => onRequestRetry(run.run_id, run.status)}
                            variant="secondary"
                            size="xs"
                        >
                            Retry run
                        </Button>
                    ) : null}
                    <Button
                        type="button"
                        data-testid="run-summary-cancel-button"
                        onClick={() => onRequestCancel(run.run_id, run.status)}
                        disabled={!cancelAvailable}
                        title={cancelAvailable ? undefined : cancelRunDisabledReason(run.status)}
                        variant={cancelAvailable ? 'destructive' : 'outline'}
                        size="xs"
                    >
                        {cancelRunActionLabel(run.status)}
                    </Button>
                </div>
            </div>
            <div
                data-testid="run-header-facts"
                className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground"
            >
                {facts.map((fact) => (
                    <span key={fact.id} data-testid={`run-header-fact-${fact.id}`}>
                        <span className="font-medium text-foreground">{fact.label}:</span> {fact.value}
                    </span>
                ))}
            </div>
            {outcomeReason ? (
                <p
                    data-testid="run-header-failure-reason"
                    className="rounded border border-destructive/40 bg-destructive/10 px-3 py-1.5 text-sm text-destructive"
                >
                    {outcomeReason}
                </p>
            ) : null}
        </header>
    )
}
