import { useEffect, useMemo, useRef, useState } from 'react'

import type {
    GroupedTimelineEntry,
    TimelineEventCategory,
    TimelineSeverity,
} from '../model/shared'
import {
    RUN_JOURNAL_WINDOW_SIZE,
    TIMELINE_CATEGORY_LABELS,
    TIMELINE_SEVERITY_LABELS,
    TIMELINE_SEVERITY_STYLES,
    formatTimestamp,
} from '../model/shared'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { cn } from '@/lib/utils'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunEventTimelineCardProps {
    collapsed: boolean
    isNarrowViewport: boolean
    isTimelineLive: boolean
    timelineEventCount: number
    timelineError: string | null
    timelineTypeFilter: string
    timelineTypeOptions: string[]
    timelineNodeStageFilter: string
    timelineCategoryFilter: 'all' | TimelineEventCategory
    timelineSeverityFilter: 'all' | TimelineSeverity
    filteredTimelineEventCount: number
    groupedTimelineEntries: GroupedTimelineEntry[]
    hasOlderTimelineEvents: boolean
    isTimelineLoadingOlder: boolean
    onLoadOlderTimelineEvents: () => void | Promise<void>
    onTimelineTypeFilterChange: (value: string) => void
    onTimelineNodeStageFilterChange: (value: string) => void
    onTimelineCategoryFilterChange: (value: 'all' | TimelineEventCategory) => void
    onTimelineSeverityFilterChange: (value: 'all' | TimelineSeverity) => void
    onCollapsedChange: (collapsed: boolean) => void
}

const JOURNAL_ROW_ESTIMATE_PX = 132
const JOURNAL_OVERSCAN_ROWS = 6

type WindowedTimelineGroup = {
    id: string
    correlation: GroupedTimelineEntry['correlation']
    totalEventCount: number
    events: GroupedTimelineEntry['events']
}

export function RunEventTimelineCard({
    collapsed,
    isNarrowViewport,
    isTimelineLive,
    timelineEventCount,
    timelineError,
    timelineTypeFilter,
    timelineTypeOptions,
    timelineNodeStageFilter,
    timelineCategoryFilter,
    timelineSeverityFilter,
    filteredTimelineEventCount,
    groupedTimelineEntries,
    hasOlderTimelineEvents,
    isTimelineLoadingOlder,
    onLoadOlderTimelineEvents,
    onTimelineTypeFilterChange,
    onTimelineNodeStageFilterChange,
    onTimelineCategoryFilterChange,
    onTimelineSeverityFilterChange,
    onCollapsedChange,
}: RunEventTimelineCardProps) {
    const listRef = useRef<HTMLDivElement | null>(null)
    const [scrollTop, setScrollTop] = useState(0)
    const [viewportHeight, setViewportHeight] = useState(448)

    useEffect(() => {
        if (!collapsed && listRef.current) {
            setViewportHeight(listRef.current.clientHeight || 448)
        }
    }, [collapsed, groupedTimelineEntries.length])

    const windowState = useMemo(() => {
        const totalRows = groupedTimelineEntries.reduce(
            (count, entry) => count + entry.events.length,
            0,
        )
        const visibleRowCount = Math.min(
            RUN_JOURNAL_WINDOW_SIZE,
            Math.max(
                JOURNAL_OVERSCAN_ROWS * 2,
                Math.ceil(viewportHeight / JOURNAL_ROW_ESTIMATE_PX) + (JOURNAL_OVERSCAN_ROWS * 2),
            ),
        )
        const unclampedStartRowIndex = Math.max(
            0,
            Math.floor(scrollTop / JOURNAL_ROW_ESTIMATE_PX) - JOURNAL_OVERSCAN_ROWS,
        )
        const startRowIndex = Math.min(
            unclampedStartRowIndex,
            Math.max(0, totalRows - visibleRowCount),
        )
        const endRowIndex = Math.min(totalRows, startRowIndex + visibleRowCount)
        const renderedGroups: WindowedTimelineGroup[] = []
        let rowCursor = 0

        for (const entry of groupedTimelineEntries) {
            const groupStartRowIndex = rowCursor
            const groupEndRowIndex = groupStartRowIndex + entry.events.length
            rowCursor = groupEndRowIndex

            if (groupEndRowIndex <= startRowIndex) {
                continue
            }
            if (groupStartRowIndex >= endRowIndex) {
                break
            }

            const eventStartIndex = Math.max(0, startRowIndex - groupStartRowIndex)
            const eventEndIndex = Math.min(entry.events.length, endRowIndex - groupStartRowIndex)
            renderedGroups.push({
                id: entry.id,
                correlation: entry.correlation,
                totalEventCount: entry.events.length,
                events: entry.events.slice(eventStartIndex, eventEndIndex),
            })
        }

        const renderedRowCount = renderedGroups.reduce((count, entry) => count + entry.events.length, 0)
        return {
            paddingTop: startRowIndex * JOURNAL_ROW_ESTIMATE_PX,
            paddingBottom: Math.max(0, (totalRows - endRowIndex) * JOURNAL_ROW_ESTIMATE_PX),
            renderedGroups,
            renderedRowCount,
        }
    }, [groupedTimelineEntries, scrollTop, viewportHeight])

    const renderSourceLabel = (event: GroupedTimelineEntry['events'][number]) => {
        if (event.sourceScope !== 'child') {
            return null
        }
        const flowLabel = event.sourceFlowName ? `Child flow ${event.sourceFlowName}` : 'Child flow'
        return event.sourceParentNodeId ? `${flowLabel} via ${event.sourceParentNodeId}` : flowLabel
    }

    return (
        <Card
            data-testid="run-event-timeline-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={cn('gap-4 py-4', isNarrowViewport ? 'p-3' : undefined)}
        >
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Run Journal</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Durable run history with live tail updates and explicit paging for older evidence.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <span
                            className={`inline-flex rounded border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                                isTimelineLive
                                    ? 'border-sky-500/40 bg-sky-500/10 text-sky-700'
                                    : 'border-border bg-muted text-muted-foreground'
                            }`}
                        >
                            {isTimelineLive ? 'Live' : 'Idle'}
                        </span>
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-event-timeline-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
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
                        data-rendered-count={windowState.renderedRowCount}
                        data-window-size={RUN_JOURNAL_WINDOW_SIZE}
                        className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                    >
                        Loaded {timelineEventCount} journal entries. Rendering {windowState.renderedRowCount} rows in a bounded window.
                    </div>
                    {timelineError && (
                        <Alert
                            data-testid="run-event-timeline-error"
                            className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                        >
                            <AlertDescription className="text-inherit">{timelineError}</AlertDescription>
                        </Alert>
                    )}
                    {!timelineError && (
                        <div className={`grid gap-2 ${isNarrowViewport ? 'grid-cols-1' : 'md:grid-cols-2'}`}>
                            <Field className="space-y-1.5">
                                <FieldLabel>Event Type</FieldLabel>
                                <NativeSelect
                                    data-testid="run-event-timeline-filter-type"
                                    value={timelineTypeFilter}
                                    onChange={(event) => onTimelineTypeFilterChange(event.target.value)}
                                    className="h-8 text-xs"
                                >
                                    <option value="all">All event types</option>
                                    {timelineTypeOptions.map((type) => (
                                        <option key={type} value={type}>{type}</option>
                                    ))}
                                </NativeSelect>
                            </Field>
                            <Field className="space-y-1.5">
                                <FieldLabel>Node/Stage</FieldLabel>
                                <Input
                                    data-testid="run-event-timeline-filter-node-stage"
                                    value={timelineNodeStageFilter}
                                    onChange={(event) => onTimelineNodeStageFilterChange(event.target.value)}
                                    placeholder="Node id, stage index, or child flow..."
                                    className="h-8 text-xs"
                                />
                            </Field>
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
                    )}
                    {!timelineError && timelineEventCount === 0 && (
                        <Empty data-testid="run-event-timeline-empty" className="text-sm text-muted-foreground">
                            <EmptyHeader>
                                <EmptyDescription>No journal history available for this run yet.</EmptyDescription>
                            </EmptyHeader>
                        </Empty>
                    )}
                    {!timelineError && timelineEventCount > 0 && filteredTimelineEventCount === 0 && (
                        <Empty data-testid="run-event-timeline-empty" className="text-sm text-muted-foreground">
                            <EmptyHeader>
                                <EmptyDescription>No journal entries match the current filters.</EmptyDescription>
                            </EmptyHeader>
                        </Empty>
                    )}
                    {groupedTimelineEntries.length > 0 && (
                        <div
                            ref={listRef}
                            data-testid="run-event-timeline-list"
                            onScroll={(event) => {
                                const target = event.currentTarget
                                setScrollTop(target.scrollTop)
                                setViewportHeight(target.clientHeight || 448)
                            }}
                            className="max-h-[28rem] overflow-auto pr-1"
                        >
                            <div style={{ paddingTop: `${windowState.paddingTop}px`, paddingBottom: `${windowState.paddingBottom}px` }}>
                                <div className="space-y-2">
                                    {windowState.renderedGroups.map((entry) => (
                                        <section
                                            key={entry.id}
                                            data-testid="run-event-timeline-group"
                                            className="space-y-2 rounded-md border border-border/60 bg-background/50 p-2"
                                        >
                                            {entry.correlation && (
                                                <div className="flex flex-wrap items-center justify-between gap-2">
                                                    <span
                                                        data-testid="run-event-timeline-group-label"
                                                        className="inline-flex rounded border border-border/80 bg-background px-2 py-0.5 text-[11px] uppercase tracking-wide text-muted-foreground"
                                                    >
                                                        {entry.correlation.label}
                                                    </span>
                                                    <span className="text-[11px] text-muted-foreground">
                                                        {entry.totalEventCount} {entry.totalEventCount === 1 ? 'entry' : 'entries'}
                                                    </span>
                                                </div>
                                            )}
                                            {entry.events.map((event) => {
                                                const sourceLabel = renderSourceLabel(event)
                                                return (
                                                    <article
                                                        key={event.id}
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
                                                        {entry.correlation && (
                                                            <p data-testid="run-event-timeline-row-correlation" className="mt-1 text-xs text-muted-foreground">
                                                                {entry.correlation.kind === 'retry' ? 'Retry correlation' : 'Interview correlation'}: {entry.correlation.label}
                                                            </p>
                                                        )}
                                                        <p data-testid="run-event-timeline-row-summary" className="mt-1 text-sm text-foreground">
                                                            {event.summary}
                                                        </p>
                                                        {event.nodeId && (
                                                            <p data-testid="run-event-timeline-row-node" className="text-xs text-muted-foreground">
                                                                Node: {event.nodeId}
                                                                {event.stageIndex !== null ? ` (index ${event.stageIndex})` : ''}
                                                            </p>
                                                        )}
                                                        {sourceLabel && (
                                                            <p data-testid="run-event-timeline-row-source" className="text-xs text-muted-foreground">
                                                                Source: {sourceLabel}
                                                            </p>
                                                        )}
                                                    </article>
                                                )
                                            })}
                                        </section>
                                    ))}
                                </div>
                            </div>
                        </div>
                    )}
                    {hasOlderTimelineEvents && (
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
                    )}
                </CardContent>
            ) : null}
        </Card>
    )
}
