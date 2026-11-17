import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from 'react'
import { ArrowRight, ChevronRight, CircleHelp, Minus, Undo2, Workflow, X, type LucideIcon } from 'lucide-react'

import type { RunExecutionPrompt, RunTranscriptSegment } from '@/lib/api/attractorApi'
import { useStore } from '@/store'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { SegmentedControl } from '@/components/app/segmented-control'
import { StatusDot, StatusPill, statusTextClassName, type StatusTone } from '@/components/app/status'
import { LiveThinkingIndicator } from '@/components/app/transcript/liveThinking'
import { ShowThinkingToggle } from '@/components/app/transcript/SegmentRows'
import { Transcript } from '@/components/app/transcript/Transcript'
import { isThinkingEntry } from '@/features/projects/model/conversationTimeline'
import { Well } from '@/components/app/well'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { formatTimestamp, type ArtifactListEntry, type TimelineEventEntry } from '../model/shared'
import { journalEntriesShown, journalEntrySummary } from '../model/timelineModel'
import {
    VISIT_OUTCOME_LABELS,
    formatVisitDuration,
    formatVisitErrorKind,
    humanizeNodeId,
    parallelBranchResults,
    parseBranchFailure,
    visitContextWrites,
    visitKey,
    visitDurationMs,
    visitMark,
    visitPrompt,
    visitToolResult,
    visitTranscriptEntries,
    type RunVisit,
    type VisitFlowNode,
    type VisitMark,
} from '../model/visitModel'
import { describeFailure, formatRunTime } from '../model/runOverviewModel'
import { useRunActions, type SteerResult } from '../hooks/useRunActions'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { FailureDetail } from './RunStatusItem'

// Colour marks exceptions: a completed visit carries no icon, an error a warning cross, a
// failure a destructive cross. A loop is how a flow works, not a problem: its return arrow is muted.
const MARKS: Record<VisitMark, { Icon: LucideIcon; label: string; tone: StatusTone }> = {
    error: { Icon: X, label: 'Error', tone: 'warning' },
    failed: { Icon: X, label: 'Failed', tone: 'destructive' },
    interrupted: { Icon: Minus, label: 'Interrupted', tone: 'warning' },
    waiting: { Icon: CircleHelp, label: 'Waiting on a question', tone: 'warning' },
    loop_back: { Icon: Undo2, label: 'Looped back', tone: 'muted' },
}

/** How a visit led to the next one, when that was not simply the next step. */
const nextNote = (visit: RunVisit, next: RunVisit | null): string | null => (
    !next ? null : visit.next?.retry ? 'Retried' : visit.next?.loopBack ? `Looped back to ${next.label}` : null
)

const OUTCOME_TONES: Record<RunVisit['outcome'], StatusTone> = {
    completed: 'success',
    error: 'warning',
    failed: 'destructive',
    interrupted: 'warning',
    running: 'info',
    waiting: 'warning',
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

const FOLLOW_THRESHOLD_PX = 8

const scrollParent = (node: HTMLElement | null): HTMLElement | null => {
    for (let element = node?.parentElement ?? null; element; element = element.parentElement) {
        if (/auto|scroll/.test(getComputedStyle(element).overflowY)) return element
    }
    return null
}

/**
 * A running step's work keeps arriving below what has been read. A reader at its end stays at
 * its end as it grows, as in a chat; one who scrolls up, or on past it to the Steps list, stays put.
 */
function useFollowOutput(ref: RefObject<HTMLElement | null>, active: boolean) {
    const pinned = useRef(false)
    // How far the end of the work is below the bottom of the view.
    const gap = (scroller: HTMLElement, node: HTMLElement) => node.getBoundingClientRect().bottom - scroller.getBoundingClientRect().bottom
    useEffect(() => {
        const node = ref.current
        const scroller = scrollParent(node)
        if (!active || !node || !scroller) return
        const update = () => {
            const below = gap(scroller, node)
            pinned.current = below <= FOLLOW_THRESHOLD_PX && (below >= -FOLLOW_THRESHOLD_PX || scroller.scrollHeight <= scroller.clientHeight)
        }
        update()
        scroller.addEventListener('scroll', update, { passive: true })
        return () => scroller.removeEventListener('scroll', update)
    }, [ref, active])
    useLayoutEffect(() => {
        const node = ref.current
        const scroller = active && pinned.current ? scrollParent(node) : null
        if (!node || !scroller) return
        const below = gap(scroller, node)
        if (below > 0) scroller.scrollTop += below
    })
}

interface RunVisitsCardProps {
    visits: RunVisit[]
    flowNodes: Record<string, VisitFlowNode>
    segments: RunTranscriptSegment[]
    prompts: RunExecutionPrompt[]
    journal: TimelineEventEntry[]
    now: number
    isNarrowViewport: boolean
    isLive: boolean
    /** Why the journal couldn't load; the steps it lists are then unknown, not none. */
    timelineError: string | null
    /** The journal hasn't loaded yet, so no steps says nothing. */
    stepsLoading?: boolean
    /** The node selected on the graph; its visits are highlighted. */
    selectedNodeId: string | null
    onSelectNode: (nodeId: string | null) => void
    onOpenRun: (runId: string) => void
    /** The status item's row: what it says and whether it needs attention. */
    statusRow: { label: string; tone?: StatusTone; detail?: string }
    /** How many keys the Context item lists; null when the context couldn't load, 'loading' until it has. */
    contextKeyCount: number | null | 'loading'
    renderStatus: (selectVisit: (visit: RunVisit) => void) => ReactNode
    renderContext: (focusKey: string | null, selectVisit: (visit: RunVisit) => void) => ReactNode
    artifactEntries: ArtifactListEntry[]
    onViewArtifact: (entry: ArtifactListEntry) => void
    /** The run graph toggle, shown in the rail's view control. */
    graph?: { shown: boolean; onToggle: () => void }
    /** The run graph, shown above the details so the rail keeps its full height beside both. */
    graphPane?: ReactNode
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
    timelineError,
    stepsLoading = false,
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
    graphPane,
}: RunVisitsCardProps) {
    // An exit node that simply completed says nothing the status item doesn't.
    const visits = useMemo(
        () => allVisits.filter((visit) => visit.kind !== 'exit' || visit.outcome !== 'completed'),
        [allVisits],
    )
    const stepsUnavailable = Boolean(timelineError) && visits.length === 0
    // The status item leads and is selected when a run opens.
    const [selectedKey, setSelectedKey] = useState<string>('status')
    const [contextFocusKey, setContextFocusKey] = useState<string | null>(null)
    const [showJournal, setShowJournal] = useState(false)
    const listRef = useRef<HTMLDivElement | null>(null)
    const detailsRef = useRef<HTMLDivElement | null>(null)

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
    // On a phone the details sit above the rail, so a tapped row brings them into view.
    const pick = (item: RunVisit | ItemKey) => {
        select(item)
        if (isNarrowViewport) detailsRef.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
    }
    const openContextKey = (key: string) => {
        select('context')
        setContextFocusKey(key)
    }
    // A link inside the details replaces them, so focus follows to the new view's heading
    // instead of falling to the page.
    const [focusRequest, setFocusRequest] = useState(0)
    const openFromDetails = (item: RunVisit | ItemKey) => {
        select(item)
        setFocusRequest((count) => count + 1)
    }
    useLayoutEffect(() => {
        if (focusRequest > 0) detailsRef.current?.querySelector<HTMLElement>('h2[tabindex]')?.focus()
    }, [focusRequest])

    // Whatever picked it (the list, a link in the details, the graph), the selected row shows in
    // the list, which scrolls itself only; and the details open at their top, not where the last
    // ones were left. On a phone the page scrolls, and a pick brings the details into view itself.
    const shownKey = showJournal ? 'journal' : itemKey(selected)
    const columnRef = useRef<HTMLDivElement | null>(null)
    useLayoutEffect(() => {
        const list = listRef.current
        const row = list?.querySelector<HTMLElement>('[aria-selected="true"]')
        if (list && row) {
            const listBox = list.getBoundingClientRect()
            const rowBox = row.getBoundingClientRect()
            if (rowBox.top < listBox.top || rowBox.bottom > listBox.bottom) {
                list.scrollTop += rowBox.top - listBox.top - (listBox.height - rowBox.height) / 2
            }
        }
        const details = detailsRef.current
        const scroller = isNarrowViewport ? null : graphPane ? columnRef.current : details
        if (details && scroller) {
            const top = scroller === details ? 0 : details.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop
            if (scroller.scrollTop > top) scroller.scrollTop = top
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps -- only a new selection moves them
    }, [shownKey])

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

    // On a phone the details lead (a question to answer, a result, why it failed); the steps follow.
    // Under the graph they scroll with it as one column, so they get its full height to read in.
    const details = (
        <div ref={detailsRef} data-testid="run-visit-details" className={cn('min-w-0 flex-1', isNarrowViewport ? 'order-first scroll-mt-16' : graphPane ? 'shrink-0' : 'min-h-0 overflow-y-auto')}>
            {showJournal ? (
                <RunJournal journal={journal} flowNodes={flowNodes} timelineError={timelineError} />
            ) : selected === 'status' ? (
                renderStatus(openFromDetails)
            ) : selected === 'context' ? (
                <div key={contextFocusKey ?? ''}>{renderContext(contextFocusKey, openFromDetails)}</div>
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
                    onSelectVisit={openFromDetails}
                    onOpenContextKey={openContextKey}
                    onOpenRun={onOpenRun}
                />
            )}
        </div>
    )

    return (
        <section
            data-testid="run-visits-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn('flex gap-6', isNarrowViewport ? 'flex-col' : 'h-full min-h-0')}
        >
            <Well className={cn('flex shrink-0 flex-col gap-0.5 p-2', isNarrowViewport ? undefined : 'min-h-0 w-64 xl:w-72')}>
                {/* Steps and Journal exclude each other; the graph is additive, so it is its own
                    one-segment track beside them (a pressed segment beside a pressed segment reads wrong). */}
                <div className="mb-1.5 flex items-center gap-1.5">
                    {/* As wide as it is in the 16rem side rail, wherever the rail sits or however wide, so the
                        Graph toggle stays put and the segments never stretch across a wide rail. */}
                    <SegmentedControl
                        aria-label="View"
                        className="min-w-0 max-w-[9.75rem] flex-1"
                        options={[
                            { key: 'steps', label: 'Steps', pressed: !showJournal, onPress: () => setShowJournal(false) },
                            {
                                key: 'journal',
                                label: 'Journal',
                                pressed: showJournal,
                                onPress: () => {
                                    setShowJournal((current) => !current)
                                    if (isNarrowViewport && !showJournal) detailsRef.current?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
                                },
                                'data-testid': 'run-journal-toggle',
                            },
                        ]}
                    />
                    {graph ? (
                        <SegmentedControl
                            aria-label="Graph"
                            className="flex-none"
                            options={[{
                                key: 'graph',
                                label: <><Workflow className="size-3.5" />Graph</>,
                                pressed: graph.shown,
                                onPress: graph.onToggle,
                                'aria-keyshortcuts': 'g',
                                title: 'Show or hide the run graph (g)',
                                'data-testid': 'run-graph-toggle',
                            }]}
                        />
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
                <div
                    ref={listRef}
                    role="listbox"
                    aria-label="Visits"
                    data-own-keys="jk"
                    data-testid="run-visit-list"
                    onKeyDown={onListKeyDown}
                    className={cn('space-y-0.5 overflow-y-auto', isNarrowViewport ? 'max-h-72' : 'min-h-0 flex-1')}
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
                            onClick={() => pick(item)}
                            className={cn(railRowClass(!showJournal && selected === item), 'h-8.5')}
                        >
                            <span className="flex w-3.5 shrink-0 justify-center">
                                {item === 'status' && statusRow.tone ? <StatusDot tone={statusRow.tone} /> : null}
                            </span>
                            <span className={cn('min-w-0 flex-1 truncate', item === 'status' && statusRow.tone ? statusTextClassName(statusRow.tone) : undefined)}>
                                {item === 'status' ? statusRow.label : 'Context'}
                            </span>
                            <span className="shrink-0 text-right text-xs tabular-nums text-muted-foreground">
                                {item === 'status' ? statusRow.detail : contextKeyCount === null ? 'Not loaded' : contextKeyCount === 'loading' ? 'Loading…' : `${contextKeyCount} ${contextKeyCount === 1 ? 'key' : 'keys'}`}
                            </span>
                        </button>
                    ))}
                    <div className="flex items-center justify-between gap-2 px-2.5 pb-1 pt-3 text-xs font-semibold text-muted-foreground">
                        <span>Steps</span>
                        <span className="font-normal">
                            {stepsUnavailable || (stepsLoading && visits.length === 0) ? null : <>{visits.length} {visits.length === 1 ? 'visit' : 'visits'}</>}
                            {isLive ? <span className="ml-1.5 text-info">Live</span> : null}
                        </span>
                    </div>
                    {stepsUnavailable ? (
                        <p data-testid="run-visits-unavailable" className="px-2.5 py-2 text-sm text-muted-foreground">
                            The steps couldn’t load.
                        </p>
                    ) : visits.length === 0 && stepsLoading ? (
                        <p data-testid="run-visits-loading" className="px-2.5 py-2 text-sm text-muted-foreground" aria-live="polite">
                            Loading the steps…
                        </p>
                    ) : visits.length === 0 ? (
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
                        const note = nextNote(visit, visits.find((candidate) => candidate.key === visit.next?.key) ?? null)
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
                                onClick={() => pick(visit)}
                                className={cn(
                                    railRowClass(isSelected),
                                    note ? 'items-start py-1.5' : 'min-h-8 py-1',
                                    visit.parentKey && 'pl-6',
                                    visit.nodeId === selectedNodeId && !isSelected && 'bg-background/40',
                                )}
                            >
                                <span className={cn('flex w-3.5 shrink-0 justify-center', note && 'mt-[3px]')} data-testid="run-visit-row-marks">
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
                                    ) : visit.outcome === 'running' ? (
                                        <span role="img" title="Running" aria-label="Running" data-testid="run-visit-row-running" className="flex">
                                            <StatusDot tone="info" className="animate-pulse" />
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
                                    {note ? (
                                        <span data-testid="run-visit-row-next-note" className="break-words text-xs font-normal text-muted-foreground">
                                            {note}
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
            {graphPane ? (
                <div ref={columnRef} className="flex min-h-0 min-w-0 flex-1 flex-col gap-5 overflow-y-auto">
                    <div data-testid="run-graph-pane" className="flex h-[60%] min-h-96 shrink-0 flex-col">{graphPane}</div>
                    {details}
                </div>
            ) : details}
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
    const showThinking = useStore((state) => state.preferredShowThinking)
    const setShowThinking = useStore((state) => state.setPreferredShowThinking)
    const articleRef = useRef<HTMLElement | null>(null)
    useFollowOutput(articleRef, visit.outcome === 'running')
    const nodeVisits = visits.filter((candidate) => candidate.nodeId === visit.nodeId)
    const next = visits.find((candidate) => candidate.key === visit.next?.key) ?? null
    const prompt = visitPrompt(visit, prompts)
    const entries = useMemo(() => visitTranscriptEntries(visit, segments), [visit, segments])
    const hasThinking = entries.some(isThinkingEntry)
    const { writes } = visitContextWrites(visit)
    const duration = formatVisitDuration(visitDurationMs(visit, now))
    const errorKind = formatVisitErrorKind(visit)
    const branchResults = parallelBranchResults(visit)
    const branchVisits = visits.filter((candidate) => candidate.parentKey === visit.key)
    const outcomeLabel = VISIT_OUTCOME_LABELS[visit.outcome]
    const returned = visit.outcome !== 'running' && visit.outcome !== 'waiting'
    // A tool's notes are its stdout, which shows once, under its command.
    const reason = visit.kind === 'tool' && visit.reason === visitToolResult(visit).output ? null : visit.reason
    // A fork's error names the branch that ended it; its branch list says which and why.
    const failure = reason && !(branchResults.length > 0 && parseBranchFailure(reason))
        ? describeFailure(reason, (nodeId) => visits.find((candidate) => candidate.nodeId === nodeId)?.label ?? humanizeNodeId(nodeId))
        : null
    const headerMeta = [visit.count > 1 ? `Visit ${visit.number} of ${visit.count}` : null, duration || null]
        .filter((part): part is string => part !== null)
        .join(' · ')
    const filesDir = `logs/${visit.nodeId}/executions/${visit.stageIndex}-${visit.attempt}/`
    const files = visit.executionIdentity === null
        ? []
        : VISIT_FILES.flatMap((name) => artifactEntries.filter((entry) => entry.path === filesDir + name))
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
    // Only agents, tools and subflows do work of their own; a gate, switch or fan-out has only its result.
    const hasWork = visit.kind === 'agent_task' || visit.kind === 'tool' || visit.kind === 'subflow' || entries.length > 0

    return (
        <article ref={articleRef} data-testid="run-visit-view" data-visit-key={visit.key} className="flex flex-col gap-5 pb-6">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
                    <h2 data-testid="run-visit-view-title" tabIndex={-1} className="rounded-sm text-base font-semibold text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring">{visit.label}</h2>
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
                                'aria-label': `Visit ${candidate.number} of ${candidate.count}${candidate.next?.loopBack ? ', looped back' : candidate.next?.retry ? ', retried' : ''}`,
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

            {!returned && branchVisits.length > 0 ? (
                <section className="flex flex-col gap-2">
                    <SectionHeading as="h3" count={branchVisits.length}>Branches</SectionHeading>
                    <BranchVisits branchVisits={branchVisits} onSelectVisit={onSelectVisit} />
                </section>
            ) : null}

            {returned ? (
                <Well data-testid="run-visit-returned" className="flex flex-col gap-2.5">
                    <div className="flex flex-wrap items-center gap-2.5">
                        <SectionHeading as="h3">Returned</SectionHeading>
                        {outcomePill}
                        {errorKind ? <span data-testid="run-visit-error-kind" className="text-sm text-muted-foreground">{errorKind}</span> : null}
                        {visit.decision ? (
                            <span data-testid="run-visit-decision" className="text-sm text-muted-foreground">
                                {/* A decision reads as its route is labelled, as a gate offered it; the ID stays a hover away. */}
                                {visit.kind === 'human_gate' ? 'Answer' : 'Decision'}{' '}
                                {flowNode?.routeLabels?.[visit.decision]
                                    ? <span title={visit.decision} className="font-medium text-foreground">{flowNode.routeLabels[visit.decision]}</span>
                                    : <code className="font-mono text-foreground">{visit.decision}</code>}
                            </span>
                        ) : null}
                        {next ? (
                            <span className="inline-flex items-center gap-1 text-sm text-muted-foreground">
                                {visit.next?.retry ? 'Retried as' : visit.next?.loopBack ? 'Looped back to' : 'Next'}
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
                    {/* A step's own notes are Markdown, as its agent wrote them; a person's gate note and an error read as written. */}
                    {failure && visit.outcome === 'completed' && !failure.detail && visit.kind !== 'human_gate' ? (
                        <div data-testid="run-visit-reason" className="max-w-[72ch] text-sm text-foreground">
                            <ProjectConversationMarkdown content={failure.summary} keepLineBreaks />
                        </div>
                    ) : failure ? (
                        <p data-testid="run-visit-reason" className="max-w-[72ch] whitespace-pre-wrap break-words text-sm leading-6 text-foreground">{failure.summary}</p>
                    ) : null}
                    {/* The runtime's own words are for whoever debugs the flow. */}
                    {failure?.detail ? (
                        <FailureDetail data-testid="run-visit-reason-detail" className="max-w-[72ch]">{failure.detail}</FailureDetail>
                    ) : null}
                    {branchResults.length > 0 ? (
                        <BranchResults results={branchResults} branchVisits={branchVisits} onSelectVisit={onSelectVisit} />
                    ) : branchVisits.length > 0 ? (
                        <BranchVisits branchVisits={branchVisits} onSelectVisit={onSelectVisit} />
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
                </Well>
            ) : null}

            <section className="flex flex-col gap-2 empty:hidden">
                {/* Only an agent is given instructions; another step shows them only when some were recorded. */}
                {prompt || visit.kind === 'agent_task' ? <details data-testid="run-visit-instructions" className="group">
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
                </details> : null}
                {flowNode && flowNode.readsContext.length > 0 ? (
                    <p data-testid="run-visit-reads" className="flex flex-wrap items-center gap-1.5 pl-5.5 text-xs text-muted-foreground">
                        Reads
                        {flowNode.readsContext.map((key) => (
                            <span key={key}>{contextKeyButton(key)}</span>
                        ))}
                    </p>
                ) : null}
            </section>

            {hasWork ? <section data-testid="run-visit-work" className="flex flex-col gap-3">
                <SectionHeading
                    as="h3"
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
                ) : (
                    <Transcript
                        data-testid="run-visit-transcript"
                        testIdPrefix="run"
                        turns={false}
                        entries={entries}
                        showThinking={showThinking}
                        formatTimestamp={formatTimestamp}
                        empty="No transcript was recorded for this visit."
                        className="max-w-[80ch]"
                    />
                )}
                {visit.kind === 'agent_task' && visit.outcome === 'running' && visit.executionIdentity !== null ? (
                    <LiveThinkingIndicator
                        thinkingKey={visitKey(visit.runId, visit.nodeId, visit.stageIndex, visit.attempt)}
                        testId="run-visit-thinking"
                    />
                ) : null}
                {visit.kind === 'agent_task' ? (
                    <RunSteerForm runId={visit.runId} nodeId={visit.nodeId} running={visit.outcome === 'running'} />
                ) : null}
                {returned ? (
                    <div data-testid="run-visit-ended" className="flex items-center gap-3 pt-1 text-xs text-muted-foreground">
                        <span aria-hidden="true" className="h-px flex-1 bg-border" />
                        Visit ended · {outcomeLabel}{duration ? ` · ${duration}` : ''}
                        <span aria-hidden="true" className="h-px flex-1 bg-border" />
                    </div>
                ) : null}
            </section> : null}
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

const BRANCH_STATUS: Record<string, { label: string; tone: StatusTone }> = {
    completed: { label: 'Completed', tone: 'success' },
    // An error the run went past, as its step in Steps says it.
    error: { label: VISIT_OUTCOME_LABELS.error, tone: OUTCOME_TONES.error },
    terminal_result: { label: 'Reached an exit', tone: 'warning' },
    // Its fork or the user's cancel stopped it, as its step in Steps says.
    canceled: { label: 'Interrupted', tone: 'warning' },
}

const BRANCH_OPEN_CLASS = 'rounded-sm font-medium text-foreground underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring'

// A fan-out's branch steps as Steps lists them, for a fork with no results recorded: one still
// running, or one a cancel stopped.
function BranchVisits({ branchVisits, onSelectVisit }: { branchVisits: RunVisit[]; onSelectVisit: (visit: RunVisit) => void }) {
    return (
        <ul data-testid="run-visit-branches" className="flex flex-col gap-1.5 text-sm">
            {branchVisits.map((branch) => (
                <li key={branch.key} data-testid="run-visit-branch" data-status={branch.outcome} className="flex flex-wrap items-center gap-2">
                    <button type="button" data-testid="run-visit-branch-open" onClick={() => onSelectVisit(branch)} className={BRANCH_OPEN_CLASS}>
                        {branch.label}
                    </button>
                    <StatusPill tone={OUTCOME_TONES[branch.outcome]}>{VISIT_OUTCOME_LABELS[branch.outcome]}</StatusPill>
                </li>
            ))}
        </ul>
    )
}

// Each declared branch of a fan-out by its step's name, which opens that step, with the
// evidence for any branch that did not complete.
function BranchResults({ results, branchVisits, onSelectVisit }: {
    results: ReturnType<typeof parallelBranchResults>
    branchVisits: RunVisit[]
    onSelectVisit: (visit: RunVisit) => void
}) {
    return (
        <ul data-testid="run-visit-branches" className="flex flex-col gap-1.5 text-sm">
            {results.map((branch) => {
                const branchVisit = branchVisits.find((candidate) => candidate.nodeId === branch.id) ?? null
                const name = branchVisit?.label ?? humanizeNodeId(branch.id)
                const error = branch.error
                const terminal = branch.terminal
                const errorVisit = error ? branchVisits.find((candidate) => candidate.nodeId === (error.node ?? branch.id)) : undefined
                // The branch error that ended the run is a failure, as its step in Steps says.
                const status = errorVisit?.outcome === 'failed' ? { label: VISIT_OUTCOME_LABELS.failed, tone: OUTCOME_TONES.failed }
                    : BRANCH_STATUS[branch.status] ?? { label: branch.status, tone: 'muted' as StatusTone }
                const failure = error?.reason ? describeFailure(error.reason) : null
                // Its error's kind and, past the branch's first step, where: "runtime error at Tsc · 2 attempts".
                const at = error?.node && error.node !== branch.id ? `at ${errorVisit?.label ?? humanizeNodeId(error.node)}` : null
                const evidence = error
                    ? [
                        [formatVisitErrorKind({ failureKind: error.kind }), at].filter(Boolean).join(' '),
                        error.attempts !== null ? `${error.attempts} ${error.attempts === 1 ? 'attempt' : 'attempts'}` : null,
                    ].filter(Boolean).join(' · ')
                    : terminal
                        ? [terminal.name ? `result ${terminal.name}` : null, terminal.outcome, terminal.node ? `from ${terminal.node}` : null].filter(Boolean).join(' · ')
                        : null
                return (
                    <li key={`${branch.index}-${branch.id}`} data-testid="run-visit-branch" data-status={branch.status} className="flex max-w-[72ch] flex-col gap-0.5">
                        <span className="flex flex-wrap items-center gap-2">
                            {branchVisit ? (
                                <button type="button" data-testid="run-visit-branch-open" onClick={() => onSelectVisit(branchVisit)} className={BRANCH_OPEN_CLASS}>
                                    {name}
                                </button>
                            ) : <span className="font-medium text-foreground">{name}</span>}
                            <StatusPill tone={status.tone}>{status.label}</StatusPill>
                            {evidence ? <span data-testid="run-visit-branch-evidence" className="text-muted-foreground">{evidence}</span> : null}
                        </span>
                        {failure ? (
                            <span data-testid="run-visit-branch-reason" className="whitespace-pre-wrap break-words text-foreground/90">{failure.summary}</span>
                        ) : null}
                        {failure?.detail ? (
                            <FailureDetail data-testid="run-visit-branch-reason-detail">{failure.detail}</FailureDetail>
                        ) : null}
                    </li>
                )
            })}
        </ul>
    )
}

// Sends a message to the running agent node. Every outcome stays visible: a
// rejection or HTTP error keeps the draft so it can be resent, and the last
// result outlives the visit ending, while the input does not.
function RunSteerForm({ runId, nodeId, running }: { runId: string; nodeId: string; running: boolean }) {
    const { sendSteer } = useRunActions()
    const [draft, setDraft] = useState('')
    const [sending, setSending] = useState(false)
    const [result, setResult] = useState<SteerResult | null>(null)
    const send = async () => {
        setSending(true)
        const next = await sendSteer(runId, nodeId, draft)
        setResult(next)
        if (next.delivered) {
            setDraft('')
        }
        setSending(false)
    }
    if (!running && !result) {
        return null
    }
    return (
        <form
            data-testid="run-visit-steer"
            className="flex flex-col gap-2"
            onSubmit={(event) => {
                event.preventDefault()
                void send()
            }}
        >
            {running ? (
                <Textarea
                    data-testid="run-visit-steer-input"
                    aria-label="Message to the running agent"
                    placeholder="Message the running agent…"
                    value={draft}
                    onChange={(event) => setDraft(event.target.value)}
                    disabled={sending}
                    className="bg-background"
                />
            ) : null}
            <div className="flex flex-wrap items-start gap-3">
                {running ? (
                    <Button
                        type="submit"
                        data-testid="run-visit-steer-send"
                        variant="outline"
                        size="sm"
                        disabled={sending || draft.trim().length === 0}
                    >
                        Send
                    </Button>
                ) : null}
                {result ? (
                    result.delivered ? (
                        <p data-testid="run-visit-steer-delivered" role="status" className="text-sm text-muted-foreground">{result.text}</p>
                    ) : (
                        <InlineError data-testid="run-visit-steer-rejected" dense className="flex-1">{result.text}</InlineError>
                    )
                ) : null}
            </div>
        </form>
    )
}

function ToolWork({ visit, command }: { visit: RunVisit; command: string | null }) {
    const { output, exitCode } = visitToolResult(visit)
    // A failed command's error is at the end of its output, so the pane opens there.
    const failed = visit.outcome === 'failed' || visit.outcome === 'error' || Boolean(exitCode)
    const outputRef = useRef<HTMLPreElement | null>(null)
    useLayoutEffect(() => {
        if (failed && outputRef.current) outputRef.current.scrollTop = outputRef.current.scrollHeight
    }, [failed, output])
    return (
        <div data-testid="run-visit-tool" className="space-y-3 text-sm">
            {/* The section heading names the command; its output is labelled and drawn differently,
                so the two never read as one block. */}
            {command ? (
                <pre className="overflow-x-auto whitespace-pre-wrap rounded-lg border border-border bg-background px-3 py-2 font-mono text-xs text-foreground">{command}</pre>
            ) : null}
            {output || exitCode !== null ? (
                <div className="space-y-1">
                    <div className="flex items-baseline justify-between gap-3 text-xs">
                        <span className="font-semibold text-muted-foreground">Output</span>
                        {exitCode !== null ? (
                            <span className={exitCode === 0 ? 'text-muted-foreground' : 'font-medium text-warning'}>Exit code {exitCode}</span>
                        ) : null}
                    </div>
                    {output ? (
                        <pre
                            ref={outputRef}
                            data-testid="run-visit-tool-output"
                            role="region"
                            aria-label="Command output"
                            tabIndex={0}
                            className="max-h-96 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted px-3 py-2 font-mono text-xs text-foreground/90 outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        >
                            {output}
                        </pre>
                    ) : null}
                </div>
            ) : null}
            {!command && !output && exitCode === null ? (
                <p className="text-muted-foreground">No command output was recorded for this visit.</p>
            ) : null}
        </div>
    )
}

// The journal names the day once and each entry by its time of day, as the run list does.
// An entry's first line is its news; an error's further lines, such as a command's output, are detail.
function JournalEntryText({ line, detail, className }: { line: string; detail: string; className?: string }) {
    return (
        <div className="flex min-w-0 flex-col gap-1">
            <span className={cn('break-words', className)}>{line}</span>
            {detail ? <FailureDetail>{detail}</FailureDetail> : null}
        </div>
    )
}

const JOURNAL_DAY = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' })

function RunJournal({ journal, flowNodes, timelineError }: {
    journal: TimelineEventEntry[]
    flowNodes: Record<string, VisitFlowNode>
    timelineError: string | null
}) {
    // Steps read as the Steps list names them; a child flow's nodes are not in this flow.
    const nodeLabel = (nodeId: string, scope: TimelineEventEntry['sourceScope']) => (
        (scope === 'root' ? flowNodes[nodeId]?.label : null) ?? humanizeNodeId(nodeId)
    )
    // Checkpoints and the run's recorded details are the engine's bookkeeping, for whoever debugs a
    // run; they show when asked for, so the steps, routes and answers read without them.
    const [showRecords, setShowRecords] = useState(false)
    const isRecord = (event: TimelineEventEntry) => event.category === 'checkpoint' || event.category === 'metadata'
    const allEntries = journalEntriesShown(journal)
    const hasRecords = allEntries.some(isRecord)
    const entries = showRecords ? allEntries : allEntries.filter((event) => !isRecord(event))
    // A run that fails with its step's error repeats the step's detail; it shows once, under the step.
    const shownDetails = new Set<string>()
    let previousDay = ''
    let afterCancel = false
    return (
        <section data-testid="run-journal-panel" className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 className="text-base font-semibold text-foreground">Journal</h2>
                {hasRecords ? (
                    <Button
                        type="button"
                        variant="ghost"
                        size="xs"
                        data-testid="run-journal-records-toggle"
                        aria-pressed={showRecords}
                        onClick={() => setShowRecords(!showRecords)}
                        className={cn('text-muted-foreground', showRecords && 'text-foreground')}
                    >
                        {showRecords ? 'Hide checkpoints' : 'Show checkpoints'}
                    </Button>
                ) : null}
            </div>
            {timelineError ? <InlineError data-testid="run-event-timeline-error">{timelineError}</InlineError> : null}
            <ol className="flex flex-col gap-1 text-sm">
                {entries.map((event) => {
                    const date = new Date(event.receivedAt)
                    const valid = !Number.isNaN(date.getTime())
                    const day = valid ? JOURNAL_DAY.format(date) : ''
                    const startsDay = day !== previousDay
                    previousDay = day
                    afterCancel ||= event.type === 'CancelRequested'
                    const [line, ...more] = journalEntrySummary(event, nodeLabel, afterCancel).split('\n')
                    const detail = more.join('\n')
                    const repeated = shownDetails.has(detail)
                    if (detail) shownDetails.add(detail)
                    return (
                        <Fragment key={event.id}>
                            {startsDay && day ? (
                                <li className="pb-1 pt-3 text-xs font-semibold text-muted-foreground first:pt-0">{day}</li>
                            ) : null}
                            <li data-testid="run-journal-row" className="flex gap-3">
                                <span className="w-20 shrink-0 text-xs leading-5 tabular-nums text-muted-foreground" title={formatTimestamp(event.receivedAt)}>
                                    {valid ? formatRunTime(event.receivedAt) : '—'}
                                </span>
                                <JournalEntryText
                                    line={line}
                                    detail={repeated ? '' : detail}
                                    className={event.severity === 'error' ? 'text-destructive' : event.severity === 'warning' ? 'text-warning' : undefined}
                                />
                            </li>
                        </Fragment>
                    )
                })}
            </ol>
        </section>
    )
}
