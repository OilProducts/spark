import { useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'

import type { RunExecutionPrompt, RunTranscriptSegment } from '@/lib/api/attractorApi'
import { InlineError } from '@/components/app/inline-error'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { formatTimestamp, type TimelineEventEntry } from '../model/shared'
import {
    VISIT_OUTCOME_LABELS,
    formatVisitDuration,
    visitContextWrites,
    visitDurationMs,
    visitMarks,
    visitPrompt,
    visitToolResult,
    visitTranscriptRows,
    type RunVisit,
    type VisitFlowNode,
    type VisitMark,
} from '../model/visitModel'
import { RunTranscriptRowItem, useTranscriptExpansion } from './RunTranscriptGroups'

const MARKS: Record<VisitMark, { glyph: string; label: string; className: string }> = {
    did_not_pass: { glyph: '✕', label: "Didn't pass", className: 'text-warning' },
    failed: { glyph: '✕', label: 'Failed', className: 'text-destructive' },
    interrupted: { glyph: '‖', label: 'Interrupted', className: 'text-warning' },
    waiting: { glyph: '?', label: 'Waiting on a question', className: 'text-info' },
    loop_back: { glyph: '↩\uFE0E', label: 'Looped back', className: 'text-info' },
}

const OUTCOME_CLASSES: Partial<Record<RunVisit['outcome'], string>> = {
    did_not_pass: 'text-warning',
    failed: 'text-destructive',
    interrupted: 'text-warning',
    waiting: 'text-info',
}

const visitCountLabel = (visit: RunVisit) => (visit.count > 1 ? `${visit.number}/${visit.count}` : '')

const formatValue = (value: unknown): string => (
    typeof value === 'string' ? value : JSON.stringify(value, null, 2)
)

interface RunVisitsCardProps {
    visits: RunVisit[]
    flowNodes: Record<string, VisitFlowNode>
    segments: RunTranscriptSegment[]
    prompts: RunExecutionPrompt[]
    journal: TimelineEventEntry[]
    now: number
    isNarrowViewport: boolean
    isLive: boolean
    transcriptError: string | null
    timelineError: string | null
    /** The node selected on the graph; its visits are highlighted. */
    selectedNodeId: string | null
    onSelectNode: (nodeId: string) => void
    onOpenRun: (runId: string) => void
}

export function RunVisitsCard({
    visits,
    flowNodes,
    segments,
    prompts,
    journal,
    now,
    isNarrowViewport,
    isLive,
    transcriptError,
    timelineError,
    selectedNodeId,
    onSelectNode,
    onOpenRun,
}: RunVisitsCardProps) {
    const [selectedKey, setSelectedKey] = useState<string | null>(null)
    const [showJournal, setShowJournal] = useState(false)
    const listRef = useRef<HTMLDivElement | null>(null)

    // An explicit pick wins while it agrees with the graph selection; a graph
    // node alone shows its latest visit; otherwise follow the latest visit.
    const picked = visits.find((visit) => visit.key === selectedKey) ?? null
    const selected = (picked && (!selectedNodeId || picked.nodeId === selectedNodeId))
        ? picked
        : [...visits].reverse().find((visit) => visit.nodeId === selectedNodeId)
            ?? [...visits].reverse().find((visit) => visit.parentKey === null)
            ?? null

    const select = (visit: RunVisit | undefined, focus = false) => {
        if (!visit) {
            return
        }
        setSelectedKey(visit.key)
        setShowJournal(false)
        onSelectNode(visit.nodeId)
        if (focus) {
            listRef.current?.querySelector<HTMLElement>(`[data-visit-key="${CSS.escape(visit.key)}"]`)?.focus()
        }
    }

    const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (!selected || event.metaKey || event.ctrlKey || event.altKey) {
            return
        }
        const index = visits.indexOf(selected)
        const sameNode = visits.filter((visit) => visit.nodeId === selected.nodeId)
        const nodeIndex = sameNode.indexOf(selected)
        const targets: Record<string, RunVisit | undefined> = {
            ArrowDown: visits[index + 1],
            j: visits[index + 1],
            ArrowUp: visits[index - 1],
            k: visits[index - 1],
            ArrowRight: sameNode[nodeIndex + 1],
            ArrowLeft: sameNode[nodeIndex - 1],
        }
        if (Object.hasOwn(targets, event.key)) {
            event.preventDefault()
            select(targets[event.key], true)
        }
    }

    return (
        <section
            data-testid="run-visits-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn('flex gap-4', isNarrowViewport ? 'flex-col' : 'h-full min-h-0')}
        >
            <div className={cn('flex shrink-0 flex-col', isNarrowViewport ? undefined : 'min-h-0 w-64 border-r border-border pr-3')}>
                <div className="flex items-center justify-between gap-2 pb-1 text-xs text-muted-foreground">
                    <span>
                        {visits.length} {visits.length === 1 ? 'visit' : 'visits'}
                        {isLive ? <span className="ml-2 text-info">Live</span> : null}
                    </span>
                    <button
                        type="button"
                        data-testid="run-journal-toggle"
                        aria-pressed={showJournal}
                        onClick={() => setShowJournal((current) => !current)}
                        className={cn('hover:text-foreground', showJournal && 'text-foreground underline underline-offset-4')}
                    >
                        Journal
                    </button>
                </div>
                {isPerformanceDebugEnabled() ? (
                    <p
                        data-testid="timeline-update-performance-budget"
                        data-budget-ms={TIMELINE_UPDATE_BUDGET_MS}
                        className="py-1 text-xs text-muted-foreground"
                    >
                        Journal update budget: {TIMELINE_UPDATE_BUDGET_MS}ms max per live update batch.
                    </p>
                ) : null}
                {transcriptError ? <InlineError data-testid="run-transcript-error">{transcriptError}</InlineError> : null}
                {visits.length === 0 ? (
                    <p data-testid="run-visits-empty" className="py-2 text-sm text-muted-foreground">
                        No visits have been recorded for this run yet.
                    </p>
                ) : (
                    <div
                        ref={listRef}
                        role="listbox"
                        aria-label="Visits"
                        data-testid="run-visit-list"
                        onKeyDown={onListKeyDown}
                        className={cn('divide-y divide-border overflow-y-auto', isNarrowViewport ? 'max-h-72' : 'min-h-0 flex-1')}
                    >
                        {visits.map((visit) => {
                            const isSelected = !showJournal && visit === selected
                            return (
                                <button
                                    key={visit.key}
                                    type="button"
                                    role="option"
                                    aria-selected={isSelected}
                                    tabIndex={visit === selected ? 0 : -1}
                                    data-testid="run-visit-row"
                                    data-visit-key={visit.key}
                                    data-node-id={visit.nodeId}
                                    data-outcome={visit.outcome}
                                    data-node-selected={visit.nodeId === selectedNodeId || undefined}
                                    onClick={() => select(visit)}
                                    className={cn(
                                        'flex w-full items-center gap-2 py-1.5 pr-1 text-left text-sm',
                                        visit.parentKey ? 'pl-5' : 'pl-1',
                                        isSelected ? 'bg-accent text-foreground' : 'text-foreground/90 hover:bg-accent/50',
                                        visit.nodeId === selectedNodeId && !isSelected && 'bg-accent/30',
                                    )}
                                >
                                    <span className="flex w-7 shrink-0 gap-0.5 text-xs" data-testid="run-visit-row-marks">
                                        {visitMarks(visit).map((mark) => (
                                            <span
                                                key={mark}
                                                data-mark={mark}
                                                title={MARKS[mark].label}
                                                aria-label={MARKS[mark].label}
                                                className={MARKS[mark].className}
                                            >
                                                {MARKS[mark].glyph}
                                            </span>
                                        ))}
                                    </span>
                                    <span className="min-w-0 flex-1 truncate">{visit.label}</span>
                                    <span data-testid="run-visit-row-count" className="shrink-0 text-xs text-muted-foreground">
                                        {visitCountLabel(visit)}
                                    </span>
                                    <span className="w-14 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                                        {formatVisitDuration(visitDurationMs(visit, now))}
                                    </span>
                                </button>
                            )
                        })}
                    </div>
                )}
            </div>
            <div className={cn('min-w-0 flex-1', isNarrowViewport ? undefined : 'min-h-0 overflow-y-auto')}>
                {showJournal ? (
                    <RunJournal journal={journal} timelineError={timelineError} />
                ) : selected ? (
                    <RunVisitView
                        key={selected.key}
                        visit={selected}
                        visits={visits}
                        flowNode={flowNodes[selected.nodeId] ?? null}
                        segments={segments}
                        prompts={prompts}
                        now={now}
                        onSelectVisit={(visit) => select(visit)}
                        onOpenRun={onOpenRun}
                    />
                ) : null}
            </div>
        </section>
    )
}

function Section({ title, children, testId }: { title: string; children: ReactNode; testId: string }) {
    return (
        <section data-testid={testId} className="space-y-2 border-t border-border pt-3">
            <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{title}</h4>
            {children}
        </section>
    )
}

function RunVisitView({
    visit,
    visits,
    flowNode,
    segments,
    prompts,
    now,
    onSelectVisit,
    onOpenRun,
}: {
    visit: RunVisit
    visits: RunVisit[]
    flowNode: VisitFlowNode | null
    segments: RunTranscriptSegment[]
    prompts: RunExecutionPrompt[]
    now: number
    onSelectVisit: (visit: RunVisit) => void
    onOpenRun: (runId: string) => void
}) {
    const expansion = useTranscriptExpansion()
    const nodeVisits = visits.filter((candidate) => candidate.nodeId === visit.nodeId)
    const next = visits.find((candidate) => candidate.key === visit.next?.key) ?? null
    const prompt = visitPrompt(visit, prompts)
    const rows = useMemo(() => visitTranscriptRows(visit, segments), [visit, segments])
    const { writes, systemCount } = visitContextWrites(visit)
    const duration = formatVisitDuration(visitDurationMs(visit, now))
    const outcomeClass = OUTCOME_CLASSES[visit.outcome]

    return (
        <article data-testid="run-visit-view" data-visit-key={visit.key} className="space-y-4 pb-6">
            <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h3 data-testid="run-visit-view-title" className="text-lg font-light text-foreground">{visit.label}</h3>
                {visit.count > 1 ? (
                    <span className="text-sm text-muted-foreground">visit {visit.number} of {visit.count}</span>
                ) : null}
                <span data-testid="run-visit-view-outcome" className={cn('text-sm', outcomeClass ?? 'text-muted-foreground')}>
                    {VISIT_OUTCOME_LABELS[visit.outcome]}
                </span>
                {duration ? <span className="text-sm tabular-nums text-muted-foreground">{duration}</span> : null}
                {nodeVisits.length > 1 ? (
                    <span role="group" aria-label={`Visits of ${visit.label}`} data-testid="run-visit-picker" className="ml-auto flex gap-1">
                        {nodeVisits.map((candidate) => (
                            <button
                                key={candidate.key}
                                type="button"
                                aria-current={candidate === visit ? 'true' : undefined}
                                aria-label={`Visit ${candidate.number} of ${candidate.count}`}
                                onClick={() => onSelectVisit(candidate)}
                                className={cn(
                                    'min-w-6 border-b px-1 text-xs tabular-nums',
                                    candidate === visit ? 'border-primary text-foreground' : 'border-transparent text-muted-foreground hover:text-foreground',
                                    OUTCOME_CLASSES[candidate.outcome],
                                )}
                            >
                                {candidate.number}
                            </button>
                        ))}
                    </span>
                ) : null}
            </header>

            {visit.parentKey === null ? (
                <details data-testid="run-visit-instructions" className="group">
                    <summary className="cursor-pointer text-xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground">
                        Instructions
                    </summary>
                    <div className="mt-2 max-h-96 overflow-y-auto whitespace-pre-wrap break-words border-l border-border pl-3 text-sm text-foreground/90">
                        {prompt ?? <span className="text-muted-foreground">No instructions were recorded for this visit.</span>}
                    </div>
                </details>
            ) : null}
            {flowNode && flowNode.readsContext.length > 0 ? (
                <p data-testid="run-visit-reads" className="text-xs text-muted-foreground">
                    Reads{' '}
                    {flowNode.readsContext.map((key, index) => (
                        <span key={key}>
                            {index > 0 ? ', ' : ''}
                            <code className="text-foreground/80">{key}</code>
                        </span>
                    ))}
                </p>
            ) : null}

            <Section title="The work" testId="run-visit-work">
                {visit.kind === 'tool' ? (
                    <ToolWork visit={visit} command={flowNode?.command ?? null} />
                ) : visit.kind === 'subflow' ? (
                    visit.childRun ? (
                        <div data-testid="run-visit-child-run" className="space-y-1 text-sm">
                            <p>
                                <span className="text-foreground">{visit.childRun.flowName ?? 'Child run'}</span>
                                <span className="ml-2 text-muted-foreground">
                                    {formatVisitDuration(visitDurationMs({ startedAt: visit.childRun.startedAt, endedAt: visit.childRun.endedAt }, now))}
                                </span>
                                <Button
                                    type="button"
                                    variant="link"
                                    size="sm"
                                    className="ml-2 h-auto p-0 align-baseline"
                                    data-testid="run-visit-open-child-run"
                                    onClick={() => onOpenRun(visit.childRun!.runId)}
                                >
                                    Open child run
                                </Button>
                            </p>
                            {visit.childRun.summary ? (
                                <p className="whitespace-pre-wrap text-foreground/90">{visit.childRun.summary}</p>
                            ) : null}
                        </div>
                    ) : (
                        <p className="text-sm text-muted-foreground">No child run was recorded for this visit.</p>
                    )
                ) : rows.length > 0 ? (
                    <ul className="list-none space-y-2" data-testid="run-visit-transcript">
                        {rows.map((row) => (
                            <RunTranscriptRowItem key={row.segment.id} row={row} expansion={expansion} />
                        ))}
                    </ul>
                ) : (
                    <p className="text-sm text-muted-foreground">No transcript was recorded for this visit.</p>
                )}
            </Section>

            {visit.outcome !== 'running' && visit.outcome !== 'waiting' ? (
                <Section title="Returned" testId="run-visit-returned">
                    <dl className="space-y-1 text-sm">
                        <div className="flex gap-3">
                            <dt className="w-20 shrink-0 text-muted-foreground">Outcome</dt>
                            <dd className={outcomeClass}>{VISIT_OUTCOME_LABELS[visit.outcome]}</dd>
                        </div>
                        {next ? (
                            <div className="flex gap-3">
                                <dt className="w-20 shrink-0 text-muted-foreground">Next</dt>
                                <dd>
                                    <button
                                        type="button"
                                        data-testid="run-visit-next"
                                        onClick={() => onSelectVisit(next)}
                                        className="text-left hover:underline hover:underline-offset-4"
                                    >
                                        {visit.next?.loopBack ? 'Sent back to ' : ''}
                                        {next.label}
                                        {next.count > 1 ? ` ${next.number}/${next.count}` : ''}
                                    </button>
                                </dd>
                            </div>
                        ) : null}
                        {visit.reason ? (
                            <div className="flex gap-3">
                                <dt className="w-20 shrink-0 text-muted-foreground">Reason</dt>
                                <dd data-testid="run-visit-reason" className="min-w-0 whitespace-pre-wrap break-words">{visit.reason}</dd>
                            </div>
                        ) : null}
                    </dl>
                    {writes.length > 0 ? (
                        <ul data-testid="run-visit-writes" className="divide-y divide-border border-y border-border text-sm">
                            {writes.map(([key, value]) => (
                                <li key={key} data-testid="run-visit-write" data-key={key}>
                                    {value === null ? (
                                        <p className="flex gap-3 py-1">
                                            <code className="truncate text-foreground/80">{key}</code>
                                            <span className="ml-auto shrink-0 text-muted-foreground">cleared</span>
                                        </p>
                                    ) : (
                                        <details>
                                            <summary className="flex cursor-pointer gap-3 py-1">
                                                <code className="shrink-0 text-foreground/80">{key}</code>
                                                <span className="min-w-0 truncate text-muted-foreground">{formatValue(value)}</span>
                                            </summary>
                                            <pre className="mb-2 max-h-72 overflow-auto whitespace-pre-wrap break-words pl-3 text-xs text-foreground/90">
                                                {formatValue(value)}
                                            </pre>
                                        </details>
                                    )}
                                </li>
                            ))}
                        </ul>
                    ) : null}
                    {systemCount > 0 ? (
                        <p data-testid="run-visit-system-writes" className="text-xs text-muted-foreground">
                            {systemCount} system {systemCount === 1 ? 'key' : 'keys'} written
                        </p>
                    ) : null}
                </Section>
            ) : null}
        </article>
    )
}

function ToolWork({ visit, command }: { visit: RunVisit; command: string | null }) {
    const { output, exitCode } = visitToolResult(visit)
    return (
        <div data-testid="run-visit-tool" className="space-y-2 text-sm">
            {command ? <pre className="overflow-x-auto whitespace-pre-wrap border-l border-border pl-3 text-xs">{command}</pre> : null}
            {output ? (
                <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words border-l border-border pl-3 text-xs text-foreground/90">{output}</pre>
            ) : null}
            {exitCode !== null ? (
                <p className={cn('text-xs', exitCode === 0 ? 'text-muted-foreground' : 'text-warning')}>Exit code {exitCode}</p>
            ) : null}
            {!command && !output && exitCode === null ? (
                <p className="text-muted-foreground">No command output was recorded for this visit.</p>
            ) : null}
        </div>
    )
}

function RunJournal({ journal, timelineError }: { journal: TimelineEventEntry[]; timelineError: string | null }) {
    return (
        <section data-testid="run-journal-panel" className="space-y-2">
            <h3 className="text-lg font-light text-foreground">Journal</h3>
            {timelineError ? <InlineError data-testid="run-event-timeline-error">{timelineError}</InlineError> : null}
            <ol className="divide-y divide-border text-sm">
                {journal.map((event) => (
                    <li key={event.id} data-testid="run-journal-row" className="flex gap-3 py-1">
                        <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{formatTimestamp(event.receivedAt)}</span>
                        <span className={cn(
                            'min-w-0 break-words',
                            event.severity === 'error' ? 'text-destructive' : event.severity === 'warning' ? 'text-warning' : undefined,
                        )}
                        >
                            {event.summary}
                        </span>
                    </li>
                ))}
            </ol>
        </section>
    )
}
