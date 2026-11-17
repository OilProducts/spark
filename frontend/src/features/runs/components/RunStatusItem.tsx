import { useState, type ComponentProps, type ReactNode } from 'react'
import { ChevronRight } from 'lucide-react'

import type { PipelineResultResponse } from '@/lib/attractorClient'
import type { TokenUsageBucketResponse } from '@/lib/api/attractorApi'
import { InlineError } from '@/components/app/inline-error'
import { StatusDot } from '@/components/app/status'
import { SectionHeading } from '@/components/app/section-heading'
import { Well } from '@/components/app/well'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { cn } from '@/lib/utils'
import {
    formatDuration,
    formatRunStatusLabel,
    formatTimestamp,
    type ArtifactListEntry,
    type RunRecord,
} from '../model/shared'
import {
    formatEstimatedCost,
    formatEstimatedModelCostLabel,
    formatEstimatedModelCostNote,
    formatLineage,
    formatOutcomeReason,
    formatTokenCount,
    hasExecutionLockMetadata,
    shouldShowWorkingDirectoryDifference,
} from '../model/runSummaryFormat'
import {
    classifyRunFailure,
    describeFailure,
    errorVisits,
    runFailureMessage,
    type RunStatusKind,
} from '../model/runOverviewModel'
import { formatVisitDuration, visitContextWrites, visitDurationMs, visitToolResult, type RunVisit, type VisitFlowNode } from '../model/visitModel'

type ViewArtifact = (entry: { path: string; viewable: boolean }) => void

interface RunStatusItemProps {
    run: RunRecord
    kind: RunStatusKind
    now: number
    visits: RunVisit[]
    flowNodes: Record<string, VisitFlowNode>
    result: PipelineResultResponse | null
    resultError: string | null
    artifactEntries: ArtifactListEntry[]
    /** The output file list failed to load, so an empty list says nothing. */
    artifactsUnavailable?: boolean
    /** The output file list is still loading, so an empty list says nothing yet. */
    artifactsLoading?: boolean
    /** The pending question, answered in place. */
    question: ReactNode
    onSelectVisit: (visit: RunVisit) => void
    onViewArtifact: ViewArtifact
}

export function VisitLink({ visit, onSelect }: { visit: RunVisit; onSelect: (visit: RunVisit) => void }) {
    return (
        <button
            type="button"
            data-testid="run-visit-link"
            data-visit-key={visit.key}
            onClick={() => onSelect(visit)}
            className="text-primary underline-offset-4 hover:underline"
        >
            {visit.label}{visit.count > 1 ? ` ${visit.number}` : ''}
        </button>
    )
}

// A command's output ends with its error, so a long one shows its last lines until asked for all.
const FAILURE_DETAIL_TAIL = 12

/** The runtime's own words for a failure, muted under the plain ones, for whoever debugs the flow. */
export function FailureDetail({ className, children, ...props }: ComponentProps<'div'>) {
    const [expanded, setExpanded] = useState(false)
    const lines = typeof children === 'string' ? children.split('\n') : null
    const hidden = lines && lines.length > FAILURE_DETAIL_TAIL ? lines.length - FAILURE_DETAIL_TAIL : 0
    return (
        <div className={cn('flex flex-col items-start gap-1 font-mono text-xs text-muted-foreground', className)} {...props}>
            {hidden > 0 ? (
                <button
                    type="button"
                    data-testid="failure-detail-toggle"
                    aria-expanded={expanded}
                    onClick={() => setExpanded((value) => !value)}
                    className="font-sans text-xs text-primary underline-offset-4 hover:underline"
                >
                    {expanded ? `Show only the last ${FAILURE_DETAIL_TAIL} lines` : `Show ${hidden} earlier lines`}
                </button>
            ) : null}
            <span className={cn('block max-w-full whitespace-pre-wrap break-words', expanded && 'max-h-[60vh] overflow-auto')}>
                {hidden > 0 && !expanded ? `…\n${lines!.slice(-FAILURE_DETAIL_TAIL).join('\n')}` : children}
            </span>
        </div>
    )
}

/** A clock time or date, with the full timestamp on hover. */
const FACT_TIME_FORMAT = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })
const FACT_TIME_YEAR_FORMAT = new Intl.DateTimeFormat(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })

/** A run fact's time, with its day, so a start and an end on different days read apart: "Nov 13, 11:02 PM". */
function RunTime({ timestamp, now }: { timestamp: string | null | undefined; now: number }) {
    const time = timestamp ? Date.parse(timestamp) : Number.NaN
    if (!Number.isFinite(time)) return <>—</>
    const format = new Date(time).getFullYear() === new Date(now).getFullYear() ? FACT_TIME_FORMAT : FACT_TIME_YEAR_FORMAT
    return <time dateTime={timestamp ?? undefined} title={formatTimestamp(timestamp)}>{format.format(time)}</time>
}

// The keys a visit wrote, named without the "context." prefix.
function VisitWrites({ visit }: { visit: RunVisit }) {
    const { writes } = visitContextWrites(visit)
    if (writes.length === 0) {
        return null
    }
    return (
        <span data-testid="run-status-did-not-pass-writes" className="text-muted-foreground">
            {' · wrote '}
            {writes.map(([key]) => (
                <code key={key} className="mr-1 rounded-md border border-border bg-background px-1.5 py-px font-mono text-xs text-foreground/80">
                    {key.replace(/^context\./, '')}
                </code>
            ))}
        </span>
    )
}

export function RunStatusItem({
    run,
    kind,
    now,
    visits,
    flowNodes,
    result,
    resultError,
    artifactEntries,
    artifactsUnavailable = false,
    artifactsLoading = false,
    question,
    onSelectVisit,
    onViewArtifact,
}: RunStatusItemProps) {
    const rootVisits = visits.filter((visit) => visit.parentKey === null)
    const lastVisit = rootVisits.at(-1) ?? null
    const duration = formatDuration(run.started_at, run.ended_at, run.status, now, run.idle_ms)
    const link = (visit: RunVisit | null | undefined) => (visit ? <VisitLink visit={visit} onSelect={onSelectVisit} /> : null)

    let heading: string
    let meta: ReactNode
    let body: ReactNode
    if (kind === 'waiting') {
        const asker = visits.find((visit) => visit.outcome === 'waiting')
        heading = 'Question'
        meta = <>{asker ? <>Asked by {link(asker)} · </> : null}the run is paused until you answer</>
        body = question
    } else if (kind === 'failed') {
        const infrastructure = classifyRunFailure(run) === 'infrastructure'
        const failure = describeFailure(runFailureMessage(run))
        const failedVisit = [...rootVisits].reverse().find((visit) => visit.outcome === 'failed')
        const stoppedIn = failedVisit ?? lastVisit
        // A command's error carries only its stderr; a test runner names what failed on stdout, so its end shows too.
        const output = failedVisit?.kind === 'tool' ? visitToolResult(failedVisit).output : null
        // The header's badge already says Failed; this section says why.
        heading = 'Why it failed'
        meta = (
            <span data-testid="run-status-failure-kind" data-kind={infrastructure ? 'infrastructure' : 'flow'}>
                {infrastructure ? 'Infrastructure: the backend failed, not the flow’s work' : 'The flow ended in failure'}
            </span>
        )
        body = (
            <>
                <div>
                    <p data-testid="run-status-failure" className="whitespace-pre-wrap break-words text-sm text-destructive">
                        {failure.summary}
                    </p>
                    {failure.detail ? <FailureDetail data-testid="run-status-failure-detail" className="mt-1">{failure.detail}</FailureDetail> : null}
                    {output ? (
                        <div data-testid="run-status-failure-output" className="mt-2">
                            <p className="text-xs font-semibold text-muted-foreground">Output</p>
                            <FailureDetail>{output}</FailureDetail>
                        </div>
                    ) : null}
                </div>
                <p data-testid="run-status-stopped-in" className="text-sm text-muted-foreground">
                    {stoppedIn ? <>Stopped in {link(stoppedIn)} after {duration}.</> : <>Stopped after {duration}.</>}
                    {infrastructure ? ' Retry resumes this run.' : ''}
                </p>
            </>
        )
    } else if (kind === 'running') {
        const running = visits.filter((visit) => visit.outcome === 'running')
        const step = [...running].reverse().find((visit) => visit.parentKey === null)
        // A fan-out runs while its branches do; the branches are where the work is.
        const branches = step ? running.filter((visit) => visit.parentKey === step.key) : []
        const current = branches.length > 0 ? branches : step ? [step] : []
        // Counted as the Steps list counts them, branches included.
        const finished = visits.filter((visit) => visit.outcome !== 'running' && visit.outcome !== 'waiting').length
        heading = 'Running'
        meta = <>for {duration}</>
        body = (
            <div className="flex flex-col gap-2 text-sm text-muted-foreground">
                {current.length > 1 ? (
                    <>
                        <p data-testid="run-status-running">{current.length} steps are running at the same time:</p>
                        <ul className="flex flex-col gap-1">
                            {current.map((visit) => (
                                <li key={visit.key} data-testid="run-status-running-step" className="flex items-center gap-2">
                                    <StatusDot tone="info" />
                                    {link(visit)}
                                    <span className="tabular-nums">{formatVisitDuration(visitDurationMs(visit, now))}</span>
                                </li>
                            ))}
                        </ul>
                    </>
                ) : (
                    <p data-testid="run-status-running">{current[0] ? <>Now in {link(current[0])}.</> : 'Starting…'}</p>
                )}
                {finished > 0 ? (
                    <p data-testid="run-status-finished-count">
                        {finished} of {visits.length} visits finished so far. Pick one to follow its work.
                    </p>
                ) : null}
            </div>
        )
    } else if (kind === 'completed') {
        const source = result?.source_node_id
        heading = 'Result'
        meta = (
            <>
                {formatRunStatusLabel(run)} · {duration}
                {source ? (
                    <span data-testid="run-status-result-source">
                        {' · from '}
                        {result?.source_artifact_path ? (
                            <button
                                type="button"
                                data-testid="run-result-source-button"
                                onClick={() => onViewArtifact({ path: result.source_artifact_path!, viewable: true })}
                                className="underline decoration-border underline-offset-4 hover:text-foreground"
                            >
                                {flowNodes[source]?.label ?? source}
                            </button>
                        ) : flowNodes[source]?.label ?? source}
                        {result?.display_mode === 'summary' ? ', summarized' : null}
                    </span>
                ) : null}
            </>
        )
        // A completed run can reach a failure exit. The flow's own result says why; the route that
        // led there is a run fact, as for a success exit, and stands in for a result the flow
        // didn't write. A result that only restates it, such as a gate's choice and note, gives way to it.
        const reason = run.outcome === 'failure' ? formatOutcomeReason(run) : null
        const resultLines = result?.state === 'ready' ? result.body_markdown.split('\n').map((line) => line.trim()).filter(Boolean) : []
        const resultRestatesReason = Boolean(reason && resultLines.length > 0 && resultLines.every((line) => reason.includes(line)))
        const resultSettled = Boolean(resultError) || (result !== null && result.state !== 'pending')
        const outcomeReason = reason && resultSettled && (resultLines.length === 0 || resultRestatesReason) ? reason : null
        body = (
            <>
                {outcomeReason ? (
                    <p
                        data-testid="run-status-outcome-reason"
                        data-outcome={run.outcome ?? undefined}
                        className="whitespace-pre-wrap break-words text-sm text-warning"
                    >
                        {outcomeReason}
                    </p>
                ) : null}
                {resultRestatesReason ? null : (
                    <RunResult result={result} resultError={resultError} plain={Boolean(source && flowNodes[source]?.kind === 'human_gate')} />
                )}
            </>
        )
    } else {
        heading = formatRunStatusLabel(run)
        meta = <>after {duration}</>
        body = lastVisit ? <p className="text-sm text-muted-foreground">Stopped in {link(lastVisit)}.</p> : null
    }

    const didNotPass = errorVisits(visits)
    // A failed run's own error is shown above; an earlier visit that hit the same error says so
    // instead of repeating it.
    const failureMessage = kind === 'failed' ? runFailureMessage(run) : null
    return (
        <article data-testid="run-status-item" data-status-kind={kind} className="flex flex-col gap-5 pb-6">
            <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h2 tabIndex={-1} className="rounded-sm text-base font-semibold text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring">{heading}</h2>
                <span className="text-sm text-muted-foreground">{meta}</span>
            </header>
            {body}
            {kind !== 'waiting' && didNotPass.length > 0 ? (
                <Well data-testid="run-status-did-not-pass" className="flex flex-col gap-2">
                    <SectionHeading as="h3" count={didNotPass.length}>{kind === 'failed' ? 'Earlier errors' : 'Errors'}</SectionHeading>
                    <ul className="flex flex-col gap-1.5 text-sm">
                        {didNotPass.map((visit) => {
                            const reason = visit.reason?.trim() || null
                            const sameAsAbove = reason !== null && reason === failureMessage
                            const failure = reason && !sameAsAbove ? describeFailure(reason) : null
                            return (
                                <li key={visit.key} data-testid="run-status-did-not-pass-row" className="max-w-[72ch]">
                                    {link(visit)}
                                    {': '}
                                    {sameAsAbove
                                        ? <span className="text-muted-foreground">the same error as above</span>
                                        : failure
                                            ? <span className="whitespace-pre-wrap break-words text-foreground/90">{failure.summary}</span>
                                            : <span className="text-muted-foreground">no reason recorded</span>}
                                    <VisitWrites visit={visit} />
                                    {failure?.detail ? <FailureDetail>{failure.detail}</FailureDetail> : null}
                                </li>
                            )
                        })}
                    </ul>
                </Well>
            ) : null}
            {kind === 'completed' || kind === 'failed' ? (
                <RunOutputs run={run} withCommit={kind === 'completed'} artifactEntries={artifactEntries} unavailable={artifactsUnavailable} loading={artifactsLoading} onViewArtifact={onViewArtifact} />
            ) : null}
            <RunFacts run={run} now={now} />
        </article>
    )
}

// `plain`: the result is a person's gate note, which reads as written, not as Markdown.
function RunResult({ result, resultError, plain }: { result: PipelineResultResponse | null; resultError: string | null; plain: boolean }) {
    if (resultError) {
        return <InlineError data-testid="run-result-error">{resultError}</InlineError>
    }
    if (!result || result.state === 'pending') {
        return <p data-testid="run-result-pending" className="text-sm text-muted-foreground">Loading the result…</p>
    }
    if (result.state === 'unavailable') {
        return <p data-testid="run-result-unavailable" className="text-sm text-muted-foreground">This flow didn’t write a result.</p>
    }
    if (result.state === 'error') {
        return <InlineError data-testid="run-result-resolution-error">{result.error || 'Result resolution failed.'}</InlineError>
    }
    return (
        <div data-testid="run-result-body" className="max-w-[72ch] text-sm">
            {plain
                ? <p className="whitespace-pre-wrap break-words leading-6">{result.body_markdown.trim()}</p>
                : <ProjectConversationMarkdown content={resultMarkdown(result.body_markdown || '')} keepLineBreaks />}
            {result.summary_error ? (
                <p data-testid="run-result-summary-error" className="mt-2 text-xs text-warning">
                    Summary unavailable: {result.summary_error}
                </p>
            ) : null}
        </div>
    )
}

// A raw result that is one JSON value reads best indented, in a code block.
function resultMarkdown(body: string): string {
    const trimmed = body.trim()
    if (!/^[[{]/.test(trimmed)) {
        return body
    }
    try {
        return `\`\`\`json\n${JSON.stringify(JSON.parse(trimmed), null, 2)}\n\`\`\``
    } catch {
        return body
    }
}

function RunOutputs({
    run,
    withCommit,
    artifactEntries,
    unavailable,
    loading,
    onViewArtifact,
}: {
    run: RunRecord
    withCommit: boolean
    artifactEntries: ArtifactListEntry[]
    unavailable: boolean
    loading: boolean
    onViewArtifact: ViewArtifact
}) {
    const resultFiles = artifactEntries.filter((entry) => (
        entry.path.startsWith('result/') || (entry.path.startsWith('artifacts/') && !entry.path.startsWith('artifacts/flow/'))
    ))
    const flowSnapshot = artifactEntries.find((entry) => entry.path === 'artifacts/flow/flow-definition.json')
        ?? artifactEntries.find((entry) => entry.path.startsWith('artifacts/flow/'))
    // A failed run's commit and branch are where it started, not something it made.
    const commit = withCommit ? run.git_commit?.trim() : null
    const branch = withCommit ? run.git_branch?.trim() : null
    const fileLink = (entry: ArtifactListEntry, label: string) => (
        <button
            key={entry.path}
            type="button"
            data-testid="run-output-file"
            data-path={entry.path}
            onClick={() => onViewArtifact(entry)}
            className="mr-3 font-mono text-xs underline decoration-border underline-offset-4 hover:decoration-foreground"
        >
            {label}
        </button>
    )
    // Outputs and run facts are grouped data: wells, not bordered boxes.
    const rows: Array<[string, ReactNode]> = [
        ...(commit || branch ? [['Commit', (
            <span data-testid="run-output-commit">
                {commit ? <code>{commit.slice(0, 7)}</code> : null}
                {commit && branch ? ' on ' : ''}
                {branch ? <code>{branch}</code> : null}
            </span>
        )] as [string, ReactNode]] : []),
        ...(resultFiles.length > 0
            ? [['Result', resultFiles.map((entry) => fileLink(entry, entry.path.replace(/^(result|artifacts)\//, '')))] as [string, ReactNode]]
            : []),
        ...(flowSnapshot ? [['Flow', fileLink(flowSnapshot, 'flow snapshot')] as [string, ReactNode]] : []),
    ]
    return (
        <Well data-testid="run-status-outputs" className="flex flex-col gap-2">
            <SectionHeading as="h3">Outputs</SectionHeading>
            {commit || branch || resultFiles.length > 0 ? (
                <dl className="space-y-1 text-sm">
                    {rows.map(([label, value]) => (
                        <div key={label} className="flex gap-3">
                            <dt className="w-16 shrink-0 text-muted-foreground">{label}</dt>
                            <dd className="min-w-0">{value}</dd>
                        </div>
                    ))}
                </dl>
            ) : unavailable ? (
                <p data-testid="run-status-outputs-unavailable" className="text-sm text-muted-foreground">
                    The output files couldn’t load.
                </p>
            ) : loading ? (
                <p data-testid="run-status-outputs-loading" className="text-sm text-muted-foreground">
                    Loading the output files…
                </p>
            ) : (
                <p data-testid="run-status-no-outputs" className="text-sm text-muted-foreground">
                    None: the run ended before it produced a result.
                    {flowSnapshot ? <> {fileLink(flowSnapshot, 'flow snapshot')}</> : null}
                </p>
            )}
        </Well>
    )
}

const LOCK_STATES: Record<string, string> = {
    holding: 'Holding',
    queued: 'Queued',
    inherited: 'Inherited from parent',
}

function RunFacts({ run, now }: { run: RunRecord; now: number }) {
    const usage = run.token_usage_breakdown
    const provider = run.llm_provider || run.provider
    const lineage = formatLineage(run)
    const costNote = formatEstimatedModelCostNote(run)
    const tokenBreakdown = (bucket: TokenUsageBucketResponse) => (
        `${formatTokenCount(bucket.total_tokens)} (${formatTokenCount(bucket.input_tokens)} in, ${formatTokenCount(bucket.cached_input_tokens)} cached, ${formatTokenCount(bucket.output_tokens)} out)`
    )
    const outcome = run.status === 'completed' && run.outcome_reason_code ? formatOutcomeReason(run) : null
    const facts: Array<[id: string, label: string, value: ReactNode]> = [
        ...(outcome ? [['outcome', 'Outcome', outcome] as [string, string, ReactNode]] : []),
        ['model', 'Model', [run.model || 'default', provider, run.reasoning_effort].filter(Boolean).join(' · ')],
        ...(run.execution_profile_id || run.execution_mode
            ? [['execution', 'Execution', [...new Set([run.execution_profile_id, run.execution_mode, run.execution_container_image].filter(Boolean))].join(' · ')] as [string, string, ReactNode]]
            : []),
        ['tokens', 'Tokens', usage ? `${tokenBreakdown(usage)}${usage.coverage === 'partial' ? ` · partial: ${formatTokenCount(usage.requests_without_usage ?? 0)} request${usage.requests_without_usage === 1 ? '' : 's'} reported no usage` : ''}` : formatTokenCount(run.token_usage)],
        ...Object.entries(usage?.by_model ?? {}).map(([modelId, bucket]) => {
            const cost = run.estimated_model_cost?.by_model?.[modelId]
            return ['model-usage', `  ${modelId}`, `${tokenBreakdown(bucket)} · ${cost?.status === 'estimated' ? formatEstimatedCost(cost.amount, cost.currency) : 'unpriced'}`] as [string, string, ReactNode]
        }),
        ['cost', 'Cost', costNote ? `${formatEstimatedModelCostLabel(run)} · ${costNote}` : formatEstimatedModelCostLabel(run)],
        ['directory', 'Directory', run.working_directory || run.project_path || '—'],
        ...(shouldShowWorkingDirectoryDifference(run, null) ? [['project', 'Project', run.project_path] as [string, string, ReactNode]] : []),
        ...(run.spec_id ? [['spec', 'Spec', run.spec_id] as [string, string, ReactNode]] : []),
        ...(run.plan_id ? [['plan', 'Plan', run.plan_id] as [string, string, ReactNode]] : []),
        ...(hasExecutionLockMetadata(run) ? [['lock', 'Lock', [
            LOCK_STATES[run.execution_lock?.state ?? ''] ?? run.execution_lock?.state,
            run.execution_lock?.key,
            run.execution_lock?.scope && `scope ${run.execution_lock.scope}`,
            run.execution_lock?.conflict_policy && `on conflict ${run.execution_lock.conflict_policy}`,
            typeof run.execution_lock?.queue_position === 'number' && `queue position ${run.execution_lock.queue_position}`,
        ].filter(Boolean).join(' · ')] as [string, string, ReactNode]] : []),
        ['started', 'Started', <RunTime timestamp={run.started_at} now={now} />],
        ['ended', 'Ended', <RunTime timestamp={run.ended_at} now={now} />],
        ['run', 'Run', run.run_id],
        ...(lineage ? [['lineage', 'Lineage', lineage] as [string, string, ReactNode]] : []),
    ]
    return (
        <details data-testid="run-status-facts" className="group">
            <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-semibold text-foreground [&::-webkit-details-marker]:hidden">
                <ChevronRight aria-hidden="true" className="size-3.5 text-muted-foreground transition-transform group-open:rotate-90" />
                Run facts
            </summary>
            <dl className="mt-2 space-y-1 pl-5.5 text-sm">
                {facts.map(([id, label, value]) => (
                    <div key={`${id}-${label}`} data-testid={`run-fact-${id}`} className="flex gap-3">
                        <dt className="w-24 shrink-0 truncate whitespace-pre text-muted-foreground" title={label.trim()}>{label}</dt>
                        <dd className="min-w-0 break-all">{value}</dd>
                    </div>
                ))}
            </dl>
        </details>
    )
}
