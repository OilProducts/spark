import { useState } from 'react'

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
import {
    EmptyState,
    FieldRow,
    InlineNotice,
    Input,
    NativeSelect,
    Panel,
    PanelContent,
    PanelHeader,
    SectionHeader,
} from '@/ui'
import { RunQuestionsPanel } from './RunQuestionsPanel'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunEventTimelineCardProps {
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
}

export function RunEventTimelineCard({
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
}: RunEventTimelineCardProps) {
    const [collapsed, setCollapsed] = useState(false)

    return (
        <Panel
            data-testid="run-event-timeline-panel"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={isNarrowViewport ? 'p-3' : undefined}
        >
            <PanelHeader>
                <SectionHeader
                    title="Event Timeline"
                    description="Live typed events, filter controls, and pending human gates."
                    action={(
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
                                onToggle={() => setCollapsed((current) => !current)}
                                testId="run-event-timeline-toggle-button"
                            />
                        </div>
                    )}
                />
            </PanelHeader>
            {!collapsed ? (
                <PanelContent className="space-y-3">
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
                    <InlineNotice data-testid="run-event-timeline-error" tone="error">
                        {timelineError}
                    </InlineNotice>
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
                        <FieldRow label="Event Type" className="space-y-1.5">
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
                        </FieldRow>
                        <FieldRow label="Node/Stage" className="space-y-1.5">
                            <Input
                                data-testid="run-event-timeline-filter-node-stage"
                                value={timelineNodeStageFilter}
                                onChange={(event) => onTimelineNodeStageFilterChange(event.target.value)}
                                placeholder="Node id or stage index..."
                                className="h-8 text-xs"
                            />
                        </FieldRow>
                        <FieldRow label="Category" className="space-y-1.5">
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
                        </FieldRow>
                        <FieldRow label="Severity" className="space-y-1.5">
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
                        </FieldRow>
                    </div>
                )}
                {!timelineError && timelineEvents.length === 0 && (
                    <EmptyState data-testid="run-event-timeline-empty" description="No typed timeline events yet." />
                )}
                {!timelineError && timelineEvents.length > 0 && filteredTimelineEvents.length === 0 && (
                    <EmptyState
                        data-testid="run-event-timeline-empty"
                        description="No timeline events match the current filters."
                    />
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
                                {entry.events.map((event) => (
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
                                    </article>
                                ))}
                            </section>
                        ))}
                    </div>
                )}
                </PanelContent>
            ) : null}
        </Panel>
    )
}
