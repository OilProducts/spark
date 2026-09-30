import { memo } from 'react'
import { ClaudeCodeReconnect } from '@/features/settings/ClaudeCodeConnectionSettings'
import { CodexReconnect } from '@/features/settings/CodexConnectionSettings'
import { isClaudeCodeAuthError } from '@/features/settings/services/claudeCodeConnection'
import { isCodexAuthError } from '@/features/settings/services/codexConnection'
import { ChevronRight } from 'lucide-react'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { TranscriptCopyButton } from './TranscriptCopyButton'

// Shared presentational rows for agent transcripts. A chat turn and a run
// node are the same activity — inference, tool calls, and thinking
// interspersed — so both surfaces render these rows; interactivity that only
// makes sense in one surface (lazy tool-output fetches, plan review) arrives
// through optional props.

export type SurfaceTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger'

const SURFACE_TONE_CLASS_MAP: Record<SurfaceTone, string> = {
    neutral: 'text-muted-foreground',
    info: 'text-info',
    success: 'text-success',
    warning: 'text-warning',
    danger: 'text-destructive',
}

export const getSurfaceToneClassName = (tone: SurfaceTone) => (
    `rounded border border-border px-2 py-0.5 text-xs font-medium uppercase tracking-wide ${SURFACE_TONE_CLASS_MAP[tone]}`
)

export interface TranscriptToolCall {
    id: string
    kind: 'command_execution' | 'file_change' | 'dynamic_tool'
    status: 'running' | 'completed' | 'failed' | 'yielded'
    completionReason?: string | null
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

export const getToolCallStatusPresentation = (status: 'running' | 'completed' | 'failed' | 'yielded') => {
    if (status === 'running') {
        return { label: 'Running', tone: 'info' as const }
    }
    if (status === 'failed') {
        return { label: 'Failed', tone: 'danger' as const }
    }
    if (status === 'yielded') {
        return { label: 'Yielded', tone: 'neutral' as const }
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
            heading: null,
            details: content,
        }
    }
    const heading = headingMatch[1]?.trim() || null
    const details = trimmed.slice(headingMatch[0].length).trim()
    return { heading, details }
}

// Air transcript: one line per step, no boxes. A tool call is its command in
// muted mono with a status word only when it didn't simply complete; a thought
// is a muted italic line; a reply is plain text. Timestamps sit in tooltips.

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
    const isCommand = entry.toolCall.kind === 'command_execution' && Boolean(entry.toolCall.command)

    return (
        <li className="min-w-0">
            <button
                type="button"
                data-testid={`${testIdPrefix}-tool-call-toggle-${entry.toolCall.id}`}
                aria-expanded={isExpanded}
                onClick={() => onToggleToolCallExpanded(entry.toolCall.id)}
                className="group flex w-full min-w-0 items-center gap-2 text-left text-xs"
            >
                <ChevronRight className={`h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform ${isExpanded ? 'rotate-90' : ''}`} />
                {isCommand ? null : (
                    <span className="shrink-0 text-muted-foreground">{entry.toolCall.title}</span>
                )}
                <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground group-hover:text-foreground">
                    {summaryDetail ?? (entry.toolCall.status === 'running' ? '' : 'No additional details')}
                </span>
                {entry.toolCall.status === 'completed' ? null : (
                    <span className={`shrink-0 ${SURFACE_TONE_CLASS_MAP[statusPresentation.tone]}`}>
                        {statusPresentation.label}
                    </span>
                )}
            </button>
            {isExpanded ? (
                <div className="mt-1.5 mb-2 space-y-2 pl-5">
                    {entry.toolCall.command ? (
                        <p className="whitespace-pre-wrap break-words rounded bg-muted px-2 py-1 font-mono text-xs text-foreground [overflow-wrap:anywhere]">
                            {entry.toolCall.command}
                        </p>
                    ) : null}
                    {entry.toolCall.filePaths.length > 0 ? (
                        <ul className="space-y-1">
                            {entry.toolCall.filePaths.map((path) => (
                                <li key={path} className="break-words font-mono text-xs text-muted-foreground [overflow-wrap:anywhere]">
                                    {path}
                                </li>
                            ))}
                        </ul>
                    ) : null}
                    {displayedOutput ? (
                        <pre className="max-h-40 max-w-full overflow-x-hidden overflow-y-auto whitespace-pre-wrap break-words rounded bg-muted px-2 py-1 font-mono text-xs text-muted-foreground [overflow-wrap:anywhere]">
                            {displayedOutput}
                        </pre>
                    ) : null}
                    {hasPreviewOnly || isLoadingFullOutput || loadFullOutputError ? (
                        <p className="text-xs text-muted-foreground">
                            {isLoadingFullOutput
                                ? 'Loading full output...'
                                : loadFullOutputError
                                    ? loadFullOutputError
                                    : `Showing preview${entry.toolCall.outputSize ? ` of ${entry.toolCall.outputSize.toLocaleString()} bytes` : ''}.`}
                        </p>
                    ) : null}
                </div>
            ) : null}
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
    const heading = (
        <>
            {parsedThinking.heading ? <span aria-hidden="true">Thought: </span> : null}
            <span>{parsedThinking.heading || 'Thinking'}</span>
        </>
    )
    const details = parsedThinking.details
    const isExpandable = details.length > 0

    return (
        <li className="min-w-0 text-xs text-muted-foreground" title={formatConversationTimestamp(entry.timestamp)}>
            {isExpandable ? (
                <button
                    type="button"
                    data-testid={`${testIdPrefix}-thinking-toggle-${entry.id}`}
                    aria-expanded={isExpanded}
                    onClick={() => onToggleThinkingEntryExpanded(entry.id)}
                    className="max-w-full truncate text-left italic hover:text-foreground"
                >
                    {heading}
                </button>
            ) : (
                <p className="italic">{heading}</p>
            )}
            {isExpanded && details ? (
                <div className="mt-1 mb-2 border-l border-border pl-3 text-sm">
                    <ProjectConversationMarkdown content={details} />
                </div>
            ) : null}
        </li>
    )
})

export const MessageRow = memo(function MessageRow({
    entry,
    enableCopy = false,
    formatConversationTimestamp,
}: {
    entry: TranscriptMessageEntry
    enableCopy?: boolean
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
    const needsCodexLogin = entry.role === 'assistant' && entry.status === 'failed'
        && isCodexAuthError(entry.error || entry.content)
    const needsClaudeLogin = entry.role === 'assistant' && entry.status === 'failed'
        && isClaudeCodeAuthError(entry.error || entry.content)
    const canCopy = enableCopy && (
        entry.role === 'user'
        || (
            entry.role === 'assistant'
            && entry.status === 'complete'
            && entry.presentation !== 'thinking'
            && entry.content.length > 0
        )
    )

    return (
        <li className={`flex min-w-0 ${entry.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
                title={formatConversationTimestamp(entry.timestamp)}
                className={`group relative min-w-0 ${
                    entry.role === 'user'
                        ? 'max-w-[85%] rounded-md bg-muted px-3 py-2 text-foreground'
                        : entry.presentation === 'thinking'
                            ? 'max-w-[80ch] text-muted-foreground'
                            : entry.status === 'failed'
                                ? 'max-w-[80ch] text-destructive'
                                : 'max-w-[80ch] text-foreground'
                }`}
            >
                {canCopy ? (
                    <div className="absolute -top-1 right-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
                        <TranscriptCopyButton label="Copy message" text={entry.content} />
                    </div>
                ) : null}
                <span className="sr-only">{entry.role === 'user' ? 'You' : entry.presentation === 'thinking' ? 'Thinking' : 'Spark'}: </span>
                {shouldRenderAssistantMarkdown ? (
                    <ProjectConversationMarkdown content={entry.content} enableCodeCopy={enableCopy && entry.status === 'complete'} />
                ) : (
                    <p
                        className={`whitespace-pre-wrap text-sm leading-6 ${
                            entry.presentation === 'thinking' ? 'italic' : ''
                        }`}
                    >
                        {needsCodexLogin && isCodexAuthError(literalContent)
                            ? 'Your Codex connection needs sign-in. Your conversation is saved.'
                            : needsClaudeLogin && isClaudeCodeAuthError(literalContent)
                                ? 'Claude Code needs sign-in. Your conversation is saved.' : literalContent}
                    </p>
                )}
                {needsCodexLogin && <CodexReconnect />}
                {needsClaudeLogin && <ClaudeCodeReconnect />}
            </div>
        </li>
    )
})
