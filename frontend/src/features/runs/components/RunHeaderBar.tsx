import { useEffect, useId, useState } from 'react'
import { CircleHelp } from 'lucide-react'

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
    formatTimestamp,
    runRecordTone,
} from '../model/shared'
import { formatCompactCount, formatRunDate, runFlowTitle, runTitle, runTrigger } from '../model/runOverviewModel'
import { openTrigger } from '@/app/useAttention'
import { useStore } from '@/store'
import { normalizeProjectPath } from '@/lib/projectPaths'

// The run's masthead: a muted breadcrumb (project · flow), the title with its status
// pill, one line of facts, and actions. Everything else about the run lives in the
// status item.

const CANCELING_STATUSES = new Set(['cancel_requested', 'abort_requested'])

// Whether the recovery buttons' explanations show, kept as you move between runs.
let actionsHelpShown = false

// The run whose heading takes focus when it shows: a launched run's detail loads after its launcher closes.
let headingFocusRunId: string | null = null
const headingId = (runId: string) => `run-heading-${runId}`

/** Moves focus to a run's heading, now or once the run shows, when the control that had focus goes away. */
export function focusRunHeading(runId: string) {
    const heading = document.getElementById(headingId(runId))
    headingFocusRunId = heading ? null : runId
    heading?.focus()
}

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
    const retry = canRetryRun(run.status)
    const helpId = useId()
    const [helpShown, setHelpShown] = useState(actionsHelpShown)
    useEffect(() => {
        if (headingFocusRunId !== run.run_id) return
        headingFocusRunId = null
        // Unless something else took focus meanwhile.
        if (!document.activeElement || document.activeElement === document.body) focusRunHeading(run.run_id)
    }, [run.run_id])
    const showCancel = canCancelRun(run.status) || CANCELING_STATUSES.has(run.status)
    // Every run reads the same: when it started, then how long it ran or has been going.
    const when = formatRunDate(run.started_at, now)
    const tokens = run.token_usage_breakdown?.total_tokens ?? run.token_usage
    const commit = run.git_commit?.trim()
    const facts = [
        when,
        formatDuration(run.started_at, run.ended_at, run.status, now),
        // A run with no model work yet, such as tools only, has no tokens to report.
        ...(typeof tokens === 'number' && tokens > 0 ? [`${formatCompactCount(tokens)} tokens${run.token_usage_breakdown?.coverage === 'partial' ? ' (partial)' : ''}`] : []),
        ...(commit ? [commit.slice(0, 7)] : []),
    ].filter((fact) => fact && fact !== '—')
    // The title is the one the run list shows. The flow's own title joins the breadcrumb
    // only under a title of the run's own; a title made from the flow's name says it already.
    const title = runTitle(run)
    const flow = flowTitle?.trim() || runFlowTitle(run)
    const showFlow = title !== runFlowTitle(run) && flow.toLowerCase() !== title.toLowerCase()
    const tone = runRecordTone(run)
    const trigger = runTrigger(run)
    const triggerProject = run.project_path || run.working_directory
    // A trigger deleted since is named, not linked: its page is gone.
    const triggerGone = useStore((state) => Boolean(trigger) && state.triggersSession.status === 'ready'
        && !state.triggersSession.triggers.some((entry) => entry.id === trigger?.id && normalizeProjectPath(entry.project_path) === normalizeProjectPath(triggerProject)))

    return (
        <header data-testid="run-summary-panel" className="flex flex-wrap items-start justify-between gap-x-6 gap-y-3">
            {/* At least 20rem before the actions share the row, so a narrow screen wraps them below. */}
            <div className="min-w-[min(100%,20rem)] flex-1 space-y-1">
                {project || showFlow ? (
                    <p className="truncate text-xs text-muted-foreground">
                        {project}
                        {project && showFlow ? ' · ' : ''}
                        {showFlow ? <span data-testid="run-header-flow">{flow}</span> : null}
                    </p>
                ) : null}
                {/* The title wraps, so a phone shows it whole; only a very long one is clamped. */}
                <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                    <h1
                        id={headingId(run.run_id)}
                        data-testid="run-header-title"
                        tabIndex={-1}
                        className="line-clamp-3 min-w-0 rounded-sm text-xl font-semibold tracking-tight text-foreground outline-none [overflow-wrap:anywhere] focus-visible:ring-2 focus-visible:ring-ring"
                        title={`${title} · ${run.run_id}`}
                    >
                        {title}
                    </h1>
                    <StatusPill data-testid="run-header-status" tone={tone}>
                        {formatRunStatusLabel(run)}
                    </StatusPill>
                </div>
                <p
                    data-testid="run-header-facts"
                    className="truncate text-sm text-muted-foreground"
                    title={run.started_at ? `Started ${formatTimestamp(run.started_at)}` : undefined}
                >
                    {facts.join(' · ')}
                    {trigger ? (
                        <span data-testid="run-header-trigger">
                            {facts.length > 0 ? ' · ' : ''}started by trigger{' '}
                            {triggerGone ? <>{trigger.name} (since deleted)</> : (
                                <button type="button" onClick={() => openTrigger(triggerProject, trigger.id)} className="rounded-sm text-foreground underline decoration-muted-foreground/50 underline-offset-4 outline-none hover:decoration-foreground focus-visible:ring-2 focus-visible:ring-ring">{trigger.name}</button>
                            )}
                        </span>
                    ) : null}
                </p>
                {/* The recovery buttons' explanations, shown by the ? button; each button is always described by its sentence. */}
                {inactive ? (
                    <p id={`${helpId}-help`} data-testid="run-header-actions-help" className={helpShown ? 'text-xs text-muted-foreground' : 'sr-only'}>
                        {retry ? <><span id={`${helpId}-retry`}>Retry picks up from the failed step.</span>{' '}</> : null}
                        <span id={`${helpId}-continue`}>
                            {selectedVisitLabel
                                ? `Continue from here starts a new run from ${selectedVisitLabel}, keeping results so far.`
                                : 'Continue from… starts a new run from a step you pick, keeping results so far.'}
                        </span>{' '}
                        <span id={`${helpId}-rerun`}>Re-run starts over with the same inputs.</span>
                    </p>
                ) : null}
            </div>
            <div className="flex flex-wrap items-center gap-2">
                {retry ? (
                    <Button
                        type="button"
                        data-testid="run-summary-retry-button"
                        onClick={() => onRequestRetry(run.run_id, run.status)}
                        aria-describedby={`${helpId}-retry`}
                        title="Pick this run up again from the step where it failed, with the same flow and its results so far"
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
                        aria-describedby={`${helpId}-continue`}
                        title={selectedVisitLabel
                            ? `Start a new run from ${selectedVisitLabel}, reusing this run’s results so far`
                            : 'Start a new run from a step you pick on the graph, reusing this run’s results so far'}
                        variant="outline"
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
                        aria-describedby={`${helpId}-rerun`}
                        title="Start a new run of the flow as it is now, from the beginning, with the same inputs"
                        variant="outline"
                        size="sm"
                    >
                        Re-run
                    </Button>
                ) : null}
                {inactive ? (
                    <Button
                        type="button"
                        data-testid="run-header-actions-help-toggle"
                        aria-label="What these buttons do"
                        title="What these buttons do"
                        aria-expanded={helpShown}
                        aria-controls={`${helpId}-help`}
                        onClick={() => { actionsHelpShown = !helpShown; setHelpShown(!helpShown) }}
                        variant="ghost"
                        size="icon-sm"
                        className="text-muted-foreground"
                    >
                        <CircleHelp />
                    </Button>
                ) : null}
            </div>
        </header>
    )
}
