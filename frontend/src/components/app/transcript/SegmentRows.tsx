import { memo } from 'react'
import { ChevronDown, ChevronUp } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'

// Shared presentational rows for agent transcripts. A chat turn and a run
// node are the same activity — inference, tool calls, and thinking
// interspersed — so both surfaces render these rows; interactivity that only
// makes sense in one surface (lazy tool-output fetches, plan review) arrives
// through optional props.

export type SurfaceTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger'

const SURFACE_TONE_CLASS_MAP: Record<SurfaceTone, string> = {
    neutral: 'bg-muted/50 text-muted-foreground',
    info: 'bg-sky-500/15 text-sky-700',
    success: 'bg-emerald-500/15 text-emerald-800',
    warning: 'bg-amber-500/15 text-amber-800',
    danger: 'bg-destructive/10 text-destructive',
}

export const getSurfaceToneClassName = (tone: SurfaceTone) => (
    `rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${SURFACE_TONE_CLASS_MAP[tone]}`
)

export interface TranscriptToolCall {
    id: string
    kind: 'command_execution' | 'file_change' | 'dynamic_tool'
    status: 'running' | 'completed' | 'failed'
    title: string
    command?: string | null
    output?: string | null
    outputSize?: number | null
    outputTruncated?: boolean
    filePaths: string[]
}

export interface TranscriptMessageEntry {
    id: string
    role: 'user' | 'assistant'
    content: string
    timestamp: string
    status: string
    error?: string | null
    presentation?: 'default' | 'thinking'
}

export interface TranscriptToolCallEntry {
    id: string
    timestamp: string
    toolCall: TranscriptToolCall
}

export const getToolCallStatusPresentation = (status: 'running' | 'completed' | 'failed') => {
    if (status === 'running') {
        return { label: 'Running', tone: 'info' as const }
    }
    if (status === 'failed') {
        return { label: 'Failed', tone: 'danger' as const }
    }
    return { label: 'Completed', tone: 'success' as const }
}

export const summarizeToolCallDetail = (toolCall: TranscriptToolCall): string | null => {
    if (toolCall.command) {
        return toolCall.command
    }
    if (toolCall.filePaths.length > 0) {
        return toolCall.filePaths[0]
    }
    if (toolCall.output) {
        return toolCall.output.split(/\r?\n/, 1)[0]?.trim() || null
    }
    return null
}

export const parseThinkingSummaryContent = (content: string): { heading: string | null; details: string } => {
    const trimmed = content.trim()
    const headingMatch = trimmed.match(/^\*\*(.+?)\*\*(?:\s*[\r\n]+|\s+|$)/)
    if (!headingMatch) {
        return {
            heading: trimmed.length > 0 ? trimmed : null,
            details: '',
        }
    }
    const heading = headingMatch[1]?.trim() || null
    const details = trimmed.slice(headingMatch[0].length).trim()
    return { heading, details }
}

export const ToolCallRow = memo(function ToolCallRow({
    entry,
    fullOutput = null,
    isLoadingFullOutput = false,
    isExpanded,
    loadFullOutputError = null,
    onToggleToolCallExpanded,
    testIdPrefix = 'project',
}: {
    entry: TranscriptToolCallEntry
    fullOutput?: string | null
    isLoadingFullOutput?: boolean
    isExpanded: boolean
    loadFullOutputError?: string | null
    onToggleToolCallExpanded: (toolCallId: string) => void
    testIdPrefix?: string
}) {
    const statusPresentation = getToolCallStatusPresentation(entry.toolCall.status)
    const summaryDetail = summarizeToolCallDetail(entry.toolCall)
    const displayedOutput = fullOutput ?? entry.toolCall.output
    const hasPreviewOnly = entry.toolCall.outputTruncated === true && fullOutput === null

    return (
        <li className="flex justify-start">
            <div className="w-full rounded-md border border-border bg-muted/40 px-3 py-2">
                <Button
                    type="button"
                    data-testid={`${testIdPrefix}-tool-call-toggle-${entry.toolCall.id}`}
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
                            {entry.toolCall.status === 'running' ? 'Running...' : 'No additional details'}
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
                                    <li key={path} className="font-mono text-[11px] text-muted-foreground">
                                        {path}
                                    </li>
                                ))}
                            </ul>
                        ) : null}
                        {displayedOutput ? (
                            <pre className="max-h-40 overflow-auto rounded border border-border/60 bg-background/80 px-2 py-1 whitespace-pre-wrap font-mono text-[11px] text-muted-foreground">
                                {displayedOutput}
                            </pre>
                        ) : null}
                        {hasPreviewOnly || isLoadingFullOutput || loadFullOutputError ? (
                            <p className="text-[11px] text-muted-foreground">
                                {isLoadingFullOutput
                                    ? 'Loading full output...'
                                    : loadFullOutputError
                                        ? loadFullOutputError
                                        : `Showing preview${entry.toolCall.outputSize ? ` of ${entry.toolCall.outputSize.toLocaleString()} bytes` : ''}.`}
                            </p>
                        ) : null}
                    </div>
                ) : null}
            </div>
        </li>
    )
})

export const ThinkingRow = memo(function ThinkingRow({
    entry,
    formatConversationTimestamp,
    isExpanded,
    onToggleThinkingEntryExpanded,
    testIdPrefix = 'project',
}: {
    entry: TranscriptMessageEntry
    formatConversationTimestamp: (value: string) => string
    isExpanded: boolean
    onToggleThinkingEntryExpanded: (entryId: string) => void
    testIdPrefix?: string
}) {
    const parsedThinking = parseThinkingSummaryContent(entry.content)
    const heading = parsedThinking.heading || 'Thinking...'
    const details = parsedThinking.details
    const isExpandable = details.length > 0

    return (
        <li className="flex justify-start">
            <div className="max-w-[85%] rounded border border-border/80 bg-background px-3 py-2 text-muted-foreground">
                {isExpandable ? (
                    <Button
                        type="button"
                        data-testid={`${testIdPrefix}-thinking-toggle-${entry.id}`}
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
})

export const MessageRow = memo(function MessageRow({
    entry,
    formatConversationTimestamp,
}: {
    entry: TranscriptMessageEntry
    formatConversationTimestamp: (value: string) => string
}) {
    const shouldRenderAssistantMarkdown =
        entry.role === 'assistant'
        && entry.presentation !== 'thinking'
        && entry.status !== 'failed'
        && (entry.status === 'complete' || entry.content.trim().length > 0)
    const literalContent =
        entry.role === 'assistant' && entry.status !== 'complete' && !entry.content.trim()
            ? entry.status === 'failed'
                ? (entry.error || 'Response failed.')
                : 'Thinking...'
            : entry.content

    return (
        <li
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
})
