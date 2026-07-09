import { useMemo, type ComponentProps, type ReactNode } from 'react'

import { Card, CardContent, CardHeader } from '@/components/ui/card'
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from '@/components/ui/empty'
import { cn } from '@/lib/utils'
import type { RunTranscriptSegment } from '@/lib/api/attractorApi'
import type {
    GroupedTimelineEntry,
    PendingInterviewGate,
} from '../model/shared'
import { buildRunTranscriptGroups } from '../model/transcriptModel'
import { EventRow } from './RunActivityCard'
import { RunTranscriptGroupSection, useTranscriptExpansion } from './RunTranscriptGroups'
import { RunArtifactsCard } from './RunArtifactsCard'
import { RunCheckpointCard } from './RunCheckpointCard'
import { RunDetailsCard } from './RunDetailsCard'
import { RunContextCard } from './RunContextCard'
import { RunResultCard } from './RunResultCard'

export type RunInspectorTab = 'result' | 'details' | 'checkpoint' | 'context' | 'artifacts' | 'node'

const RUN_SCOPE_TABS: Array<{ value: RunInspectorTab; label: string }> = [
    { value: 'result', label: 'Result' },
    { value: 'details', label: 'Details' },
    { value: 'checkpoint', label: 'Checkpoint' },
    { value: 'context', label: 'Context' },
    { value: 'artifacts', label: 'Artifacts' },
]

interface RunInspectorPanelProps {
    inspectorTab: RunInspectorTab
    onInspectorTabChange: (tab: RunInspectorTab) => void
    selectedNodeId: string | null
    // Node scope inputs
    transcriptSegments: RunTranscriptSegment[]
    groupedTimelineEntries: GroupedTimelineEntry[]
    nodeRetryCount: number | null
    pendingGatesForNode: PendingInterviewGate[]
    // Run scope card props, passed through unchanged
    resultCardProps: ComponentProps<typeof RunResultCard>
    detailsCardProps: ComponentProps<typeof RunDetailsCard>
    checkpointCardProps: ComponentProps<typeof RunCheckpointCard>
    contextCardProps: ComponentProps<typeof RunContextCard>
    artifactsCardProps: ComponentProps<typeof RunArtifactsCard>
}

function NodeScopeContent({
    selectedNodeId,
    transcriptSegments,
    groupedTimelineEntries,
    nodeRetryCount,
    pendingGatesForNode,
}: Pick<
    RunInspectorPanelProps,
    | 'selectedNodeId'
    | 'transcriptSegments'
    | 'groupedTimelineEntries'
    | 'nodeRetryCount'
    | 'pendingGatesForNode'
>) {
    const expansion = useTranscriptExpansion()
    const nodeTranscriptGroups = useMemo(() => (
        buildRunTranscriptGroups(transcriptSegments, selectedNodeId)
    ), [transcriptSegments, selectedNodeId])
    const nodeEvents = useMemo(() => (
        groupedTimelineEntries.flatMap((group) => (
            group.events
                .filter((event) => event.nodeId === selectedNodeId)
                .map((event) => ({ event, correlation: group.correlation }))
        ))
    ), [groupedTimelineEntries, selectedNodeId])

    return (
        <div data-testid="run-inspector-node" className="space-y-4">
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span data-testid="run-inspector-node-id" className="font-semibold text-foreground">
                    {selectedNodeId}
                </span>
                {nodeRetryCount !== null && nodeRetryCount > 0 ? (
                    <span
                        data-testid="run-inspector-node-retries"
                        className="inline-flex rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 uppercase tracking-wide text-amber-800"
                    >
                        {nodeRetryCount} {nodeRetryCount === 1 ? 'retry' : 'retries'}
                    </span>
                ) : null}
                {pendingGatesForNode.length > 0 ? (
                    <span
                        data-testid="run-inspector-node-pending-gates"
                        className="inline-flex rounded border border-sky-500/40 bg-sky-500/10 px-1.5 py-0.5 uppercase tracking-wide text-sky-700"
                    >
                        Waiting on input · answer in the pinned questions panel
                    </span>
                ) : null}
            </div>
            <section className="space-y-2">
                <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Transcript
                </h4>
                {nodeTranscriptGroups.length === 0 ? (
                    <Empty data-testid="run-inspector-node-transcript-empty" className="text-sm text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>No agent activity recorded at this node yet.</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                ) : (
                    <div data-testid="run-inspector-node-transcript" className="max-h-[24rem] space-y-3 overflow-auto pr-1">
                        {nodeTranscriptGroups.map((group) => (
                            <RunTranscriptGroupSection
                                key={group.turnId}
                                group={group}
                                expansion={expansion}
                            />
                        ))}
                    </div>
                )}
            </section>
            <section className="space-y-2">
                <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                    Events
                </h4>
                {nodeEvents.length === 0 ? (
                    <Empty data-testid="run-inspector-node-events-empty" className="text-sm text-muted-foreground">
                        <EmptyHeader>
                            <EmptyDescription>No journal events for this node in the loaded history.</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                ) : (
                    <div data-testid="run-inspector-node-events" className="max-h-[24rem] space-y-2 overflow-auto pr-1">
                        {nodeEvents.map(({ event, correlation }) => (
                            <EventRow
                                key={event.id}
                                event={event}
                                correlationLabel={correlation ? correlation.label : null}
                            />
                        ))}
                    </div>
                )}
            </section>
        </div>
    )
}

export function RunInspectorPanel({
    inspectorTab,
    onInspectorTabChange,
    selectedNodeId,
    transcriptSegments,
    groupedTimelineEntries,
    nodeRetryCount,
    pendingGatesForNode,
    resultCardProps,
    detailsCardProps,
    checkpointCardProps,
    contextCardProps,
    artifactsCardProps,
}: RunInspectorPanelProps) {
    const tabs: Array<{ value: RunInspectorTab; label: string }> = selectedNodeId
        ? [{ value: 'node', label: `Node: ${selectedNodeId}` }, ...RUN_SCOPE_TABS]
        : RUN_SCOPE_TABS
    const activeTab: RunInspectorTab = !selectedNodeId && inspectorTab === 'node'
        ? 'result'
        : inspectorTab

    let content: ReactNode
    switch (activeTab) {
        case 'node':
            content = (
                <NodeScopeContent
                    selectedNodeId={selectedNodeId}
                    transcriptSegments={transcriptSegments}
                    groupedTimelineEntries={groupedTimelineEntries}
                    nodeRetryCount={nodeRetryCount}
                    pendingGatesForNode={pendingGatesForNode}
                />
            )
            break
        case 'details':
            content = <RunDetailsCard {...detailsCardProps} />
            break
        case 'checkpoint':
            content = <RunCheckpointCard {...checkpointCardProps} />
            break
        case 'context':
            content = <RunContextCard {...contextCardProps} />
            break
        case 'artifacts':
            content = <RunArtifactsCard {...artifactsCardProps} />
            break
        default:
            content = <RunResultCard {...resultCardProps} />
    }

    return (
        <Card data-testid="run-inspector-panel" className="gap-3 py-4">
            <CardHeader className="gap-2 px-4">
                <div
                    role="tablist"
                    aria-label="Run inspector"
                    className="flex flex-wrap gap-1"
                >
                    {tabs.map((tab) => (
                        <button
                            key={tab.value}
                            type="button"
                            role="tab"
                            aria-selected={activeTab === tab.value}
                            data-testid={`run-inspector-tab-${tab.value}`}
                            onClick={() => onInspectorTabChange(tab.value)}
                            className={cn(
                                'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                                activeTab === tab.value
                                    ? 'bg-primary text-primary-foreground'
                                    : 'bg-muted/40 text-muted-foreground hover:bg-muted',
                            )}
                        >
                            {tab.label}
                        </button>
                    ))}
                </div>
            </CardHeader>
            <CardContent className="px-4">
                {content}
            </CardContent>
        </Card>
    )
}
