import { StatusPill } from '@/components/app/status'
import { Button } from '@/components/ui/button'
import type { RunRecord } from '../model/shared'
import {
    canCancelRun,
    canContinueRun,
    canRetryRun,
    cancelRunActionLabel,
    formatDuration,
    formatRunStatusLabel,
    runStatusTone,
} from '../model/shared'
import { formatCompactCount, formatRunDate, flowTitle as flowTitleFromName, runTitle } from '../model/runOverviewModel'

// The run's masthead: a muted breadcrumb (project · flow), the title with its status
// pill, one line of facts, and actions. Everything else about the run lives in the
// status item.

const CANCELING_STATUSES = new Set(['cancel_requested', 'abort_requested'])

export interface RunHeaderBarProps {
    run: RunRecord
    now: number
    /** The flow's own title from the run's snapshot, when loaded. */
    flowTitle: string | null
    /** The run's project, for the breadcrumb. */
    project?: string | null
    /** The selected visit's label; "Continue from here" restarts there. */
    selectedVisitLabel: string | null
    /** A continuation draft is open below, so its commit button is the screen's one filled button. */
    continuationOpen?: boolean
    onRequestCancel: (runId: string, currentStatus: string) => void
    onRequestRetry: (runId: string, currentStatus: string) => void
    onContinueFromRun: (run: RunRecord) => void
    onRerunRun: (run: RunRecord) => void
}

export function RunHeaderBar({
    run,
    now,
    flowTitle,
    project = null,
    selectedVisitLabel,
    continuationOpen = false,
    onRequestCancel,
    onRequestRetry,
    onContinueFromRun,
    onRerunRun,
}: RunHeaderBarProps) {
    const inactive = canContinueRun(run.status)
    const showCancel = canCancelRun(run.status) || CANCELING_STATUSES.has(run.status)
    // An active run's duration already says how long it has been going.
    const when = inactive ? formatRunDate(run.started_at, now) : ''
    const tokens = run.token_usage_breakdown?.total_tokens ?? run.token_usage
    const commit = run.git_commit?.trim()
    const facts = [
        when,
        formatDuration(run.started_at, run.ended_at, run.status, now),
        ...(typeof tokens === 'number' ? [`${formatCompactCount(tokens)} tokens`] : []),
        ...(commit ? [commit.slice(0, 7)] : []),
    ].filter((fact) => fact && fact !== '—')
    const title = runTitle(run)
    const flow = flowTitle || flowTitleFromName(run.flow_name)
    const showFlow = flow !== title
    // A completed run that reports a failed outcome (a rejected gate) is a warning, not a success.
    const tone = run.status === 'completed' && run.outcome === 'failure' ? 'warning' : runStatusTone(run.status)

    return (
        <header data-testid="run-summary-panel" className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
            <div className="min-w-0 flex-1 space-y-1">
                {project || showFlow ? (
                    <p className="truncate text-xs text-muted-foreground">
                        {project}
                        {project && showFlow ? ' · ' : ''}
                        {showFlow ? <span data-testid="run-header-flow">{flow}</span> : null}
                    </p>
                ) : null}
                <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                    <h3
                        data-testid="run-header-title"
                        className="min-w-0 truncate text-xl font-semibold tracking-tight text-foreground"
                        title={`${title} · ${run.run_id}`}
                    >
                        {title}
                    </h3>
                    <StatusPill data-testid="run-header-status" tone={tone}>
                        {formatRunStatusLabel(run)}
                    </StatusPill>
                </div>
                <p data-testid="run-header-facts" className="truncate text-sm text-muted-foreground">
                    {facts.join(' · ')}
                </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
                {canRetryRun(run.status) ? (
                    <Button
                        type="button"
                        data-testid="run-summary-retry-button"
                        onClick={() => onRequestRetry(run.run_id, run.status)}
                        variant={continuationOpen ? 'outline' : 'default'}
                        size="sm"
                    >
                        Retry
                    </Button>
                ) : null}
                {showCancel ? (
                    <Button
                        type="button"
                        data-testid="run-summary-cancel-button"
                        onClick={() => onRequestCancel(run.run_id, run.status)}
                        disabled={!canCancelRun(run.status)}
                        variant="outline"
                        size="sm"
                    >
                        {cancelRunActionLabel(run.status)}
                    </Button>
                ) : null}
                {inactive ? (
                    <Button
                        type="button"
                        data-testid="run-summary-continue-button"
                        onClick={() => onContinueFromRun(run)}
                        title={selectedVisitLabel
                            ? `Start a new run from ${selectedVisitLabel}`
                            : 'Start a new run from a node you pick on the graph'}
                        variant="ghost"
                        size="sm"
                    >
                        {selectedVisitLabel ? 'Continue from here' : 'Continue from…'}
                    </Button>
                ) : null}
                {inactive ? (
                    <Button
                        type="button"
                        data-testid="run-summary-rerun-button"
                        onClick={() => onRerunRun(run)}
                        title="Launch a new run of this flow with the same inputs"
                        variant="outline"
                        size="sm"
                    >
                        Re-run
                    </Button>
                ) : null}
            </div>
        </header>
    )
}
