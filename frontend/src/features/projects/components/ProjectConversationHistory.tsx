import { ChevronDown, ChevronUp } from 'lucide-react'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
} from '../model/types'
import {
    ProjectFlowLaunchEntry,
    ProjectFlowRunRequestEntry,
} from './ProjectArtifactEntries'
import {
    getFlowLaunchStatusPresentation,
    getFlowRunRequestStatusPresentation,
    getSurfaceToneClassName,
    getToolCallStatusPresentation,
    parseThinkingSummaryContent,
    summarizeToolCallDetail,
} from '../model/presentation'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { ProjectConversationMarkdown } from './ProjectConversationMarkdown'
interface ProjectConversationHistoryProps {
    activeConversationId: string | null
    isConversationHistoryLoading: boolean
    hasRenderableConversationHistory: boolean
    activeConversationHistory: ConversationTimelineEntry[]
    activeFlowRunRequestsById: Map<string, ProjectFlowRunRequest>
    activeFlowLaunchesById: Map<string, ProjectFlowLaunch>
    latestFlowRunRequestId: string | null
    latestFlowLaunchId: string | null
    expandedToolCalls: Record<string, boolean>
    expandedThinkingEntries: Record<string, boolean>
    pendingFlowRunRequestId: string | null
    formatConversationTimestamp: (value: string) => string
    onToggleToolCallExpanded: (toolCallId: string) => void
    onToggleThinkingEntryExpanded: (entryId: string) => void
    onReviewFlowRunRequest: (
        flowRunRequest: ProjectFlowRunRequest,
        disposition: 'approved' | 'rejected',
    ) => void | Promise<void>
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
}

export function ProjectConversationHistory({
    activeConversationId,
    isConversationHistoryLoading,
    hasRenderableConversationHistory,
    activeConversationHistory,
    activeFlowRunRequestsById,
    activeFlowLaunchesById,
    latestFlowRunRequestId,
    latestFlowLaunchId,
    expandedToolCalls,
    expandedThinkingEntries,
    pendingFlowRunRequestId,
    formatConversationTimestamp,
    onToggleToolCallExpanded,
    onToggleThinkingEntryExpanded,
    onReviewFlowRunRequest,
    onOpenFlowRun,
}: ProjectConversationHistoryProps) {
    return (
        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col">
            {isConversationHistoryLoading && !hasRenderableConversationHistory ? (
                <Alert
                    data-testid="project-conversation-history-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        Restoring thread history…
                    </AlertDescription>
                </Alert>
            ) : !hasRenderableConversationHistory ? (
                <p className="rounded-md border border-dashed border-border px-3 py-4 text-sm text-muted-foreground">
                    {activeConversationId
                        ? 'No conversation history for this thread yet.'
                        : 'Create or select a thread to begin chatting.'}
                </p>
            ) : (
                <ol data-testid="project-ai-conversation-history-list" className="space-y-3">
                    {activeConversationHistory.map((entry, index) => {
                        const key = `${entry.id}-${entry.timestamp}-${index}`
                        if (entry.kind === 'tool_call') {
                            const statusPresentation = getToolCallStatusPresentation(entry.toolCall.status)
                            const isExpanded = expandedToolCalls[entry.toolCall.id] === true
                            const summaryDetail = summarizeToolCallDetail(entry.toolCall)
                            return (
                                <li key={key} className="flex justify-start">
                                    <div className="w-full rounded-md border border-border bg-muted/40 px-3 py-2">
                                        <Button
                                            type="button"
                                            data-testid={`project-tool-call-toggle-${entry.toolCall.id}`}
                                            aria-expanded={isExpanded}
                                            onClick={() => onToggleToolCallExpanded(entry.toolCall.id)}
                                            variant="ghost"
                                            size="sm"
                                            className="h-auto w-full justify-start px-0 py-0 text-left hover:bg-transparent"
                                        >
                                            {isExpanded ? (
                                                <ChevronUp className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                                            ) : (
                                                <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                                            )}
                                            <p className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                                                {entry.toolCall.kind === 'file_change' ? 'File change' : 'Tool call'}
                                            </p>
                                            <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                                                {statusPresentation.label}
                                            </span>
                                            <p className="shrink-0 text-xs font-medium text-foreground">{entry.toolCall.title}</p>
                                            {summaryDetail ? (
                                                <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">
                                                    {summaryDetail}
                                                </p>
                                            ) : (
                                                <p className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                                                    {entry.toolCall.status === 'running' ? 'Running…' : 'No additional details'}
                                                </p>
                                            )}
                                        </Button>
                                        {isExpanded ? (
                                            <div className="mt-2 space-y-2">
                                                {entry.toolCall.command ? (
                                                    <p className="whitespace-pre-wrap rounded border border-border/60 bg-background/80 px-2 py-1 font-mono text-[11px] text-foreground">
                                                        {entry.toolCall.command}
                                                    </p>
                                                ) : null}
                                                {entry.toolCall.filePaths.length > 0 ? (
                                                    <ul className="space-y-1">
                                                        {entry.toolCall.filePaths.map((path) => (
                                                            <li key={`${key}-${path}`} className="font-mono text-[11px] text-muted-foreground">
                                                                {path}
                                                            </li>
                                                        ))}
                                                    </ul>
                                                ) : null}
                                                {entry.toolCall.output ? (
                                                    <pre className="max-h-40 overflow-auto rounded border border-border/60 bg-background/80 px-2 py-1 whitespace-pre-wrap font-mono text-[11px] text-muted-foreground">
                                                        {entry.toolCall.output}
                                                    </pre>
                                                ) : null}
                                            </div>
                                        ) : null}
                                    </div>
                                </li>
                            )
                        }

                        if (entry.kind === 'final_separator') {
                            return (
                                <li key={key} className="flex justify-center">
                                    <div className="flex w-full max-w-[85%] items-center gap-3 py-1 text-[11px] text-muted-foreground">
                                        <span className="h-px flex-1 bg-border" />
                                        <span className="shrink-0 whitespace-nowrap">{entry.label}</span>
                                        <span className="h-px flex-1 bg-border" />
                                    </div>
                                </li>
                            )
                        }

                        if (entry.kind === 'message' && entry.role === 'assistant' && entry.presentation === 'thinking') {
                            const parsedThinking = parseThinkingSummaryContent(entry.content)
                            const heading = parsedThinking.heading || 'Thinking...'
                            const details = parsedThinking.details
                            const isExpandable = details.length > 0
                            const isExpanded = expandedThinkingEntries[entry.id] === true
                            return (
                                <li key={key} className="flex justify-start">
                                    <div className="max-w-[85%] rounded border border-border/80 bg-background px-3 py-2 text-muted-foreground">
                                        {isExpandable ? (
                                            <Button
                                                type="button"
                                                data-testid={`project-thinking-toggle-${entry.id}`}
                                                aria-expanded={isExpanded}
                                                onClick={() => onToggleThinkingEntryExpanded(entry.id)}
                                                variant="ghost"
                                                size="sm"
                                                className="h-auto w-full justify-start px-0 py-0 text-left hover:bg-transparent"
                                            >
                                                {isExpanded ? (
                                                    <ChevronUp className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                                                ) : (
                                                    <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                                                )}
                                                <p className="min-w-0 flex-1 truncate text-xs font-semibold text-foreground">
                                                    {heading}
                                                </p>
                                            </Button>
                                        ) : (
                                            <p className="text-xs font-semibold text-foreground">{heading}</p>
                                        )}
                                        {isExpanded && details ? (
                                            <p className="mt-2 whitespace-pre-wrap text-xs italic leading-5">
                                                {details}
                                            </p>
                                        ) : null}
                                        <p className="mt-1 text-[10px] opacity-70">{formatConversationTimestamp(entry.timestamp)}</p>
                                    </div>
                                </li>
                            )
                        }

                        if (entry.kind === 'flow_run_request') {
                            const flowRunRequest = activeFlowRunRequestsById.get(entry.artifactId) || null
                            const isLatestFlowRunRequest = flowRunRequest?.id === latestFlowRunRequestId
                            return (
                                <li
                                    key={key}
                                    data-testid={isLatestFlowRunRequest ? 'project-flow-run-request-history-row' : undefined}
                                    className="flex justify-start"
                                >
                                    <ProjectFlowRunRequestEntry
                                        flowRunRequest={flowRunRequest}
                                        isLatestFlowRunRequest={isLatestFlowRunRequest}
                                        pendingFlowRunRequestId={pendingFlowRunRequestId}
                                        onReviewFlowRunRequest={onReviewFlowRunRequest}
                                        onOpenFlowRun={onOpenFlowRun}
                                        formatConversationTimestamp={formatConversationTimestamp}
                                        getFlowRunRequestStatusPresentation={getFlowRunRequestStatusPresentation}
                                        getSurfaceToneClassName={getSurfaceToneClassName}
                                    />
                                </li>
                            )
                        }

                        if (entry.kind === 'flow_launch') {
                            const flowLaunch = activeFlowLaunchesById.get(entry.artifactId) || null
                            const isLatestFlowLaunch = flowLaunch?.id === latestFlowLaunchId
                            return (
                                <li
                                    key={key}
                                    data-testid={isLatestFlowLaunch ? 'project-flow-launch-history-row' : undefined}
                                    className="flex justify-start"
                                >
                                    <ProjectFlowLaunchEntry
                                        flowLaunch={flowLaunch}
                                        isLatestFlowLaunch={isLatestFlowLaunch}
                                        onOpenFlowRun={onOpenFlowRun}
                                        formatConversationTimestamp={formatConversationTimestamp}
                                        getFlowLaunchStatusPresentation={getFlowLaunchStatusPresentation}
                                        getSurfaceToneClassName={getSurfaceToneClassName}
                                    />
                                </li>
                            )
                        }

                        if (entry.kind !== 'message') {
                            return null
                        }

                        const shouldRenderAssistantMarkdown =
                            entry.role === 'assistant' &&
                            entry.presentation !== 'thinking' &&
                            entry.status !== 'failed' &&
                            (entry.status === 'complete' || entry.content.trim().length > 0)
                        const literalContent =
                            entry.role === 'assistant' && entry.status !== 'complete' && !entry.content.trim()
                                ? entry.status === 'failed'
                                    ? (entry.error || 'Response failed.')
                                    : 'Thinking...'
                                : entry.content

                        return (
                            <li
                                key={key}
                                className={`flex ${entry.role === 'user' ? 'justify-end' : 'justify-start'}`}
                            >
                                <div
                                    className={`max-w-[85%] rounded border px-3 py-2 ${
                                        entry.role === 'user'
                                            ? 'border-primary/40 bg-primary/10 text-foreground'
                                            : entry.presentation === 'thinking'
                                                ? 'border-border/80 bg-background text-muted-foreground'
                                                : 'border-border bg-muted/40 text-foreground'
                                    }`}
                                >
                                    <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                                        {entry.role === 'assistant'
                                            ? (entry.presentation === 'thinking' ? 'Thinking' : 'Spark')
                                            : entry.role}
                                    </p>
                                    {shouldRenderAssistantMarkdown ? (
                                        <ProjectConversationMarkdown content={entry.content} />
                                    ) : (
                                        <p
                                            className={`whitespace-pre-wrap text-xs leading-5 ${
                                                entry.presentation === 'thinking' ? 'italic' : ''
                                            }`}
                                        >
                                            {literalContent}
                                        </p>
                                    )}
                                    <p className="mt-1 text-[10px] opacity-70">{formatConversationTimestamp(entry.timestamp)}</p>
                                </div>
                            </li>
                        )
                    })}
                </ol>
            )}
        </div>
    )
}
