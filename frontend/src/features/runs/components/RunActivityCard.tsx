import { useMemo } from 'react'

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
import { Field, FieldLabel } from '@/components/ui/field'
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
            className="rounded-md border border-border/70 bg-muted/30 px-3 py-2"
        >
            <div className="flex flex-wrap items-center gap-2 text-[11px]">
                <span
                    data-testid="run-event-timeline-row-type"
                    className="inline-flex rounded border border-border/80 bg-background px-1.5 py-0.5 font-semibold uppercase tracking-wide text-foreground"
                >
                    {event.type}
                </span>
                <span
                    data-testid="run-event-timeline-row-category"
                    className="inline-flex rounded border border-border/80 bg-background px-1.5 py-0.5 uppercase tracking-wide text-muted-foreground"
                >
                    {TIMELINE_CATEGORY_LABELS[event.category]}
                </span>
                <span
                    data-testid="run-event-timeline-row-severity"
                    className={`inline-flex rounded border px-1.5 py-0.5 uppercase tracking-wide ${TIMELINE_SEVERITY_STYLES[event.severity]}`}
                >
                    {TIMELINE_SEVERITY_LABELS[event.severity]}
                </span>
                <span data-testid="run-event-timeline-row-time" className="text-muted-foreground">
                    {formatTimestamp(event.receivedAt)}
                </span>
            </div>
            {correlationLabel ? (
                <p data-testid="run-event-timeline-row-correlation" className="mt-1 text-xs text-muted-foreground">
                    {correlationLabel}
                </p>
            ) : null}
            <p data-testid="run-event-timeline-row-summary" className="mt-1 text-sm text-foreground">
                {event.summary}
            </p>
            {event.nodeId ? (
                <p data-testid="run-event-timeline-row-node" className="text-xs text-muted-foreground">
                    Node: {event.nodeId}
                    {event.stageIndex !== null ? ` (index ${event.stageIndex})` : ''}
                </p>
            ) : null}
            {sourceLabel ? (
                <p data-testid="run-event-timeline-row-source" className="text-xs text-muted-foreground">
                    Source: {sourceLabel}
                </p>
            ) : null}
        </article>
    )
}

export function RunActivityCard({
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
        group.correlation
            ? `${group.correlation.kind === 'retry' ? 'Retry correlation' : 'Interview correlation'}: ${group.correlation.label}`
            : null
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
                    rows.push({
                        kind: 'event',
                        sequence: event.sequence,
                        event,
                        correlationLabel,
                    })
                }
            }
        }
        rows.sort((left, right) => right.sequence - left.sequence)
        return rows
    }, [activityMode, transcriptGroups, scopedTimelineGroups])

    const renderedRows = activityRows.slice(0, MAX_RENDERED_ACTIVITY_ROWS)
    const truncatedRowCount = activityRows.length - renderedRows.length
    const showEventFilters = activityMode !== 'transcript'
    const scopedEmpty = activityRows.length === 0

    return (
        <Card
            data-testid="run-activity-stream-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn('gap-4 py-4', isNarrowViewport ? 'p-3' : undefined)}
        >
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Activity</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Transcript and journal history in one stream, newest first. Select a
                            graph node to focus its activity.
                        </p>
                    </div>
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
            <CardContent className="space-y-3 px-4">
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
                    <div className={`grid gap-2 ${isNarrowViewport ? 'grid-cols-1' : 'md:grid-cols-2'}`}>
                        <Field className="space-y-1.5">
                            <FieldLabel>Category</FieldLabel>
                            <NativeSelect
                                data-testid="run-event-timeline-filter-category"
                                value={timelineCategoryFilter}
                                onChange={(event) => onTimelineCategoryFilterChange(event.target.value as 'all' | TimelineEventCategory)}
                                className="h-8 text-xs"
                            >
                                <option value="all">All categories</option>
                                {Object.entries(TIMELINE_CATEGORY_LABELS).map(([category, label]) => (
                                    <option key={category} value={category}>{label}</option>
                                ))}
                            </NativeSelect>
                        </Field>
                        <Field className="space-y-1.5">
                            <FieldLabel>Severity</FieldLabel>
                            <NativeSelect
                                data-testid="run-event-timeline-filter-severity"
                                value={timelineSeverityFilter}
                                onChange={(event) => onTimelineSeverityFilterChange(event.target.value as 'all' | TimelineSeverity)}
                                className="h-8 text-xs"
                            >
                                <option value="all">All severities</option>
                                <option value="info">Info</option>
                                <option value="warning">Warning</option>
                                <option value="error">Error</option>
                            </NativeSelect>
                        </Field>
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
                    <div
                        data-testid="run-activity-list"
                        className="max-h-[32rem] space-y-2 overflow-auto pr-1"
                    >
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
                ) : null}
                {truncatedRowCount > 0 ? (
                    <p data-testid="run-activity-truncation-note" className="text-center text-xs text-muted-foreground">
                        Showing the latest {renderedRows.length} rows; {truncatedRowCount} older loaded rows are
                        hidden. Narrow the filters or focus a node to see more.
                    </p>
                ) : null}
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
            </CardContent>
        </Card>
    )
}
