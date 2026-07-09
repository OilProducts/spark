import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import type { RunTranscriptSegment } from '@/lib/api/attractorApi'

import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { NativeSelect } from '@/components/ui/native-select'
import { cn } from '@/lib/utils'
import type {
    GroupedTimelineEntry,
    TimelineEventCategory,
    TimelineEventEntry,
    TimelineSeverity,
} from '../model/shared'
import { buildRunTranscriptGroups } from '../model/transcriptModel'
import type { RunTranscriptGroup } from '../model/transcriptModel'
import { RunTranscriptGroupSection, useTranscriptExpansion } from './RunTranscriptGroups'
import {
    TIMELINE_CATEGORY_LABELS,
    TIMELINE_SEVERITY_LABELS,
    TIMELINE_SEVERITY_STYLES,
    formatTimestamp,
    humanizeTimelineType,
} from '../model/shared'

export type RunActivityMode = 'all' | 'transcript' | 'events'

const ACTIVITY_MODE_OPTIONS: Array<{ value: RunActivityMode; label: string }> = [
    { value: 'all', label: 'All' },
    { value: 'transcript', label: 'Transcript' },
    { value: 'events', label: 'Events' },
]

const MAX_RENDERED_ACTIVITY_ROWS = 150

type ActivityRow =
    | { kind: 'transcript'; sequence: number; group: RunTranscriptGroup }
    | {
        kind: 'event'
        sequence: number
        event: GroupedTimelineEntry['events'][number]
        correlationLabel: string | null
    }

interface RunActivityCardProps {
    /** Fill the parent pane; the row list scrolls instead of capping height. */
    fillHeight?: boolean
    isNarrowViewport: boolean
    isLive: boolean
    activityMode: RunActivityMode
    onActivityModeChange: (mode: RunActivityMode) => void
    selectedNodeId: string | null
    onClearNodeSelection: () => void
    transcriptSegments: RunTranscriptSegment[]
    transcriptError: string | null
    groupedTimelineEntries: GroupedTimelineEntry[]
    timelineError: string | null
    timelineEventCount: number
    filteredTimelineEventCount: number
    timelineCategoryFilter: 'all' | TimelineEventCategory
    timelineSeverityFilter: 'all' | TimelineSeverity
    onTimelineCategoryFilterChange: (value: 'all' | TimelineEventCategory) => void
    onTimelineSeverityFilterChange: (value: 'all' | TimelineSeverity) => void
    hasOlderTimelineEvents: boolean
    isTimelineLoadingOlder: boolean
    onLoadOlderTimelineEvents: () => void | Promise<void>
}

const renderSourceLabel = (event: TimelineEventEntry) => {
    if (event.sourceScope !== 'child') {
        return null
    }
    const flowLabel = event.sourceFlowName ? `Child flow ${event.sourceFlowName}` : 'Child flow'
    return event.sourceParentNodeId ? `${flowLabel} via ${event.sourceParentNodeId}` : flowLabel
}

export function EventRow({
    event,
    correlationLabel,
}: {
    event: TimelineEventEntry
    correlationLabel: string | null
}) {
    const sourceLabel = renderSourceLabel(event)
    return (
        <article
            data-testid="run-event-timeline-row"
            className="rounded-md border border-border/70 bg-muted/30 px-2.5 py-1.5"
        >
            <div className="flex items-center gap-2 text-[11px]">
                <span
                    data-testid="run-event-timeline-row-type"
                    className="inline-flex shrink-0 rounded border border-border/80 bg-background px-1.5 py-0.5 font-medium text-foreground"
                >
                    {humanizeTimelineType(event.type)}
                </span>
                {event.severity !== 'info' ? (
                    <span
                        data-testid="run-event-timeline-row-severity"
                        className={`inline-flex shrink-0 rounded border px-1.5 py-0.5 uppercase tracking-wide ${TIMELINE_SEVERITY_STYLES[event.severity]}`}
                    >
                        {TIMELINE_SEVERITY_LABELS[event.severity]}
                    </span>
                ) : null}
                <span data-testid="run-event-timeline-row-time" className="ml-auto shrink-0 text-muted-foreground">
                    {formatTimestamp(event.receivedAt)}
                </span>
            </div>
            <p data-testid="run-event-timeline-row-summary" className="mt-0.5 text-sm text-foreground">
                {event.summary}
            </p>
            {correlationLabel || event.nodeId || sourceLabel ? (
                <div className="mt-0.5 flex flex-wrap items-center gap-x-3 text-xs text-muted-foreground">
                    {correlationLabel ? (
                        <span data-testid="run-event-timeline-row-correlation">
                            {correlationLabel}
                        </span>
                    ) : null}
                    {event.nodeId ? (
                        <span data-testid="run-event-timeline-row-node">
                            Node: {event.nodeId}
                            {event.stageIndex !== null ? ` (index ${event.stageIndex})` : ''}
                        </span>
                    ) : null}
                    {sourceLabel ? (
                        <span data-testid="run-event-timeline-row-source">
                            Source: {sourceLabel}
                        </span>
                    ) : null}
                </div>
            ) : null}
        </article>
    )
}

export function RunActivityCard({
    fillHeight = false,
    isNarrowViewport,
    isLive,
    activityMode,
    onActivityModeChange,
    selectedNodeId,
    onClearNodeSelection,
    transcriptSegments,
    transcriptError,
    groupedTimelineEntries,
    timelineError,
    timelineEventCount,
    filteredTimelineEventCount,
    timelineCategoryFilter,
    timelineSeverityFilter,
    onTimelineCategoryFilterChange,
    onTimelineSeverityFilterChange,
    hasOlderTimelineEvents,
    isTimelineLoadingOlder,
    onLoadOlderTimelineEvents,
}: RunActivityCardProps) {
    const expansion = useTranscriptExpansion()
    const transcriptGroups = useMemo(() => (
        buildRunTranscriptGroups(transcriptSegments, selectedNodeId)
    ), [transcriptSegments, selectedNodeId])

    const scopedTimelineGroups = useMemo(() => {
        if (!selectedNodeId) {
            return groupedTimelineEntries
        }
        return groupedTimelineEntries
            .map((group) => ({
                ...group,
                events: group.events.filter((event) => event.nodeId === selectedNodeId),
            }))
            .filter((group) => group.events.length > 0)
    }, [groupedTimelineEntries, selectedNodeId])

    const correlationLabelFor = (group: GroupedTimelineEntry) => (
        group.correlation?.label ?? null
    )

    const activityRows = useMemo<ActivityRow[]>(() => {
        const rows: ActivityRow[] = []
        if (activityMode !== 'events') {
            for (const group of transcriptGroups) {
                rows.push({ kind: 'transcript', sequence: group.latestSequence, group })
            }
        }
        if (activityMode !== 'transcript') {
            for (const group of scopedTimelineGroups) {
                const correlationLabel = correlationLabelFor(group)
                for (const event of group.events) {
                    // The narrative view folds checkpoint/state bookkeeping
                    // away; Events mode and the category filter keep them.
                    if (activityMode === 'all' && (event.category === 'checkpoint' || event.category === 'state')) {
                        continue
                    }
                    rows.push({
                        kind: 'event',
                        sequence: event.sequence,
                        event,
                        correlationLabel,
                    })
                }
            }
        }
        // Chronological, like a log: the live edge is the bottom. A growing
        // node keeps appending there, and a newly started node lands there
        // too, so following the run means watching one place.
        rows.sort((left, right) => left.sequence - right.sequence)
        return rows
    }, [activityMode, transcriptGroups, scopedTimelineGroups])

    const renderedRows = activityRows.slice(-MAX_RENDERED_ACTIVITY_ROWS)
    const truncatedRowCount = activityRows.length - renderedRows.length

    // Stick to the live edge: while the operator is at (or near) the bottom,
    // new activity keeps the view pinned there; scrolling up releases the
    // pin until they return or jump back.
    const listRef = useRef<HTMLDivElement | null>(null)
    const isFollowingRef = useRef(true)
    const [isFollowingLiveEdge, setIsFollowingLiveEdge] = useState(true)
    const setFollowing = useCallback((following: boolean) => {
        isFollowingRef.current = following
        setIsFollowingLiveEdge((current) => (current === following ? current : following))
    }, [])
    const handleListScroll = useCallback(() => {
        const list = listRef.current
        if (!list) {
            return
        }
        setFollowing(list.scrollHeight - list.scrollTop - list.clientHeight < 48)
    }, [setFollowing])
    const jumpToLatest = useCallback(() => {
        const list = listRef.current
        if (list) {
            list.scrollTop = list.scrollHeight
        }
        setFollowing(true)
    }, [setFollowing])
    const liveEdgeSignature = renderedRows.length > 0
        ? `${renderedRows.length}:${renderedRows[renderedRows.length - 1].sequence}:${transcriptSegments.length}`
        : ''
    useEffect(() => {
        const list = listRef.current
        if (list && isFollowingRef.current) {
            list.scrollTop = list.scrollHeight
        }
    }, [liveEdgeSignature])
    const showEventFilters = activityMode !== 'transcript'
    const scopedEmpty = activityRows.length === 0

    return (
        <Card
            data-testid="run-activity-stream-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn(
                'gap-2 py-3',
                fillHeight && 'flex h-full min-h-0 flex-col',
                isNarrowViewport ? 'p-3' : undefined,
            )}
        >
            <CardHeader className="gap-1 px-4">
                <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
                    {fillHeight ? <span aria-hidden className="h-0 w-0" /> : (
                        <h3
                            className="text-sm font-semibold text-foreground"
                            title="Transcript and journal history in one chronological stream; the newest activity is at the bottom and the view follows it while a run is live. Select a graph node to focus its activity."
                        >
                            Activity
                        </h3>
                    )}
                    <div className="flex items-center gap-2">
                        <div
                            role="group"
                            aria-label="Activity mode"
                            className="inline-flex overflow-hidden rounded-md border border-border"
                        >
                            {ACTIVITY_MODE_OPTIONS.map((option) => (
                                <button
                                    key={option.value}
                                    type="button"
                                    data-testid={`run-activity-mode-${option.value}`}
                                    aria-pressed={activityMode === option.value}
                                    onClick={() => onActivityModeChange(option.value)}
                                    className={cn(
                                        'px-2 py-1 text-xs font-medium transition-colors',
                                        activityMode === option.value
                                            ? 'bg-primary text-primary-foreground'
                                            : 'bg-background text-muted-foreground hover:bg-muted/60',
                                    )}
                                >
                                    {option.label}
                                </button>
                            ))}
                        </div>
                        <span
                            className={cn(
                                'inline-flex rounded border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide',
                                isLive
                                    ? 'border-sky-500/40 bg-sky-500/10 text-sky-700'
                                    : 'border-border bg-muted text-muted-foreground',
                            )}
                        >
                            {isLive ? 'Live' : 'Idle'}
                        </span>
                    </div>
                </div>
            </CardHeader>
            <CardContent className={cn('space-y-2 px-4', fillHeight && 'flex min-h-0 flex-1 flex-col')}>
                {isPerformanceDebugEnabled() ? (
                    <>
                        <div
                            data-testid="timeline-update-performance-budget"
                            data-budget-ms={TIMELINE_UPDATE_BUDGET_MS}
                            className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                        >
                            Journal update budget: {TIMELINE_UPDATE_BUDGET_MS}ms max per live update batch.
                        </div>
                        <div
                            data-testid="run-event-timeline-throughput"
                            data-loaded-count={timelineEventCount}
                            data-rendered-count={renderedRows.length}
                            data-window-size={MAX_RENDERED_ACTIVITY_ROWS}
                            className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                        >
                            Loaded {timelineEventCount} journal entries. Rendering {renderedRows.length} rows in a bounded window.
                        </div>
                    </>
                ) : null}
                {selectedNodeId ? (
                    <div className="flex items-center gap-2">
                        <span
                            data-testid="run-activity-node-scope"
                            className="inline-flex items-center gap-1 rounded-full border border-sky-500/40 bg-sky-500/10 px-2 py-0.5 text-xs font-medium text-sky-700"
                        >
                            Node: {selectedNodeId}
                            <button
                                type="button"
                                data-testid="run-activity-node-scope-clear"
                                aria-label="Clear node focus"
                                onClick={onClearNodeSelection}
                                className="ml-1 font-semibold hover:text-sky-900"
                            >
                                ×
                            </button>
                        </span>
                    </div>
                ) : null}
                {timelineError ? (
                    <Alert
                        data-testid="run-event-timeline-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                    >
                        <AlertDescription className="text-inherit">{timelineError}</AlertDescription>
                    </Alert>
                ) : null}
                {transcriptError && activityMode !== 'events' ? (
                    <Alert
                        data-testid="run-transcript-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                    >
                        <AlertDescription className="text-inherit">{transcriptError}</AlertDescription>
                    </Alert>
                ) : null}
                {!timelineError && showEventFilters ? (
                    <div className="flex flex-wrap items-center gap-2">
                        <NativeSelect
                            aria-label="Category"
                            data-testid="run-event-timeline-filter-category"
                            value={timelineCategoryFilter}
                            onChange={(event) => onTimelineCategoryFilterChange(event.target.value as 'all' | TimelineEventCategory)}
                            className="h-7 w-auto min-w-32 text-xs"
                        >
                            <option value="all">All categories</option>
                            {Object.entries(TIMELINE_CATEGORY_LABELS).map(([category, label]) => (
                                <option key={category} value={category}>{label}</option>
                            ))}
                        </NativeSelect>
                        <NativeSelect
                            aria-label="Severity"
                            data-testid="run-event-timeline-filter-severity"
                            value={timelineSeverityFilter}
                            onChange={(event) => onTimelineSeverityFilterChange(event.target.value as 'all' | TimelineSeverity)}
                            className="h-7 w-auto min-w-28 text-xs"
                        >
                            <option value="all">All severities</option>
                            <option value="info">Info</option>
                            <option value="warning">Warning</option>
                            <option value="error">Error</option>
                        </NativeSelect>
                    </div>
                ) : null}
                {!timelineError && scopedEmpty && hasOlderTimelineEvents ? (
                    <div className="flex justify-center">
                        <Button
                            type="button"
                            data-testid="run-journal-load-older"
                            variant="outline"
                            size="sm"
                            disabled={isTimelineLoadingOlder}
                            onClick={() => {
                                void onLoadOlderTimelineEvents()
                            }}
                        >
                            {isTimelineLoadingOlder ? 'Loading…' : 'Load older'}
                        </Button>
                    </div>
                ) : null}
                {!timelineError && scopedEmpty ? (
                    <Empty data-testid="run-activity-empty" className="text-sm text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>
                                {timelineEventCount === 0
                                    ? 'No activity has been recorded for this run yet.'
                                    : selectedNodeId
                                        ? `No ${activityMode === 'transcript' ? 'transcript' : 'activity'} entries for node ${selectedNodeId} in the loaded history.`
                                        : filteredTimelineEventCount === 0 && activityMode !== 'transcript'
                                            ? 'No journal entries match the current filters.'
                                            : 'No matching activity in the loaded history.'}
                            </EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                ) : null}
                {renderedRows.length > 0 ? (
                    <div className={cn('relative', fillHeight ? 'flex min-h-0 flex-1 flex-col' : undefined)}>
                        <div
                            ref={listRef}
                            onScroll={handleListScroll}
                            data-testid="run-activity-list"
                            className={cn(
                                'space-y-1.5 overflow-auto pr-1',
                                fillHeight ? 'min-h-0 flex-1' : 'max-h-[32rem]',
                            )}
                        >
                            {hasOlderTimelineEvents ? (
                                <div className="flex justify-center">
                                    <Button
                                        type="button"
                                        data-testid="run-journal-load-older"
                                        variant="outline"
                                        size="sm"
                                        disabled={isTimelineLoadingOlder}
                                        onClick={() => {
                                            void onLoadOlderTimelineEvents()
                                        }}
                                    >
                                        {isTimelineLoadingOlder ? 'Loading…' : 'Load older'}
                                    </Button>
                                </div>
                            ) : null}
                            {truncatedRowCount > 0 ? (
                                <p data-testid="run-activity-truncation-note" className="text-center text-xs text-muted-foreground">
                                    Showing the latest {renderedRows.length} rows; {truncatedRowCount} older loaded rows are
                                    hidden. Narrow the filters or focus a node to see more.
                                </p>
                            ) : null}
                            {renderedRows.map((row) => (
                                row.kind === 'transcript'
                                    ? (
                                        <RunTranscriptGroupSection
                                            key={`transcript-${row.group.turnId}`}
                                            group={row.group}
                                            expansion={expansion}
                                        />
                                    )
                                    : (
                                        <EventRow
                                            key={`event-${row.event.id}`}
                                            event={row.event}
                                            correlationLabel={row.correlationLabel}
                                        />
                                    )
                            ))}
                        </div>
                        {!isFollowingLiveEdge ? (
                            <button
                                type="button"
                                data-testid="run-activity-jump-to-latest"
                                onClick={jumpToLatest}
                                className="absolute bottom-2 left-1/2 -translate-x-1/2 rounded-full border border-border bg-background/95 px-3 py-1 text-xs font-medium text-foreground shadow-sm hover:bg-muted"
                            >
                                Jump to latest ↓
                            </button>
                        ) : null}
                    </div>
                ) : null}
            </CardContent>
        </Card>
    )
}
