import { useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { ArrowRight, ChevronRight, CircleHelp, Minus, Undo2, Workflow, X, type LucideIcon } from 'lucide-react'

import type { RunExecutionPrompt, RunTranscriptSegment } from '@/lib/api/attractorApi'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { SegmentedControl } from '@/components/app/segmented-control'
import { StatusDot, StatusPill, statusTextClassName, type StatusTone } from '@/components/app/status'
import { ShowThinkingToggle } from '@/components/app/transcript/SegmentRows'
import { Well } from '@/components/app/well'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { formatTimestamp, type ArtifactListEntry, type TimelineEventEntry } from '../model/shared'
import {
    VISIT_OUTCOME_LABELS,
    formatVisitDuration,
    visitContextWrites,
    visitDurationMs,
    visitMark,
    visitPrompt,
    visitToolResult,
    visitTranscriptRows,
    type RunVisit,
    type VisitFlowNode,
    type VisitMark,
} from '../model/visitModel'
import { RunTranscript } from './RunTranscriptGroups'

// Colour marks exceptions: a successful visit carries no icon, a visit that was sent
// back (or didn't pass) a warning return arrow, a failure a destructive cross.
const MARKS: Record<VisitMark, { Icon: LucideIcon; label: string; tone: StatusTone }> = {
    did_not_pass: { Icon: Undo2, label: "Didn't pass", tone: 'warning' },
    failed: { Icon: X, label: 'Failed', tone: 'destructive' },
    interrupted: { Icon: Minus, label: 'Interrupted', tone: 'warning' },
    waiting: { Icon: CircleHelp, label: 'Waiting on a question', tone: 'info' },
    loop_back: { Icon: Undo2, label: 'Sent back', tone: 'warning' },
}

const OUTCOME_TONES: Record<RunVisit['outcome'], StatusTone> = {
    succeeded: 'success',
    partly_succeeded: 'warning',
    did_not_pass: 'warning',
    failed: 'destructive',
    interrupted: 'warning',
    running: 'info',
    waiting: 'info',
}

// ponytail: fixed file set per execution; list the directory if nodes write more worth reading.
const VISIT_FILES = ['initial-context.txt', 'prompt.md', 'response.md', 'status.json', 'transcript.jsonl']

type ItemKey = 'status' | 'context'

const formatValue = (value: unknown): string => (
    typeof value === 'string' ? value : JSON.stringify(value, null, 2)
)

const railRowClass = (isSelected: boolean) => cn(
    'flex w-full items-center gap-2 rounded-lg border px-2.5 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring',
    isSelected ? 'border-border bg-background font-medium text-foreground' : 'border-transparent text-foreground hover:bg-background/60',
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
    onSelectNode: (nodeId: string | null) => void
    onOpenRun: (runId: string) => void
    /** The status item's row: what it says and whether it needs attention. */
    statusRow: { label: string; tone?: StatusTone; detail?: string }
    /** How many keys the Context item lists. */
    contextKeyCount: number
    renderStatus: (selectVisit: (visit: RunVisit) => void) => ReactNode
    renderContext: (focusKey: string | null, selectVisit: (visit: RunVisit) => void) => ReactNode
    artifactEntries: ArtifactListEntry[]
    onViewArtifact: (entry: ArtifactListEntry) => void
    /** The run graph toggle, shown in the rail's view control. */
    graph?: { shown: boolean; onToggle: () => void }
}

export function RunVisitsCard({
    visits: allVisits,
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
    statusRow,
    contextKeyCount,
    renderStatus,
    renderContext,
    artifactEntries,
    onViewArtifact,
    graph,
}: RunVisitsCardProps) {
    // An exit node that simply succeeded says nothing the status item doesn't.
    const visits = useMemo(
        () => allVisits.filter((visit) => visit.kind !== 'exit' || visit.outcome !== 'succeeded'),
        [allVisits],
    )
    // The status item leads and is selected when a run opens.
    const [selectedKey, setSelectedKey] = useState<string>('status')
    const [contextFocusKey, setContextFocusKey] = useState<string | null>(null)
    const [showJournal, setShowJournal] = useState(false)
    const listRef = useRef<HTMLDivElement | null>(null)

    // The node selection drives what the actions act on, so a picked visit only
    // shows while its node is selected; a graph node alone shows its latest
    // visit; with no node selected, the status or Context item.
    const picked = visits.find((visit) => visit.key === selectedKey && visit.nodeId === selectedNodeId) ?? null
    const graphVisit = selectedNodeId && !picked
        ? [...visits].reverse().find((visit) => visit.nodeId === selectedNodeId) ?? null
        : null
    const selected: RunVisit | ItemKey = picked
        ?? graphVisit
        ?? (selectedKey === 'context' ? 'context' : 'status')
    const items: Array<RunVisit | ItemKey> = ['status', 'context', ...visits]
    const itemKey = (item: RunVisit | ItemKey) => (typeof item === 'string' ? item : item.key)

    const select = (item: RunVisit | ItemKey) => {
        setSelectedKey(itemKey(item))
        setContextFocusKey(null)
        setShowJournal(false)
        onSelectNode(typeof item === 'string' ? null : item.nodeId)
    }
    const openContextKey = (key: string) => {
        select('context')
        setContextFocusKey(key)
    }

    const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        if (event.metaKey || event.ctrlKey || event.altKey) {
            return
        }
        const index = items.indexOf(selected)
        const sameNode = typeof selected === 'string' ? [] : visits.filter((visit) => visit.nodeId === selected.nodeId)
        const nodeIndex = typeof selected === 'string' ? -1 : sameNode.indexOf(selected)
        const targets: Record<string, RunVisit | ItemKey | undefined> = {
            ArrowDown: items[index + 1],
            j: items[index + 1],
            ArrowUp: items[index - 1],
            k: items[index - 1],
            ArrowRight: sameNode[nodeIndex + 1],
            ArrowLeft: nodeIndex > 0 ? sameNode[nodeIndex - 1] : undefined,
        }
        const target = targets[event.key]
        if (Object.hasOwn(targets, event.key)) {
            event.preventDefault()
        }
        if (target) {
            select(target)
            listRef.current?.querySelector<HTMLElement>(`[data-visit-key="${CSS.escape(itemKey(target))}"]`)?.focus()
        }
    }

    return (
        <section
            data-testid="run-visits-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn('flex gap-6', isNarrowViewport ? 'flex-col' : 'h-full min-h-0')}
        >
            <Well className={cn('flex shrink-0 flex-col gap-0.5 p-2', isNarrowViewport ? undefined : 'min-h-0 w-64')}>
                {/* Steps and Journal exclude each other on the track; the graph is additive, so
                    its toggle stands apart (a pressed segment beside a pressed segment reads wrong). */}
                <div className="mb-1.5 flex items-center gap-1">
                    <SegmentedControl
                        aria-label="View"
                        className="min-w-0 flex-1"
                        options={[
                            { key: 'steps', label: 'Steps', pressed: !showJournal, onPress: () => setShowJournal(false) },
                            {
                                key: 'journal',
                                label: 'Journal',
                                pressed: showJournal,
                                onPress: () => setShowJournal((current) => !current),
                                'data-testid': 'run-journal-toggle',
                            },
                        ]}
                    />
                    {graph ? (
                        <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            aria-pressed={graph.shown}
                            aria-keyshortcuts="g"
                            title="Show or hide the run graph (g)"
                            data-testid="run-graph-toggle"
                            onClick={graph.onToggle}
                            className="shrink-0 px-2 text-xs text-muted-foreground hover:bg-background/60 hover:text-foreground aria-pressed:bg-background aria-pressed:font-semibold aria-pressed:text-foreground"
                        >
                            <Workflow className="size-3.5" />
                            Graph
                        </Button>
                    ) : null}
                </div>
                {isPerformanceDebugEnabled() ? (
                    <p
                        data-testid="timeline-update-performance-budget"
                        data-budget-ms={TIMELINE_UPDATE_BUDGET_MS}
                        className="px-2.5 py-1 text-xs text-muted-foreground"
                    >
                        Journal update budget: {TIMELINE_UPDATE_BUDGET_MS}ms max per live update batch.
                    </p>
                ) : null}
                {transcriptError ? <InlineError data-testid="run-transcript-error">{transcriptError}</InlineError> : null}
                <div
                    ref={listRef}
                    role="listbox"
                    aria-label="Visits"
                    data-testid="run-visit-list"
                    onKeyDown={onListKeyDown}
                    className={cn('flex flex-col gap-0.5 overflow-y-auto', isNarrowViewport ? 'max-h-72' : 'min-h-0 flex-1')}
                >
                    {(['status', 'context'] as const).map((item) => (
                        <button
                            key={item}
                            type="button"
                            role="option"
                            aria-selected={!showJournal && selected === item}
                            tabIndex={selected === item ? 0 : -1}
                            data-testid={`run-visit-item-${item}`}
                            data-visit-key={item}
                            onClick={() => select(item)}
                            className={cn(railRowClass(!showJournal && selected === item), 'h-8.5')}
                        >
                            <span className="flex w-3.5 shrink-0 justify-center">
                                {item === 'status' && statusRow.tone ? <StatusDot tone={statusRow.tone} /> : null}
                            </span>
                            <span className={cn('min-w-0 flex-1 truncate', item === 'status' && statusRow.tone ? statusTextClassName(statusRow.tone) : undefined)}>
                                {item === 'status' ? statusRow.label : 'Context'}
                            </span>
                            <span className="shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                                {item === 'status' ? statusRow.detail : `${contextKeyCount} ${contextKeyCount === 1 ? 'key' : 'keys'}`}
                            </span>
                        </button>
                    ))}
                    <div className="flex items-center justify-between gap-2 px-2.5 pb-1 pt-3 text-xs font-semibold text-muted-foreground">
                        <span>Steps</span>
                        <span className="font-normal">
                            {visits.length} {visits.length === 1 ? 'visit' : 'visits'}
                            {isLive ? <span className="ml-1.5 text-info">Live</span> : null}
                        </span>
                    </div>
                    {visits.length === 0 ? (
                        <p data-testid="run-visits-empty" className="px-2.5 py-2 text-sm text-muted-foreground">
                            No visits have been recorded for this run yet.
                        </p>
                    ) : null}
                    {visits.map((visit) => {
                        const isSelected = !showJournal && visit === selected
                        const mark = visitMark(visit)
                        const Mark = mark ? MARKS[mark] : null
                        // Where a loop-back went is generic: the model marks a visit whose next node
                        // first ran no later than this one, whatever the nodes are called.
                        const sentBackTo = visit.next?.loopBack
                            ? visits.find((candidate) => candidate.key === visit.next?.key) ?? null
                            : null
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
                                    railRowClass(isSelected),
                                    sentBackTo ? 'items-start py-1.5' : 'min-h-8 py-1',
                                    visit.parentKey && 'pl-6',
                                    visit.nodeId === selectedNodeId && !isSelected && 'bg-background/40',
                                )}
                            >
                                <span className={cn('flex w-3.5 shrink-0 justify-center', sentBackTo && 'mt-[3px]')} data-testid="run-visit-row-marks">
                                    {Mark ? (
                                        <span
                                            data-mark={mark}
                                            role="img"
                                            title={Mark.label}
                                            aria-label={Mark.label}
                                            className={cn('flex', statusTextClassName(Mark.tone))}
                                        >
                                            <Mark.Icon className="size-3.5" />
                                        </span>
                                    ) : null}
                                </span>
                                <span className="flex min-w-0 flex-1 flex-col">
                                    <span className="break-words">
                                        {visit.label}
                                        <span data-testid="run-visit-row-count" className="ml-1.5 text-xs font-normal text-muted-foreground">
                                            {visit.count > 1 ? visit.number : ''}
                                        </span>
                                    </span>
                                    {sentBackTo ? (
                                        <span data-testid="run-visit-row-sent-back" className="break-words text-xs font-normal text-warning">
                                            Sent back to {sentBackTo.label}
                                        </span>
                                    ) : null}
                                </span>
                                <span className="w-14 shrink-0 text-right text-xs font-normal tabular-nums text-muted-foreground">
                                    {formatVisitDuration(visitDurationMs(visit, now))}
                                </span>
                            </button>
                        )
                    })}
                </div>
            </Well>
            <div className={cn('min-w-0 flex-1', isNarrowViewport ? undefined : 'min-h-0 overflow-y-auto')}>
                {showJournal ? (
                    <RunJournal journal={journal} timelineError={timelineError} />
                ) : selected === 'status' ? (
                    renderStatus(select)
                ) : selected === 'context' ? (
                    <div key={contextFocusKey ?? ''}>{renderContext(contextFocusKey, select)}</div>
                ) : (
                    <RunVisitView
                        key={selected.key}
                        visit={selected}
                        visits={visits}
                        flowNode={flowNodes[selected.nodeId] ?? null}
                        segments={segments}
                        prompts={prompts}
                        now={now}
                        artifactEntries={artifactEntries}
                        onViewArtifact={onViewArtifact}
                        onSelectVisit={(visit) => select(visit)}
                        onOpenContextKey={openContextKey}
                        onOpenRun={onOpenRun}
                    />
                )}
            </div>
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
    artifactEntries,
    onViewArtifact,
    onSelectVisit,
    onOpenContextKey,
    onOpenRun,
}: {
    visit: RunVisit
    visits: RunVisit[]
    flowNode: VisitFlowNode | null
    segments: RunTranscriptSegment[]
    prompts: RunExecutionPrompt[]
    now: number
    artifactEntries: ArtifactListEntry[]
    onViewArtifact: (entry: ArtifactListEntry) => void
    onSelectVisit: (visit: RunVisit) => void
    onOpenContextKey: (key: string) => void
    onOpenRun: (runId: string) => void
}) {
    const [showThinking, setShowThinking] = useState(false)
    const nodeVisits = visits.filter((candidate) => candidate.nodeId === visit.nodeId)
    const next = visits.find((candidate) => candidate.key === visit.next?.key) ?? null
    const prompt = visitPrompt(visit, prompts)
    const rows = useMemo(() => visitTranscriptRows(visit, segments), [visit, segments])
    const hasThinking = rows.some((row) => row.kind === 'thinking')
    const { writes, systemCount } = visitContextWrites(visit)
    const duration = formatVisitDuration(visitDurationMs(visit, now))
    const outcomeLabel = VISIT_OUTCOME_LABELS[visit.outcome]
    const returned = visit.outcome !== 'running' && visit.outcome !== 'waiting'
    const headerMeta = [visit.count > 1 ? `Visit ${visit.number} of ${visit.count}` : null, duration || null]
        .filter((part): part is string => part !== null)
        .join(' · ')
    const filesDir = `logs/${visit.nodeId}/executions/${visit.stageIndex}-${visit.attempt}/`
    const files = visit.parentKey === null
        ? VISIT_FILES.flatMap((name) => artifactEntries.filter((entry) => entry.path === filesDir + name))
        : []
    const contextKeyButton = (key: string) => (
        <button
            type="button"
            data-testid="run-visit-context-key"
            data-key={key}
            onClick={(event) => {
                event.preventDefault()
                onOpenContextKey(key)
            }}
            title={key}
            className="rounded-md border border-border bg-background px-1.5 py-px font-mono text-xs text-foreground/80 hover:text-foreground"
        >
            {key.replace(/^context\./, '')}
        </button>
    )
    // The pill sits in the Returned well once the visit has returned; until then it is the
    // live state and belongs beside the title.
    const outcomePill = (
        <StatusPill data-testid="run-visit-view-outcome" tone={OUTCOME_TONES[visit.outcome]}>{outcomeLabel}</StatusPill>
    )
    const workTitle = visit.kind === 'tool' ? 'Command' : visit.kind === 'subflow' ? 'Child run' : 'Transcript'

    return (
        <article data-testid="run-visit-view" data-visit-key={visit.key} className="flex flex-col gap-5 pb-6">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
                    <h3 data-testid="run-visit-view-title" className="text-lg font-semibold text-foreground">{visit.label}</h3>
                    {headerMeta ? <span className="text-sm text-muted-foreground">{headerMeta}</span> : null}
                    {returned ? null : outcomePill}
                </div>
                {nodeVisits.length > 1 ? (
                    <SegmentedControl
                        aria-label={`Visits of ${visit.label}`}
                        data-testid="run-visit-picker"
                        className="flex-none"
                        options={nodeVisits.map((candidate) => {
                            const mark = visitMark(candidate)
                            return {
                                key: candidate.key,
                                pressed: candidate === visit,
                                onPress: () => onSelectVisit(candidate),
                                'aria-label': `Visit ${candidate.number} of ${candidate.count}${candidate.next?.loopBack ? ', sent back' : ''}`,
                                className: 'flex-none tabular-nums',
                                label: (
                                    <>
                                        {candidate.number}
                                        {mark ? <StatusDot tone={MARKS[mark].tone} className="size-1.5" /> : null}
                                    </>
                                ),
                            }
                        })}
                    />
                ) : null}
            </header>

            {returned ? (
                <Well data-testid="run-visit-returned" className="flex flex-col gap-2.5">
                    <div className="flex flex-wrap items-center gap-2.5">
                        <SectionHeading as="h4">Returned</SectionHeading>
                        {outcomePill}
                        {next ? (
                            <span className="inline-flex items-center gap-1 text-sm text-muted-foreground">
                                {visit.next?.loopBack ? 'Sent back to' : 'Next'}
                                <ArrowRight aria-hidden="true" className="size-3.5" />
                                <button
                                    type="button"
                                    data-testid="run-visit-next"
                                    onClick={() => onSelectVisit(next)}
                                    className="font-medium text-foreground underline-offset-4 hover:underline"
                                >
                                    {next.label}
                                    {next.count > 1 ? ` ${next.number}` : ''}
                                </button>
                            </span>
                        ) : null}
                    </div>
                    {visit.reason ? (
                        <p data-testid="run-visit-reason" className="max-w-[72ch] whitespace-pre-wrap break-words text-sm leading-6 text-foreground">{visit.reason}</p>
                    ) : null}
                    {writes.length > 0 ? (
                        <ul data-testid="run-visit-writes" className="flex flex-col gap-1 text-xs text-muted-foreground">
                            {writes.map(([key, value]) => (
                                <li key={key} data-testid="run-visit-write" data-key={key} className="min-w-0">
                                    {value === null ? (
                                        <p className="flex items-center gap-2">
                                            {contextKeyButton(key)}
                                            <span>cleared</span>
                                        </p>
                                    ) : (
                                        <details className="group">
                                            <summary className="flex cursor-pointer list-none items-center gap-2 [&::-webkit-details-marker]:hidden">
                                                <span className="shrink-0">{contextKeyButton(key)}</span>
                                                <span className="min-w-0 truncate text-foreground/80 group-open:hidden">{formatValue(value)}</span>
                                                <ChevronRight aria-hidden="true" className="size-3 shrink-0 transition-transform group-open:rotate-90" />
                                            </summary>
                                            <pre className="mt-1.5 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-background px-3 py-2 text-xs text-foreground/90">
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
                </Well>
            ) : null}

            {visit.parentKey === null || (flowNode && flowNode.readsContext.length > 0) ? (
                <section className="flex flex-col gap-2">
                    {visit.parentKey === null ? (
                        <details data-testid="run-visit-instructions" className="group">
                            <summary className="flex min-w-0 cursor-pointer list-none items-center gap-2 text-sm [&::-webkit-details-marker]:hidden">
                                <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
                                <span className="shrink-0 font-semibold text-foreground">Instructions</span>
                                {prompt ? (
                                    <span data-testid="run-visit-instructions-preview" className="min-w-0 truncate text-muted-foreground group-open:hidden">
                                        {prompt.replace(/\s+/g, ' ').slice(0, 180)}{prompt.length > 180 ? '…' : ''}
                                    </span>
                                ) : null}
                            </summary>
                            <div className="mt-2 max-h-96 max-w-[72ch] overflow-y-auto whitespace-pre-wrap break-words pl-5.5 text-sm leading-6 text-foreground/90">
                                {prompt ?? <span className="text-muted-foreground">No instructions were recorded for this visit.</span>}
                            </div>
                        </details>
                    ) : null}
                    {flowNode && flowNode.readsContext.length > 0 ? (
                        <p data-testid="run-visit-reads" className={cn('flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground', visit.parentKey === null && 'pl-5.5')}>
                            Reads
                            {flowNode.readsContext.map((key) => (
                                <span key={key}>{contextKeyButton(key)}</span>
                            ))}
                        </p>
                    ) : null}
                </section>
            ) : null}

            <section data-testid="run-visit-work" className="flex flex-col gap-3">
                <SectionHeading
                    as="h4"
                    action={hasThinking ? <ShowThinkingToggle pressed={showThinking} onPressedChange={setShowThinking} /> : null}
                >
                    {workTitle}
                </SectionHeading>
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
                                <p className="max-w-[72ch] whitespace-pre-wrap text-foreground/90">{visit.childRun.summary}</p>
                            ) : null}
                        </div>
                    ) : (
                        <p className="text-sm text-muted-foreground">No child run was recorded for this visit.</p>
                    )
                ) : rows.length > 0 ? (
                    <RunTranscript data-testid="run-visit-transcript" rows={rows} showThinking={showThinking} />
                ) : (
                    <p className="text-sm text-muted-foreground">No transcript was recorded for this visit.</p>
                )}
                {returned ? (
                    <div data-testid="run-visit-ended" className="flex items-center gap-3 pt-1 text-xs text-muted-foreground">
                        <span aria-hidden="true" className="h-px flex-1 bg-border" />
                        Visit ended · {outcomeLabel}{duration ? ` · ${duration}` : ''}
                        <span aria-hidden="true" className="h-px flex-1 bg-border" />
                    </div>
                ) : null}
            </section>
            {files.length > 0 ? (
                <p data-testid="run-visit-files" className="text-xs text-muted-foreground">
                    Files:{' '}
                    {files.map((entry, index) => (
                        <span key={entry.path}>
                            {index > 0 ? ' · ' : ''}
                            <button
                                type="button"
                                data-testid="run-visit-file"
                                data-path={entry.path}
                                onClick={() => onViewArtifact(entry)}
                                className="hover:text-foreground hover:underline hover:underline-offset-4"
                            >
                                {entry.path.slice(filesDir.length).replace(/\.\w+$/, '')}
                            </button>
                        </span>
                    ))}
                </p>
            ) : null}
        </article>
    )
}

function ToolWork({ visit, command }: { visit: RunVisit; command: string | null }) {
    const { output, exitCode } = visitToolResult(visit)
    return (
        <div data-testid="run-visit-tool" className="space-y-2 text-sm">
            {command ? <pre className="overflow-x-auto whitespace-pre-wrap rounded-lg bg-muted px-3 py-2 font-mono text-xs text-foreground">{command}</pre> : null}
            {output ? (
                <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted px-3 py-2 font-mono text-xs text-foreground/90">{output}</pre>
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
        <section data-testid="run-journal-panel" className="flex flex-col gap-3">
            <h3 className="text-lg font-semibold text-foreground">Journal</h3>
            {timelineError ? <InlineError data-testid="run-event-timeline-error">{timelineError}</InlineError> : null}
            <ol className="flex flex-col gap-1 text-sm">
                {journal.map((event) => (
                    <li key={event.id} data-testid="run-journal-row" className="flex gap-3">
                        <span className="shrink-0 text-xs leading-5 tabular-nums text-muted-foreground">{formatTimestamp(event.receivedAt)}</span>
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
