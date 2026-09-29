import type { ReactNode } from 'react'

import type { PipelineResultResponse } from '@/lib/attractorClient'
import { InlineError } from '@/components/app/inline-error'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
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
    didNotPassVisits,
    runFailureMessage,
    type RunStatusKind,
} from '../model/runOverviewModel'
import type { RunVisit, VisitFlowNode } from '../model/visitModel'

type ViewArtifact = (entry: { path: string; viewable: boolean }) => void
type GitRef = { commit: string | null; branch: string | null }

interface RunStatusItemProps {
    run: RunRecord
    kind: RunStatusKind
    now: number
    visits: RunVisit[]
    flowNodes: Record<string, VisitFlowNode>
    result: PipelineResultResponse | null
    resultError: string | null
    artifactEntries: ArtifactListEntry[]
    /** The commit and branch the flow recorded in its context. */
    recordedRef: GitRef
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
            className="text-foreground underline decoration-border underline-offset-4 hover:decoration-foreground"
        >
            {visit.label}{visit.count > 1 ? ` ${visit.number}/${visit.count}` : ''}
        </button>
    )
}

function SectionLabel({ children }: { children: ReactNode }) {
    return <h4 className="pt-4 text-xs font-medium uppercase tracking-wide text-muted-foreground">{children}</h4>
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
    recordedRef,
    question,
    onSelectVisit,
    onViewArtifact,
}: RunStatusItemProps) {
    const rootVisits = visits.filter((visit) => visit.parentKey === null)
    const lastVisit = rootVisits.at(-1) ?? null
    const duration = formatDuration(run.started_at, run.ended_at, run.status, now)
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
        const stoppedIn = [...rootVisits].reverse().find((visit) => visit.outcome === 'failed') ?? lastVisit
        heading = 'Failed'
        meta = (
            <span data-testid="run-status-failure-kind" data-kind={infrastructure ? 'infrastructure' : 'flow'}>
                {infrastructure ? 'Infrastructure: the backend failed, not the flow’s work' : 'The flow ended in failure'}
            </span>
        )
        body = (
            <>
                <p data-testid="run-status-failure" className="whitespace-pre-wrap break-words text-sm text-destructive">
                    {runFailureMessage(run)}
                </p>
                <p data-testid="run-status-stopped-in" className="text-sm text-muted-foreground">
                    {stoppedIn ? <>Stopped in {link(stoppedIn)} after {duration}.</> : <>Stopped after {duration}.</>}
                    {infrastructure ? ' Retry resumes this run.' : ''}
                </p>
            </>
        )
    } else if (kind === 'running') {
        const current = [...visits].reverse().find((visit) => visit.outcome === 'running')
        heading = 'Running'
        meta = <>for {duration}</>
        body = (
            <p data-testid="run-status-running" className="text-sm text-muted-foreground">
                {current ? <>Now in {link(current)}.</> : 'Starting…'}
            </p>
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
                        {result?.display_mode === 'summary' ? ', summarized' : ', raw output'}
                    </span>
                ) : null}
            </>
        )
        // A completed run can still report a failed outcome, such as a rejected gate.
        const outcomeReason = run.outcome === 'failure' || run.outcome_reason_code ? formatOutcomeReason(run) : null
        body = (
            <>
                {outcomeReason ? (
                    <p data-testid="run-status-outcome-reason" className="whitespace-pre-wrap break-words text-sm text-warning">{outcomeReason}</p>
                ) : null}
                <RunResult result={result} resultError={resultError} />
            </>
        )
    } else {
        heading = formatRunStatusLabel(run)
        meta = <>after {duration}</>
        body = lastVisit ? <p className="text-sm text-muted-foreground">Stopped in {link(lastVisit)}.</p> : null
    }

    const didNotPass = didNotPassVisits(visits)
    return (
        <article data-testid="run-status-item" data-status-kind={kind} className="space-y-3 pb-6">
            <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h3 className="text-lg font-light text-foreground">{heading}</h3>
                <span className="text-sm text-muted-foreground">{meta}</span>
            </header>
            {body}
            {kind !== 'waiting' && didNotPass.length > 0 ? (
                <section data-testid="run-status-did-not-pass">
                    <SectionLabel>Didn’t pass ({didNotPass.length})</SectionLabel>
                    <ul className="divide-y divide-border text-sm">
                        {didNotPass.map((visit) => (
                            <li key={visit.key} data-testid="run-status-did-not-pass-row" className="py-1.5">
                                {link(visit)}
                                {': '}
                                {visit.reason
                                    ? <span className="whitespace-pre-wrap break-words text-foreground/90">{visit.reason}</span>
                                    : <span className="text-muted-foreground">no reason recorded</span>}
                            </li>
                        ))}
                    </ul>
                </section>
            ) : null}
            {kind === 'completed' || kind === 'failed' ? (
                <RunOutputs run={run} recordedRef={recordedRef} withCommit={kind === 'completed'} artifactEntries={artifactEntries} onViewArtifact={onViewArtifact} />
            ) : null}
            <RunFacts run={run} />
        </article>
    )
}

function RunResult({ result, resultError }: { result: PipelineResultResponse | null; resultError: string | null }) {
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
        <div data-testid="run-result-body" className="text-sm">
            <ProjectConversationMarkdown content={result.body_markdown || ''} />
            {result.summary_error ? (
                <p data-testid="run-result-summary-error" className="mt-2 text-xs text-warning">
                    Summary unavailable: {result.summary_error}
                </p>
            ) : null}
        </div>
    )
}

function RunOutputs({
    run,
    recordedRef,
    withCommit,
    artifactEntries,
    onViewArtifact,
}: {
    run: RunRecord
    recordedRef: GitRef
    withCommit: boolean
    artifactEntries: ArtifactListEntry[]
    onViewArtifact: ViewArtifact
}) {
    const resultFiles = artifactEntries.filter((entry) => (
        entry.path.startsWith('result/') || (entry.path.startsWith('artifacts/') && !entry.path.startsWith('artifacts/flow/'))
    ))
    const flowSnapshot = artifactEntries.find((entry) => entry.path.startsWith('artifacts/flow/flow-source'))
        ?? artifactEntries.find((entry) => entry.path.startsWith('artifacts/flow/'))
    // The flow's recorded commit is its output. Without one, a completed run's
    // start commit stands in; a failed run's is where it started, not something it made.
    const startRef = withCommit ? { commit: run.git_commit?.trim() || null, branch: run.git_branch?.trim() || null } : null
    const { commit, branch } = recordedRef.commit ? recordedRef : startRef ?? { commit: null, branch: null }
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
        <section data-testid="run-status-outputs">
            <SectionLabel>Outputs</SectionLabel>
            {commit || branch || resultFiles.length > 0 ? (
                <dl className="space-y-1 text-sm">
                    {rows.map(([label, value]) => (
                        <div key={label} className="flex gap-3">
                            <dt className="w-16 shrink-0 text-muted-foreground">{label}</dt>
                            <dd className="min-w-0">{value}</dd>
                        </div>
                    ))}
                </dl>
            ) : (
                <p data-testid="run-status-no-outputs" className="text-sm text-muted-foreground">
                    None: the run ended before it produced a result.
                    {flowSnapshot ? <> {fileLink(flowSnapshot, 'flow snapshot')}</> : null}
                </p>
            )}
        </section>
    )
}

const LOCK_STATES: Record<string, string> = {
    holding: 'Holding',
    queued: 'Queued',
    inherited: 'Inherited from parent',
}

function RunFacts({ run }: { run: RunRecord }) {
    const usage = run.token_usage_breakdown
    const provider = run.llm_provider || run.provider
    const lineage = formatLineage(run)
    const costNote = formatEstimatedModelCostNote(run)
    const facts: Array<[id: string, label: string, value: ReactNode]> = [
        ['model', 'Model', [run.model || 'default', provider, run.reasoning_effort].filter(Boolean).join(' · ')],
        ...(run.execution_profile_id || run.execution_mode
            ? [['execution', 'Execution', [run.execution_profile_id, run.execution_mode, run.execution_container_image].filter(Boolean).join(' · ')] as [string, string, ReactNode]]
            : []),
        ['tokens', 'Tokens', usage
            ? `${formatTokenCount(usage.total_tokens)} (${formatTokenCount(usage.input_tokens)} in, ${formatTokenCount(usage.cached_input_tokens)} cached, ${formatTokenCount(usage.output_tokens)} out)`
            : formatTokenCount(run.token_usage)],
        ...Object.entries(usage?.by_model ?? {}).map(([modelId, bucket]) => {
            const cost = run.estimated_model_cost?.by_model?.[modelId]
            return ['model-usage', `  ${modelId}`, `${formatTokenCount(bucket.total_tokens)} tokens · ${cost?.status === 'estimated' ? formatEstimatedCost(cost.amount, cost.currency) : 'unpriced'}`] as [string, string, ReactNode]
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
        ['started', 'Started', formatTimestamp(run.started_at)],
        ['ended', 'Ended', formatTimestamp(run.ended_at)],
        ['run', 'Run', run.run_id],
        ...(lineage ? [['lineage', 'Lineage', lineage] as [string, string, ReactNode]] : []),
    ]
    return (
        <details data-testid="run-status-facts" className="border-t border-border pt-3">
            <summary className="cursor-pointer text-xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground">
                Run facts
            </summary>
            <dl className="mt-2 space-y-1 text-sm">
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
