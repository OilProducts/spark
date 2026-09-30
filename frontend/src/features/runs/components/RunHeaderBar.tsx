import { Button } from '@/components/ui/button'
import type { RunRecord } from '../model/shared'
import {
    canCancelRun,
    canContinueRun,
    canRetryRun,
    cancelRunActionLabel,
    formatDuration,
    formatRunStatusLabel,
    statusToneClassName,
} from '../model/shared'
import { formatCompactCount, formatRunDate, flowTitle as flowTitleFromName, runTitle } from '../model/runOverviewModel'

// The run's masthead: flow and run title, one line of facts, and actions.
// Everything else about the run lives in the status item.

const ATTENTION_STATUSES = new Set(['failed', 'validation_error', 'waiting'])
const CANCELING_STATUSES = new Set(['cancel_requested', 'abort_requested'])

export interface RunHeaderBarProps {
    run: RunRecord
    now: number
    /** The flow's own title from the run's snapshot, when loaded. */
    flowTitle: string | null
    /** The selected visit's label; "Continue from here" restarts there. */
    selectedVisitLabel: string | null
    onRequestCancel: (runId: string, currentStatus: string) => void
    onRequestRetry: (runId: string, currentStatus: string) => void
    onContinueFromRun: (run: RunRecord) => void
    onRerunRun: (run: RunRecord) => void
}

export function RunHeaderBar({
    run,
    now,
    flowTitle,
    selectedVisitLabel,
    onRequestCancel,
    onRequestRetry,
    onContinueFromRun,
    onRerunRun,
}: RunHeaderBarProps) {
    const inactive = canContinueRun(run.status)
    const showCancel = canCancelRun(run.status) || CANCELING_STATUSES.has(run.status)
    // An active run's duration already says how long it has been going.
    const when = inactive ? formatRunDate(run.ended_at || run.started_at, now) : ''
    const tokens = run.token_usage_breakdown?.total_tokens ?? run.token_usage
    const commit = run.git_commit?.trim()
    const facts = [
        `${formatDuration(run.started_at, run.ended_at, run.status, now)}`,
        ...(typeof tokens === 'number' ? [`${formatCompactCount(tokens)} tokens`] : []),
        ...(commit ? [commit.slice(0, 7)] : []),
    ].filter((fact) => fact && fact !== '—')
    const title = runTitle(run)
    const flow = flowTitle || flowTitleFromName(run.flow_name)

    return (
        <header data-testid="run-summary-panel" className="space-y-1 border-b border-border pb-3">
            <div className="flex flex-wrap items-end gap-3">
                <div className="min-w-0 flex-1">
                    {flow !== title ? (
                        <p data-testid="run-header-flow" className="truncate text-xs text-muted-foreground">
                            {flow}
                        </p>
                    ) : null}
                    <h3
                        data-testid="run-header-title"
                        className="truncate text-2xl font-light tracking-tight text-foreground"
                        title={`${title} · ${run.run_id}`}
                    >
                        {title}
                    </h3>
                </div>
                <div className="flex flex-wrap items-center gap-1">
                    {canRetryRun(run.status) ? (
                        <Button
                            type="button"
                            data-testid="run-summary-retry-button"
                            onClick={() => onRequestRetry(run.run_id, run.status)}
                            size="xs"
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
                            size="xs"
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
                            variant="outline"
                            size="xs"
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
                            size="xs"
                        >
                            Re-run
                        </Button>
                    ) : null}
                </div>
            </div>
            <p data-testid="run-header-facts" className="truncate text-xs text-muted-foreground">
                <span
                    data-testid="run-header-status"
                    className={ATTENTION_STATUSES.has(run.status) ? statusToneClassName(run.status) : 'text-foreground'}
                >
                    {formatRunStatusLabel(run)}
                </span>
                {when ? ` ${when}` : ''}
                {facts.map((fact) => ` · ${fact}`).join('')}
            </p>
        </header>
    )
}
