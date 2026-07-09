import type { ComponentProps, ReactNode, Ref } from 'react'

import { cn } from '@/lib/utils'
import { RunArtifactsCard } from './RunArtifactsCard'
import { RunCheckpointCard } from './RunCheckpointCard'
import { RunDetailsCard } from './RunDetailsCard'
import { RunContextCard } from './RunContextCard'
import { RunResultCard } from './RunResultCard'

export type RunInspectorTab = 'activity' | 'result' | 'details' | 'checkpoint' | 'context' | 'artifacts'

const INSPECTOR_TABS: Array<{ value: RunInspectorTab; label: string }> = [
    { value: 'activity', label: 'Activity' },
    { value: 'result', label: 'Result' },
    { value: 'details', label: 'Details' },
    { value: 'checkpoint', label: 'Checkpoint' },
    { value: 'context', label: 'Context' },
    { value: 'artifacts', label: 'Artifacts' },
]

interface RunInspectorPanelProps {
    inspectorTab: RunInspectorTab
    onInspectorTabChange: (tab: RunInspectorTab) => void
    /** The live activity/transcript stream, rendered as the primary tab. */
    activityContent: ReactNode
    fillHeight?: boolean
    scrollRegionRef?: Ref<HTMLDivElement>
    // Run scope card props, passed through unchanged
    resultCardProps: ComponentProps<typeof RunResultCard>
    detailsCardProps: ComponentProps<typeof RunDetailsCard>
    checkpointCardProps: ComponentProps<typeof RunCheckpointCard>
    contextCardProps: ComponentProps<typeof RunContextCard>
    artifactsCardProps: ComponentProps<typeof RunArtifactsCard>
}

export function RunInspectorPanel({
    inspectorTab,
    onInspectorTabChange,
    activityContent,
    fillHeight = false,
    scrollRegionRef,
    resultCardProps,
    detailsCardProps,
    checkpointCardProps,
    contextCardProps,
    artifactsCardProps,
}: RunInspectorPanelProps) {
    let content: ReactNode
    switch (inspectorTab) {
        case 'activity':
            content = activityContent
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
        <div
            data-testid="run-inspector-panel"
            className={cn('flex flex-col gap-2', fillHeight && 'h-full min-h-0 flex-1')}
        >
            <div
                role="tablist"
                aria-label="Run inspector"
                className="flex shrink-0 flex-wrap gap-1"
            >
                {INSPECTOR_TABS.map((tab) => (
                    <button
                        key={tab.value}
                        type="button"
                        role="tab"
                        aria-selected={inspectorTab === tab.value}
                        data-testid={`run-inspector-tab-${tab.value}`}
                        onClick={() => onInspectorTabChange(tab.value)}
                        className={cn(
                            'rounded-md px-2.5 py-1 text-xs font-medium transition-colors',
                            inspectorTab === tab.value
                                ? 'bg-primary text-primary-foreground'
                                : 'bg-muted/40 text-muted-foreground hover:bg-muted',
                        )}
                    >
                        {tab.label}
                    </button>
                ))}
            </div>
            <div
                ref={scrollRegionRef}
                data-testid="run-details-scroll-region"
                className={cn(fillHeight && 'min-h-0 flex-1 overflow-auto')}
            >
                {content}
            </div>
        </div>
    )
}
