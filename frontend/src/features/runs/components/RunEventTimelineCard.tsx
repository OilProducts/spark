import type {
    GroupedTimelineEntry,
    PendingInterviewGate,
    PendingInterviewGateGroup,
    TimelineEventCategory,
    TimelineSeverity,
} from '../model/shared'
import {
    TIMELINE_CATEGORY_LABELS,
    TIMELINE_MAX_ITEMS,
    TIMELINE_SEVERITY_LABELS,
    TIMELINE_SEVERITY_STYLES,
    formatTimestamp,
} from '../model/shared'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { RunQuestionsPanel } from './RunQuestionsPanel'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunEventTimelineCardProps {
    collapsed: boolean
    isNarrowViewport: boolean
    isTimelineLive: boolean
    timelineEvents: Array<{ id: string }>
    timelineDroppedCount: number
    timelineError: string | null
    visiblePendingInterviewGates: PendingInterviewGate[]
    groupedPendingInterviewGates: PendingInterviewGateGroup[]
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
    freeformAnswersByGateId: Record<string, string>
    timelineTypeFilter: string
    timelineTypeOptions: string[]
    timelineNodeStageFilter: string
    timelineCategoryFilter: 'all' | TimelineEventCategory
    timelineSeverityFilter: 'all' | TimelineSeverity
    filteredTimelineEvents: Array<{ id: string }>
    groupedTimelineEntries: GroupedTimelineEntry[]
    onTimelineTypeFilterChange: (value: string) => void
    onTimelineNodeStageFilterChange: (value: string) => void
    onTimelineCategoryFilterChange: (value: 'all' | TimelineEventCategory) => void
    onTimelineSeverityFilterChange: (value: 'all' | TimelineSeverity) => void
    onFreeformAnswerChange: (questionId: string, value: string) => void
    onSubmitPendingGateAnswer: (gate: PendingInterviewGate, answer: string) => void | Promise<void>
    onCollapsedChange: (collapsed: boolean) => void
}

export function RunEventTimelineCard({
    collapsed,
    isNarrowViewport,
    isTimelineLive,
    timelineEvents,
    timelineDroppedCount,
    timelineError,
    visiblePendingInterviewGates,
    groupedPendingInterviewGates,
    pendingGateActionError,
    submittingGateIds,
    freeformAnswersByGateId,
    timelineTypeFilter,
    timelineTypeOptions,
    timelineNodeStageFilter,
    timelineCategoryFilter,
    timelineSeverityFilter,
    filteredTimelineEvents,
    groupedTimelineEntries,
    onTimelineTypeFilterChange,
    onTimelineNodeStageFilterChange,
    onTimelineCategoryFilterChange,
    onTimelineSeverityFilterChange,
    onFreeformAnswerChange,
    onSubmitPendingGateAnswer,
    onCollapsedChange,
}: RunEventTimelineCardProps) {
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
                        <h3 className="text-sm font-semibold text-foreground">Event Timeline</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Live typed events, filter controls, and pending human gates.
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
                    Timeline update budget: {TIMELINE_UPDATE_BUDGET_MS}ms max per stream update batch.
                </div>
                {(timelineEvents.length > 0 || timelineDroppedCount > 0) && (
                    <div
                        data-testid="run-event-timeline-throughput"
                        data-max-items={TIMELINE_MAX_ITEMS}
                        data-dropped-count={timelineDroppedCount}
                        className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                    >
                        Showing latest {Math.min(timelineEvents.length, TIMELINE_MAX_ITEMS)} events.
                        {timelineDroppedCount > 0
                            ? ` Dropped ${timelineDroppedCount} older events to stay responsive.`
                            : ''}
                    </div>
                )}
                {timelineError && (
                    <Alert
                        data-testid="run-event-timeline-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                    >
                        <AlertDescription className="text-inherit">{timelineError}</AlertDescription>
                    </Alert>
                )}
                {!timelineError && visiblePendingInterviewGates.length > 0 && (
                    <RunQuestionsPanel
                        freeformAnswersByGateId={freeformAnswersByGateId}
                        groupedPendingInterviewGates={groupedPendingInterviewGates}
                        onFreeformAnswerChange={onFreeformAnswerChange}
                        onSubmitPendingGateAnswer={(gate, answer) => {
                            void onSubmitPendingGateAnswer(gate, answer)
                        }}
                        pendingGateActionError={pendingGateActionError}
                        submittingGateIds={submittingGateIds}
                    />
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
                                placeholder="Node id or stage index..."
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
                {!timelineError && timelineEvents.length === 0 && (
                    <Empty data-testid="run-event-timeline-empty" className="text-sm text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>No typed timeline events yet.</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                )}
                {!timelineError && timelineEvents.length > 0 && filteredTimelineEvents.length === 0 && (
                    <Empty data-testid="run-event-timeline-empty" className="text-sm text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>No timeline events match the current filters.</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                )}
                {groupedTimelineEntries.length > 0 && (
                    <div data-testid="run-event-timeline-list" className="max-h-80 space-y-2 overflow-auto pr-1">
                        {groupedTimelineEntries.map((entry) => (
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
                                            {entry.events.length} event{entry.events.length === 1 ? '' : 's'}
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
                )}
                </CardContent>
            ) : null}
        </Card>
    )
}
