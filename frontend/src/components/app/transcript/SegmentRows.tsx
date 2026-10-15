import { memo, useState, type ComponentProps, type ReactNode } from 'react'
import { ClaudeCodeReconnect } from '@/features/settings/ClaudeCodeConnectionSettings'
import { CodexReconnect } from '@/features/settings/CodexConnectionSettings'
import { isClaudeCodeAuthError } from '@/features/settings/services/claudeCodeConnection'
import { isCodexAuthError } from '@/features/settings/services/codexConnection'
import { ChevronRight, FileText, Play, Terminal } from 'lucide-react'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { formatAttachmentSize } from '@/features/projects/hooks/useComposerAttachments'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { StatusDot, statusTextClassName, type StatusTone } from '../status'
import { stripShellWrapper } from './displayCommand'
import { plural, type ToolGroupSummary } from './groupTranscript'
import { TranscriptCopyButton } from './TranscriptCopyButton'

// Shared presentational rows for agent transcripts. A chat turn, a mission log and a
// run node are the same activity — inference, tool calls, and thinking interspersed —
// so all three surfaces render these rows; interactivity that only makes sense in one
// surface (lazy tool-output fetches, plan review) arrives through optional props.
//
// Grammar: consecutive tool calls fold into a ToolCallGroupRow (see groupTranscript.ts)
// whose expanded body lists each call as a compact ToolCallRow variant="line"; thinking
// is hidden until the ShowThinkingToggle is pressed; assistant turns open with an
// AssistantTurnHeader; things that happen to a mission or chat are EventLines or RunCards.

export type SurfaceTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger'

/** A small bordered status word in the tone's colour (sentence case, no tracking). */
export const getSurfaceToneClassName = (tone: SurfaceTone) => (
    `rounded-md border border-border px-1.5 py-0.5 text-xs font-medium ${statusTextClassName(tone)}`
)

export interface TranscriptToolCall {
    id: string
    kind: 'command_execution' | 'file_change' | 'dynamic_tool' | 'subagent'
    status: 'running' | 'completed' | 'failed' | 'yielded'
    completionReason?: string | null
    title: string
    command?: string | null
    output?: string | null
    outputSize?: number | null
    outputTruncated?: boolean
    filePaths: string[]
    /** The subagent call this call was made under; it renders nested there. */
    parentCallId?: string | null
    subagentType?: string | null
    prompt?: string | null
    /** A subagent's latest progress, or its final summary. */
    progress?: string | null
    /** The subagent's own count of its tool calls, which may exceed the calls received. */
    toolUses?: number | null
}


export interface TranscriptAttachment {
    id: string
    filename: string
    media_type: string
    size_bytes: number
}

export interface TranscriptMessageEntry {
    id: string
    /** The assistant turn this entry belongs to. */
    turnId?: string
    role: 'user' | 'assistant'
    content: string
    timestamp: string
    status: string
    error?: string | null
    presentation?: 'default' | 'thinking'
    attachments?: TranscriptAttachment[]
}

const INLINE_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']

/** A sent message's files: image previews and download links. */
function MessageAttachments({
    attachments,
    attachmentUrl,
}: {
    attachments: TranscriptAttachment[]
    attachmentUrl: (attachmentId: string) => string
}) {
    return (
        <ul aria-label="Attachments" data-testid="message-attachments" className="mt-1.5 flex flex-wrap gap-2">
            {attachments.map((attachment) => {
                const url = attachmentUrl(attachment.id)
                return (
                    <li key={attachment.id} className="min-w-0">
                        {INLINE_IMAGE_TYPES.includes(attachment.media_type) ? (
                            <a href={url} target="_blank" rel="noreferrer" title={attachment.filename}>
                                <img src={url} alt={attachment.filename} loading="lazy" className="max-h-48 max-w-full rounded-md border border-border object-contain" />
                            </a>
                        ) : (
                            <a
                                href={url}
                                download={attachment.filename}
                                className="inline-flex max-w-full items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1 text-xs underline-offset-2 hover:underline"
                            >
                                <FileText aria-hidden="true" className="size-3.5 shrink-0" />
                                <span className="truncate">{attachment.filename}</span>
                                <span className="shrink-0 text-muted-foreground">{formatAttachmentSize(attachment.size_bytes)}</span>
                            </a>
                        )}
                    </li>
                )
            })}
        </ul>
    )
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
    if (toolCall.progress) {
        return toolCall.progress
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

/** The expanded body of a tool call: the full command, touched files and output. */
function ToolCallDetails({
    entry,
    fullOutput,
    isLoadingFullOutput,
    loadFullOutputError,
    className,
}: {
    entry: TranscriptToolCallEntry
    fullOutput: string | null
    isLoadingFullOutput: boolean
    loadFullOutputError: string | null
    className?: string
}) {
    const displayedOutput = fullOutput ?? entry.toolCall.output
    const hasPreviewOnly = entry.toolCall.outputTruncated === true && fullOutput === null
    return (
        <div className={cn('mt-1.5 mb-2 space-y-2', className)}>
            {entry.toolCall.prompt ? (
                <p className="whitespace-pre-wrap break-words rounded-md bg-muted px-2 py-1 text-xs text-foreground [overflow-wrap:anywhere]">
                    {entry.toolCall.prompt}
                </p>
            ) : null}
            {entry.toolCall.command ? (
                <p className="whitespace-pre-wrap break-words rounded-md bg-muted px-2 py-1 font-mono text-xs text-foreground [overflow-wrap:anywhere]">
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
                <pre className="max-h-40 max-w-full overflow-x-hidden overflow-y-auto whitespace-pre-wrap break-words rounded-md bg-muted px-2 py-1 font-mono text-xs text-muted-foreground [overflow-wrap:anywhere]">
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
    )
}

/** A subagent call's own calls: "3 tool calls", collapsed until opened, then indented below. */
function NestedToolCalls({ parentId, testIdPrefix, count, children }: { parentId: string; testIdPrefix: string; count: number; children: ReactNode[] }) {
    const [isExpanded, setIsExpanded] = useState(false)
    if (children.length === 0) {
        return <p data-testid={`${testIdPrefix}-tool-call-children-${parentId}`} className="ml-5 text-xs text-muted-foreground">{plural(count, 'tool call')}</p>
    }
    return (
        <div className="ml-5 min-w-0">
            <button
                type="button"
                data-testid={`${testIdPrefix}-tool-call-children-${parentId}`}
                aria-expanded={isExpanded}
                onClick={() => setIsExpanded((current) => !current)}
                className="inline-flex items-center gap-1 text-xs text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
                <ChevronRight aria-hidden="true" className={cn('size-3 shrink-0 transition-transform', isExpanded && 'rotate-90')} />
                {plural(count, 'tool call')}
            </button>
            {isExpanded ? (
                <ul className="mt-1 flex min-w-0 flex-col gap-0.5 border-l border-border pl-3">{children}</ul>
            ) : null}
        </div>
    )
}

/**
 * One tool call. `variant="row"` (default) is the standalone chevron row; `variant="line"`
 * is the compact monospace line inside an expanded ToolCallGroupRow. Both show the
 * command without its shell wrapper, keep the full command in the tooltip and the
 * expanded body, and mark anything that did not simply complete. A subagent call shows
 * its type and lists the calls it made (`childCalls`, rendered by the surface) beneath.
 */
export const ToolCallRow = memo(function ToolCallRow({
    entry,
    fullOutput = null,
    isLoadingFullOutput = false,
    isExpanded,
    loadFullOutputError = null,
    onToggleToolCallExpanded,
    testIdPrefix = 'project',
    variant = 'row',
    childCalls,
}: {
    entry: TranscriptToolCallEntry
    fullOutput?: string | null
    isLoadingFullOutput?: boolean
    isExpanded: boolean
    loadFullOutputError?: string | null
    onToggleToolCallExpanded: (toolCallId: string) => void
    testIdPrefix?: string
    variant?: 'row' | 'line'
    childCalls?: ReactNode[]
}) {
    const statusPresentation = getToolCallStatusPresentation(entry.toolCall.status)
    const summaryDetail = entry.toolCall.command
        ? stripShellWrapper(entry.toolCall.command)
        : summarizeToolCallDetail(entry.toolCall)
    const isCommand = entry.toolCall.kind === 'command_execution' && Boolean(entry.toolCall.command)
    const statusMark = entry.toolCall.status === 'completed' ? null : (
        <span className={`shrink-0 font-sans ${statusTextClassName(statusPresentation.tone)}`}>
            {statusPresentation.label}
        </span>
    )
    const details = isExpanded ? (
        <ToolCallDetails
            entry={entry}
            fullOutput={fullOutput}
            isLoadingFullOutput={isLoadingFullOutput}
            loadFullOutputError={loadFullOutputError}
            className={variant === 'row' ? 'pl-5' : undefined}
        />
    ) : null
    const subagentType = entry.toolCall.subagentType ? (
        <span className="shrink-0 font-sans text-muted-foreground/70">{entry.toolCall.subagentType}</span>
    ) : null
    const nestedCount = Math.max(childCalls?.length ?? 0, entry.toolCall.toolUses ?? 0)
    const nested = nestedCount > 0 ? (
        <NestedToolCalls parentId={entry.toolCall.id} testIdPrefix={testIdPrefix} count={nestedCount}>{childCalls ?? []}</NestedToolCalls>
    ) : null

    if (variant === 'line') {
        return (
            <li className="min-w-0">
                <button
                    type="button"
                    data-testid={`${testIdPrefix}-tool-call-toggle-${entry.toolCall.id}`}
                    aria-expanded={isExpanded}
                    title={entry.toolCall.command ?? undefined}
                    onClick={() => onToggleToolCallExpanded(entry.toolCall.id)}
                    className="flex w-full min-w-0 items-center gap-2.5 rounded-sm text-left font-mono text-xs leading-6 text-foreground/80 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                >
                    {isCommand ? null : (
                        <span className="shrink-0 font-sans text-muted-foreground">{entry.toolCall.title}</span>
                    )}
                    {subagentType}
                    <span className="min-w-0 flex-1 truncate">
                        {summaryDetail ?? (entry.toolCall.status === 'running' ? '' : 'No additional details')}
                    </span>
                    {statusMark}
                </button>
                {details}
                {nested}
            </li>
        )
    }

    return (
        <li className="min-w-0">
            <button
                type="button"
                data-testid={`${testIdPrefix}-tool-call-toggle-${entry.toolCall.id}`}
                aria-expanded={isExpanded}
                title={entry.toolCall.command ?? undefined}
                onClick={() => onToggleToolCallExpanded(entry.toolCall.id)}
                className="group flex w-full min-w-0 items-center gap-2 text-left text-xs"
            >
                <ChevronRight className={`h-3 w-3 shrink-0 text-muted-foreground/50 transition-transform ${isExpanded ? 'rotate-90' : ''}`} />
                {isCommand ? null : (
                    <span className="shrink-0 text-muted-foreground">{entry.toolCall.title}</span>
                )}
                {subagentType}
                <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground group-hover:text-foreground">
                    {summaryDetail ?? (entry.toolCall.status === 'running' ? '' : 'No additional details')}
                </span>
                {statusMark}
            </button>
            {details}
            {nested}
        </li>
    )
})

/**
 * "Ran 6 commands · 2 failed" with a chevron; when expanded, renders its children (the
 * calls as `<ToolCallRow variant="line">`, and thinking rows when shown) in a muted well.
 * A group with a running call also says "Running…".
 */
export function ToolCallGroupRow({
    summary,
    isExpanded,
    onToggle,
    testId,
    className,
    children,
}: {
    summary: ToolGroupSummary
    isExpanded: boolean
    onToggle: () => void
    testId?: string
    className?: string
    children?: ReactNode
}) {
    return (
        <li className={cn('min-w-0', className)}>
            <button
                type="button"
                data-testid={testId}
                aria-expanded={isExpanded}
                onClick={onToggle}
                className="inline-flex h-7.5 max-w-full items-center gap-2 rounded-lg border border-border bg-background px-2.5 text-xs text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
                <Terminal aria-hidden="true" className="size-3.5 shrink-0" />
                <span className="truncate">{summary.label}</span>
                {summary.failed > 0 ? <span className="shrink-0 text-destructive">· {summary.failed} failed</span> : null}
                {summary.running ? <span className="shrink-0 text-info">· Running…</span> : null}
                <ChevronRight aria-hidden="true" className={cn('size-3.5 shrink-0 transition-transform', isExpanded && 'rotate-90')} />
            </button>
            {isExpanded ? (
                <ul className="mt-2 flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted px-3 py-2">{children}</ul>
            ) : null}
        </li>
    )
}

/** The per-transcript toggle that reveals thinking rows; `aria-pressed` carries the state. */
export function ShowThinkingToggle({
    pressed,
    onPressedChange,
    className,
    ...props
}: Omit<ComponentProps<typeof Button>, 'onClick' | 'children'> & { pressed: boolean; onPressedChange: (next: boolean) => void }) {
    return (
        <Button
            type="button"
            variant="ghost"
            size="xs"
            aria-pressed={pressed}
            onClick={() => onPressedChange(!pressed)}
            className={cn('text-muted-foreground aria-pressed:text-foreground', className)}
            {...props}
        >
            Show thinking
        </Button>
    )
}

/** The Spark mark: the app icon at 22px. */
export function SparkMark({ className }: { className?: string }) {
    return (
        <img
            src="/assets/spark-app-icon.png"
            alt=""
            aria-hidden="true"
            width={22}
            height={22}
            className={cn('size-5.5 shrink-0 rounded-md', className)}
        />
    )
}

/** Opens an assistant turn: the mark, "Spark", and muted meta such as "Oct 1 · worked 39s". */
export function AssistantTurnHeader({ meta, className, ...props }: ComponentProps<'div'> & { meta?: ReactNode }) {
    return (
        <div className={cn('flex items-center gap-2', className)} {...props}>
            <SparkMark />
            <span className="text-sm font-semibold text-foreground">Spark</span>
            {meta ? <span className="truncate text-xs text-muted-foreground">{meta}</span> : null}
        </div>
    )
}

/**
 * A quiet line for something that happened to a mission or chat (started, a run ended,
 * a mode change). Wrap it in the list's `<li>`. Links and `<b>` inside read in the
 * foreground colour so the subject stands out from the verb.
 */
export function EventLine({
    icon,
    at,
    className,
    children,
    ...props
}: ComponentProps<'div'> & { icon?: ReactNode; at?: ReactNode }) {
    return (
        <div className={cn('flex min-w-0 items-center gap-2.5 py-1 text-xs text-muted-foreground', className)} {...props}>
            <span aria-hidden="true" className="flex size-4 shrink-0 items-center justify-center [&_svg]:size-3.5">{icon}</span>
            <span className="min-w-0 flex-1 truncate [&_a]:text-foreground [&_a:hover]:underline [&_b]:font-normal [&_b]:text-foreground">{children}</span>
            {at ? <span className="shrink-0 tabular-nums">{at}</span> : null}
        </div>
    )
}

/**
 * A launched run as a bordered card: run icon, kicker ("Launched run · Implement
 * Change"), the title (pass the surface's own link), and live status with duration.
 * Wrap it in the list's `<li>`.
 */
export function RunCard({
    kicker,
    title,
    tone,
    status,
    duration,
    className,
    ...props
}: Omit<ComponentProps<'div'>, 'title'> & { kicker: ReactNode; title: ReactNode; tone: StatusTone; status: ReactNode; duration?: ReactNode }) {
    return (
        <div className={cn('flex items-center gap-3 rounded-xl border border-border bg-background px-3 py-2.5', className)} {...props}>
            <span aria-hidden="true" className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-muted text-primary">
                <Play className="size-3.5" />
            </span>
            <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-xs text-muted-foreground">{kicker}</span>
                <span className="truncate text-sm font-medium text-foreground [&_a]:text-foreground [&_a:hover]:underline">{title}</span>
            </span>
            <span className={cn('inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap text-xs', statusTextClassName(tone))}>
                <StatusDot tone={tone} className="size-1.5" />
                {status}
                {duration ? <span className="text-muted-foreground"> · {duration}</span> : null}
            </span>
        </div>
    )
}

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
    const heading = parsedThinking.heading || 'Thinking'
    const details = parsedThinking.details
    const isExpandable = details.length > 0

    return (
        <li className="min-w-0 font-sans text-xs text-muted-foreground" title={formatConversationTimestamp(entry.timestamp)}>
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
    attachmentUrl,
    onRetryTurn,
}: {
    entry: TranscriptMessageEntry
    enableCopy?: boolean
    formatConversationTimestamp: (value: string) => string
    /** Where a message's attachments download from; without it they are not shown. */
    attachmentUrl?: (attachmentId: string) => string
    /** Resends a failed turn's message; without it failed turns offer no retry. */
    onRetryTurn?: (turnId: string) => void
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
    const turnId = entry.turnId
    const retry = entry.role === 'assistant' && entry.status === 'failed' && turnId && onRetryTurn
        ? () => onRetryTurn(turnId) : undefined
    // A message sent while a turn ran: queued behind it, delivered into it, or not sent.
    const delivery = entry.role !== 'user' ? null
        : entry.status === 'queued' ? 'Queued · starts when the current turn ends'
            : entry.status === 'delivered' ? 'Sent into the running turn'
                : entry.status === 'failed' ? (entry.error || 'Not sent')
                    : null
    const canCopy = enableCopy && (
        (entry.role === 'user' && entry.content.length > 0)
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
                        ? 'max-w-[80%] rounded-xl rounded-br-sm bg-accent px-3.5 py-2.5 text-foreground'
                        : entry.presentation === 'thinking'
                            ? 'max-w-[72ch] text-muted-foreground'
                            : entry.status === 'failed'
                                ? 'max-w-[72ch] text-destructive'
                                : 'max-w-[72ch] text-foreground'
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
                ) : literalContent ? (
                    <p
                        className={`whitespace-pre-wrap text-sm leading-6 [overflow-wrap:anywhere] ${
                            entry.presentation === 'thinking' ? 'italic' : ''
                        }`}
                    >
                        {needsCodexLogin && isCodexAuthError(literalContent)
                            ? 'Your Codex connection needs sign-in. Your conversation is saved.'
                            : needsClaudeLogin && isClaudeCodeAuthError(literalContent)
                                ? 'Claude Code needs sign-in. Your conversation is saved.' : literalContent}
                    </p>
                ) : null}
                {entry.attachments?.length && attachmentUrl ? (
                    <MessageAttachments attachments={entry.attachments} attachmentUrl={attachmentUrl} />
                ) : null}
                {delivery ? (
                    <p data-testid="message-delivery" className={`mt-1 text-xs ${entry.status === 'failed' ? 'text-destructive' : 'text-muted-foreground'}`}>
                        {delivery}
                    </p>
                ) : null}
                {(needsCodexLogin || needsClaudeLogin || retry) && <div className="flex flex-wrap gap-2">
                    {needsCodexLogin && <CodexReconnect />}
                    {needsClaudeLogin && <ClaudeCodeReconnect onRetry={retry} />}
                    {retry && <Button size="sm" variant="outline" className="mt-2" onClick={retry}>Retry</Button>}
                </div>}
            </div>
        </li>
    )
})
