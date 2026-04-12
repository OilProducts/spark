import { useEffect, useMemo, useRef } from 'react'

import { useStore } from '@/store'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import type { GroupedTimelineEntry, RunRecord } from '../model/shared'
import { formatTimestamp, STATUS_LABELS } from '../model/shared'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunActivityCardProps {
    checkpointCompletedNodes: string
    checkpointCurrentNode: string
    checkpointRetryCounters: string
    collapsed: boolean
    groupedTimelineEntries: GroupedTimelineEntry[]
    pendingGateCount: number
    rawLogsCollapsed: boolean
    run: RunRecord
    onCollapsedChange: (collapsed: boolean) => void
    onRawLogsCollapsedChange: (collapsed: boolean) => void
}

interface RecentActivityRow {
    id: string
    summary: string
    label: string
    timestamp: string
}

function formatStatusLabel(status: string): string {
    return STATUS_LABELS[status] || status
}

function deriveReasonText(run: RunRecord): string | null {
    if (run.outcome_reason_message && run.outcome_reason_message.trim().length > 0) {
        return run.outcome_reason_message.trim()
    }
    if (run.outcome_reason_code && run.outcome_reason_code.trim().length > 0) {
        return run.outcome_reason_code.trim()
    }
    if (run.last_error && run.last_error.trim().length > 0) {
        return run.last_error.trim()
    }
    return null
}

function deriveRetryFact(
    groupedTimelineEntries: GroupedTimelineEntry[],
    checkpointRetryCounters: string,
): string | null {
    const formatEventLabel = (event: GroupedTimelineEntry['events'][number]): string => {
        const baseLabel = event.nodeId || 'stage'
        if (event.sourceScope !== 'child') {
            return baseLabel
        }
        const sourceLabel = event.sourceFlowName
            ? `child ${event.sourceFlowName}`
            : 'child flow'
        return event.sourceParentNodeId ? `${baseLabel} (${sourceLabel} via ${event.sourceParentNodeId})` : `${baseLabel} (${sourceLabel})`
    }
    for (const entry of groupedTimelineEntries) {
        for (const event of entry.events) {
            const attempt = typeof event.payload.attempt === 'number' && Number.isFinite(event.payload.attempt)
                ? event.payload.attempt
                : null
            if (event.type === 'StageRetrying') {
                return attempt !== null
                    ? `Retrying ${formatEventLabel(event)} (attempt ${attempt})`
                    : `Retrying ${formatEventLabel(event)}`
            }
            if (attempt !== null && (event.type === 'StageStarted' || event.type === 'StageCompleted')) {
                return `${formatEventLabel(event)} attempt ${attempt}`
            }
        }
    }
    return checkpointRetryCounters !== '—' ? checkpointRetryCounters : null
}

function deriveCompletedNodeCount(selectedRunCompletedNodes: string[], checkpointCompletedNodes: string): number {
    if (selectedRunCompletedNodes.length > 0) {
        return selectedRunCompletedNodes.length
    }
    if (checkpointCompletedNodes === '—') {
        return 0
    }
    return checkpointCompletedNodes
        .split(',')
        .map((value) => value.trim())
        .filter((value) => value.length > 0)
        .length
}

function deriveCurrentNode(run: RunRecord, checkpointCurrentNode: string): string | null {
    if (run.current_node && run.current_node.trim().length > 0) {
        return run.current_node.trim()
    }
    if (checkpointCurrentNode !== '—') {
        return checkpointCurrentNode
    }
    return null
}

function deriveHeadline(run: RunRecord, currentNode: string | null, pendingGateCount: number): string {
    const reasonText = deriveReasonText(run)
    let headline: string

    if (pendingGateCount > 0) {
        headline = `Waiting for input at ${currentNode || 'current node'}`
    } else if (run.status === 'running' && currentNode) {
        headline = `Running ${currentNode}`
    } else if (run.status === 'cancel_requested' || run.status === 'abort_requested') {
        headline = currentNode
            ? `Cancel requested while ${currentNode} winds down`
            : 'Cancel requested while the run winds down'
    } else if (run.status === 'completed' && run.outcome === 'success') {
        headline = 'Completed successfully'
    } else if (run.status === 'completed' && run.outcome === 'failure') {
        headline = 'Completed with failure outcome'
    } else if (run.status === 'failed' || run.status === 'validation_error') {
        headline = currentNode ? `Failed in ${currentNode}` : 'Failed'
    } else {
        headline = formatStatusLabel(run.status)
    }

    return reasonText ? `${headline}: ${reasonText}` : headline
}

function buildRecentActivityRows(groupedTimelineEntries: GroupedTimelineEntry[]): RecentActivityRow[] {
    return groupedTimelineEntries.slice(0, 6).map((entry) => {
        const primaryEvent = entry.events[0]
        const sourceLabel = primaryEvent.sourceScope === 'child'
            ? primaryEvent.sourceFlowName
                ? `Child · ${primaryEvent.sourceFlowName}${primaryEvent.sourceParentNodeId ? ` via ${primaryEvent.sourceParentNodeId}` : ''}`
                : `Child${primaryEvent.sourceParentNodeId ? ` via ${primaryEvent.sourceParentNodeId}` : ''}`
            : null
        const eventLabel = primaryEvent.nodeId
            ? primaryEvent.stageIndex !== null
                ? `${primaryEvent.nodeId} · stage ${primaryEvent.stageIndex}`
                : primaryEvent.nodeId
            : primaryEvent.stageIndex !== null
                ? `stage ${primaryEvent.stageIndex}`
                : primaryEvent.type
        const label = sourceLabel ? `${sourceLabel} · ${eventLabel}` : eventLabel
        return {
            id: entry.id,
            summary: primaryEvent.summary,
            label,
            timestamp: primaryEvent.receivedAt,
        }
    })
}

export function RunActivityCard({
    checkpointCompletedNodes,
    checkpointCurrentNode,
    checkpointRetryCounters,
    collapsed,
    groupedTimelineEntries,
    pendingGateCount,
    rawLogsCollapsed,
    run,
    onCollapsedChange,
    onRawLogsCollapsedChange,
}: RunActivityCardProps) {
    const logs = useStore((state) => state.logs)
    const clearLogs = useStore((state) => state.clearLogs)
    const selectedRunCompletedNodes = useStore((state) => state.selectedRunCompletedNodes)
    const logsEndRef = useRef<HTMLDivElement>(null)

    useEffect(() => {
        if (rawLogsCollapsed) {
            return
        }
        logsEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    }, [logs, rawLogsCollapsed])

    const currentNode = useMemo(
        () => deriveCurrentNode(run, checkpointCurrentNode),
        [checkpointCurrentNode, run],
    )
    const completedNodeCount = useMemo(
        () => deriveCompletedNodeCount(selectedRunCompletedNodes, checkpointCompletedNodes),
        [checkpointCompletedNodes, selectedRunCompletedNodes],
    )
    const retryFact = useMemo(
        () => deriveRetryFact(groupedTimelineEntries, checkpointRetryCounters),
        [checkpointRetryCounters, groupedTimelineEntries],
    )
    const recentActivity = useMemo(
        () => buildRecentActivityRows(groupedTimelineEntries),
        [groupedTimelineEntries],
    )
    const latestMeaningfulEventAt = recentActivity[0]?.timestamp ?? null
    const headline = useMemo(
        () => deriveHeadline(run, currentNode, pendingGateCount),
        [currentNode, pendingGateCount, run],
    )
    const statusLabel = formatStatusLabel(run.status)

    const facts = [
        {
            id: 'current-node',
            label: 'Current node',
            testId: 'run-activity-fact-current-node',
            value: currentNode || '—',
        },
        {
            id: 'completed-count',
            label: 'Completed nodes',
            testId: 'run-activity-fact-completed-count',
            value: String(completedNodeCount),
        },
        ...(retryFact
            ? [{
                id: 'retry-state',
                label: 'Retry state',
                testId: 'run-activity-fact-retry-state',
                value: retryFact,
            }]
            : []),
        ...(pendingGateCount > 0
            ? [{
                id: 'pending-gates',
                label: 'Pending gates',
                testId: 'run-activity-fact-pending-gates',
                value: String(pendingGateCount),
            }]
            : []),
        ...(latestMeaningfulEventAt
            ? [{
                id: 'latest-event',
                label: 'Latest event',
                testId: 'run-activity-fact-latest-event',
                value: formatTimestamp(latestMeaningfulEventAt),
            }]
            : []),
    ]

    return (
        <Card data-testid="run-activity-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Run Activity</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            What the selected run is doing now, why, and the latest meaningful runtime evidence.
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <span
                            data-testid="run-activity-status"
                            className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground"
                        >
                            {statusLabel}
                        </span>
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-activity-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-4 px-4">
                    <section
                        data-testid="run-activity-headline"
                        className="rounded-md border border-border/80 bg-muted/20 px-4 py-3"
                    >
                        <p className="text-sm font-medium text-foreground">{headline}</p>
                    </section>

                    <section
                        data-testid="run-activity-facts"
                        className="grid gap-3 md:grid-cols-2 xl:grid-cols-4"
                    >
                        {facts.map((fact) => (
                            <div
                                key={fact.id}
                                data-testid={fact.testId}
                                className="rounded-md border border-border/80 bg-background/80 px-3 py-2"
                            >
                                <div className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                                    {fact.label}
                                </div>
                                <div className="mt-1 text-sm text-foreground">{fact.value}</div>
                            </div>
                        ))}
                    </section>

                    <section
                        data-testid="run-activity-recent-feed"
                        className="rounded-md border border-border/80 bg-background/70 p-3"
                    >
                        <div className="flex items-center justify-between gap-3">
                            <div>
                                <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                    Recent Activity
                                </h3>
                                <p className="mt-1 text-xs text-muted-foreground">
                                    Latest grouped events from the selected run timeline.
                                </p>
                            </div>
                        </div>
                        {recentActivity.length === 0 ? (
                            <div className="mt-3">
                                <Empty className="text-sm text-muted-foreground">
                                    <EmptyHeader>
                                        <EmptyDescription>
                                            No recent activity events have arrived for this run yet.
                                        </EmptyDescription>
                                    </EmptyHeader>
                                </Empty>
                            </div>
                        ) : (
                            <div className="mt-3 space-y-2">
                                {recentActivity.map((entry) => (
                                    <article
                                        key={entry.id}
                                        data-testid="run-activity-entry"
                                        className="rounded-md border border-border/70 bg-muted/20 px-3 py-2"
                                    >
                                        <div className="flex flex-wrap items-center justify-between gap-2">
                                            <span
                                                data-testid="run-activity-entry-label"
                                                className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
                                            >
                                                {entry.label}
                                            </span>
                                            <span
                                                data-testid="run-activity-entry-time"
                                                className="text-[11px] text-muted-foreground"
                                            >
                                                {formatTimestamp(entry.timestamp)}
                                            </span>
                                        </div>
                                        <p
                                            data-testid="run-activity-entry-summary"
                                            className="mt-1 text-sm text-foreground"
                                        >
                                            {entry.summary}
                                        </p>
                                    </article>
                                ))}
                            </div>
                        )}
                    </section>

                    <section
                        data-testid="run-activity-logs-section"
                        className="rounded-md border border-border/80 bg-background/70"
                    >
                        <div className="flex items-center justify-between gap-2 border-b border-border/70 px-3 py-2">
                            <div>
                                <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                    Raw Logs
                                </h3>
                                <p className="mt-1 text-xs text-muted-foreground">
                                    Runtime log lines remain available here, but they are secondary to the activity summary.
                                </p>
                            </div>
                            <div className="flex items-center gap-2">
                                <span
                                    data-testid="run-activity-log-count"
                                    className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground"
                                >
                                    {logs.length} log{logs.length === 1 ? '' : 's'}
                                </span>
                                <Button
                                    data-testid="run-activity-clear-logs-button"
                                    onClick={clearLogs}
                                    variant="outline"
                                    size="xs"
                                >
                                    Clear
                                </Button>
                                <RunSectionToggleButton
                                    collapsed={rawLogsCollapsed}
                                    onToggle={() => onRawLogsCollapsedChange(!rawLogsCollapsed)}
                                    testId="run-activity-logs-toggle-button"
                                />
                            </div>
                        </div>
                        {!rawLogsCollapsed ? (
                            <div
                                data-testid="run-activity-logs-panel"
                                className="max-h-96 overflow-y-auto p-4 font-mono text-sm"
                            >
                                {logs.length === 0 ? (
                                    <Empty className="text-sm text-muted-foreground">
                                        <EmptyHeader>
                                            <EmptyDescription>
                                                No runtime logs have arrived for this run yet.
                                            </EmptyDescription>
                                        </EmptyHeader>
                                    </Empty>
                                ) : (
                                    <div className="space-y-1">
                                        {logs.map((log, index) => (
                                            <div
                                                key={`${log.time}-${index}`}
                                                data-testid="run-activity-log-row"
                                                className="flex gap-4 rounded px-2 py-0.5 hover:bg-muted/50"
                                            >
                                                <span className="w-20 shrink-0 select-none text-muted-foreground">
                                                    {log.time}
                                                </span>
                                                <span className={
                                                    log.type === 'success'
                                                        ? 'break-all text-green-500'
                                                        : log.type === 'error'
                                                            ? 'break-all text-destructive'
                                                            : 'break-all text-foreground'
                                                }>
                                                    {log.msg}
                                                </span>
                                            </div>
                                        ))}
                                        <div ref={logsEndRef} />
                                    </div>
                                )}
                            </div>
                        ) : null}
                    </section>
                </CardContent>
            ) : null}
        </Card>
    )
}
