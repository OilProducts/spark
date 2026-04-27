import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Empty, EmptyDescription, EmptyHeader } from '@/components/ui/empty'
import { cn } from '@/lib/utils'
import type { RunProgressEntry, RunProgressNodeFilter, RunProgressProjection } from '../model/shared'
import { RunSectionToggleButton } from './RunSectionToggleButton'

interface RunProgressCardProps {
    collapsed: boolean
    currentNodeId: string | null | undefined
    isLive: boolean
    progressNodeFilter: RunProgressNodeFilter
    progressProjection: RunProgressProjection
    onCollapsedChange: (collapsed: boolean) => void
    onProgressNodeFilterChange: (filter: RunProgressNodeFilter) => void
}

const CHANNEL_LABELS: Record<RunProgressEntry['channel'], string> = {
    assistant: 'Assistant',
    reasoning: 'Reasoning',
    plan: 'Plan',
}

export function RunProgressCard({
    collapsed,
    currentNodeId,
    isLive,
    progressNodeFilter,
    progressProjection,
    onCollapsedChange,
    onProgressNodeFilterChange,
}: RunProgressCardProps) {
    const currentNodeHasNoContent = progressNodeFilter === 'current' && Boolean(currentNodeId) && !progressProjection.activeEntry
    const progressEntries = (() => {
        if (progressNodeFilter === 'current') {
            return progressProjection.activeEntry
                ? [progressProjection.activeEntry, ...progressProjection.recentEntries]
                : progressProjection.recentEntries
        }
        if (progressNodeFilter === 'recent') {
            return progressProjection.recentEntries
        }
        const entries = progressProjection.activeEntry
            ? [progressProjection.activeEntry, ...progressProjection.recentEntries]
            : progressProjection.recentEntries
        return entries.filter((entry) => entry.nodeId === progressNodeFilter)
    })()

    return (
        <Card data-testid="run-progress-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Progress</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            {progressEntries.length === 1 ? '1 stream' : `${progressEntries.length} streams`}
                        </p>
                    </div>
                    <div className="flex items-center gap-2">
                        <select
                            aria-label="Progress node filter"
                            data-testid="run-progress-node-filter"
                            className="h-7 rounded-md border border-input bg-background px-2 text-xs text-foreground shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
                            value={progressNodeFilter}
                            onChange={(event) => {
                                onProgressNodeFilterChange(event.target.value as RunProgressNodeFilter)
                            }}
                        >
                            <option value="current">Current node</option>
                            <option value="recent">Recent</option>
                            {progressProjection.nodeOptions.map((nodeId) => (
                                <option key={nodeId} value={nodeId}>{nodeId}</option>
                            ))}
                        </select>
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
                        <RunSectionToggleButton
                            collapsed={collapsed}
                            onToggle={() => onCollapsedChange(!collapsed)}
                            testId="run-progress-toggle-button"
                        />
                    </div>
                </div>
            </CardHeader>
            {!collapsed ? (
                <CardContent className="space-y-3 px-4">
                    {currentNodeHasNoContent ? (
                        <div
                            data-testid="run-progress-current-empty-note"
                            className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                        >
                            The current node has not emitted LLM content yet. Showing recent LLM streams.
                        </div>
                    ) : null}
                    {progressEntries.length === 0 ? (
                        <Empty data-testid="run-progress-empty" className="text-sm text-muted-foreground">
                            <EmptyHeader>
                                <EmptyDescription>No LLM progress content has been recorded for this run yet.</EmptyDescription>
                            </EmptyHeader>
                        </Empty>
                    ) : (
                        <div data-testid="run-progress-list" className="max-h-[32rem] space-y-3 overflow-auto pr-1">
                            {progressEntries.map((entry) => (
                                <article
                                    key={entry.id}
                                    data-testid="run-progress-entry"
                                    className="rounded-md border border-border/70 bg-muted/20 px-3 py-2"
                                >
                                    <div className="mb-2 flex flex-wrap items-center gap-2 text-[11px]">
                                        <span
                                            data-testid="run-progress-entry-channel"
                                            className="inline-flex rounded border border-border/80 bg-background px-1.5 py-0.5 font-semibold uppercase tracking-wide text-foreground"
                                        >
                                            {CHANNEL_LABELS[entry.channel]}
                                        </span>
                                        {entry.id === progressProjection.activeEntry?.id ? (
                                            <span
                                                data-testid="run-progress-entry-current-label"
                                                className="inline-flex rounded border border-sky-500/40 bg-sky-500/10 px-1.5 py-0.5 font-semibold uppercase tracking-wide text-sky-700"
                                            >
                                                Current node
                                            </span>
                                        ) : null}
                                        <span
                                            data-testid="run-progress-entry-status"
                                            className={cn(
                                                'inline-flex rounded border px-1.5 py-0.5 uppercase tracking-wide',
                                                entry.status === 'complete'
                                                    ? 'border-green-500/40 bg-green-500/10 text-green-800'
                                                    : 'border-sky-500/40 bg-sky-500/10 text-sky-700',
                                            )}
                                        >
                                            {entry.status === 'complete' ? 'Complete' : 'Streaming'}
                                        </span>
                                        {entry.nodeId ? (
                                            <span data-testid="run-progress-entry-node" className="text-muted-foreground">
                                                Node: {entry.nodeId}
                                            </span>
                                        ) : null}
                                    </div>
                                    <ProjectConversationMarkdown content={entry.content} />
                                </article>
                            ))}
                        </div>
                    )}
                </CardContent>
            ) : null}
        </Card>
    )
}
