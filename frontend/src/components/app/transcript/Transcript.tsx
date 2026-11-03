import { Fragment, memo, useCallback, useState, type ComponentProps, type ReactNode } from 'react'
import { TriangleAlert } from 'lucide-react'

import { statusTextClassName } from '@/components/app/status'
import { groupTranscript, type TranscriptItemClass, type TranscriptToolCallNode } from '@/components/app/transcript/groupTranscript'
import {
    AssistantTurnHeader,
    EventLine,
    MessageRow,
    ThinkingRow,
    ToolCallGroupRow,
    ToolCallRow,
} from '@/components/app/transcript/SegmentRows'
import { Empty, EmptyDescription, EmptyHeader } from '@/components/ui/empty'
import { isThinkingEntry } from '@/features/projects/model/conversationTimeline'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { loadConversationSegmentToolOutput } from '@/features/projects/services/conversationArtifacts'
import { formatRunDate } from '@/features/runs/model/runOverviewModel'
import { cn } from '@/lib/utils'

// The one transcript list for chats, missions and run nodes, in the shared grammar:
// assistant turns open with a header, consecutive tool calls fold into a group (a lone
// call stays a plain row), and thinking is hidden until asked for. Surface-specific rows
// (plan review, run cards, mission events) arrive through `renderEntry`.

type Entry = ConversationTimelineEntry
type MessageEntry = Extract<Entry, { kind: 'message' }>
type ToolCallEntry = Extract<Entry, { kind: 'tool_call' }>

export interface TranscriptExpansionState {
    expandedToolCalls: Record<string, boolean>
    expandedThinking: Record<string, boolean>
    onToggleToolCall: (toolCallId: string) => void
    onToggleThinking: (entryId: string) => void
}

export function useTranscriptExpansion(): TranscriptExpansionState {
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinking, setExpandedThinking] = useState<Record<string, boolean>>({})
    const onToggleToolCall = useCallback((toolCallId: string) => {
        setExpandedToolCalls((current) => ({ ...current, [toolCallId]: !current[toolCallId] }))
    }, [])
    const onToggleThinking = useCallback((entryId: string) => {
        setExpandedThinking((current) => ({ ...current, [entryId]: !current[entryId] }))
    }, [])
    return { expandedToolCalls, expandedThinking, onToggleToolCall, onToggleThinking }
}

// A user message keeps its row from pending to recorded, when its timestamp changes.
const entryKey = (entry: Entry) => entry.kind === 'message' && entry.role === 'user'
    ? `user:${entry.id}`
    : `${entry.kind}:${entry.id}:${entry.timestamp}`

type TranscriptTurn =
    | { kind: 'entry'; entry: Entry }
    | { kind: 'assistant'; id: string; timestamp: string; worked: string | null; entries: Entry[] }

/**
 * Splits the flat timeline into user messages, standalone events (mode changes, notices)
 * and assistant turns. The "Worked for 20s" separator the timeline inserts before a turn's
 * final answer becomes the turn header's meta instead of a row of its own.
 */
function groupTurns(entries: readonly Entry[]): TranscriptTurn[] {
    const turns: TranscriptTurn[] = []
    let open: Extract<TranscriptTurn, { kind: 'assistant' }> | null = null
    for (const entry of entries) {
        const worked = entry.kind === 'final_separator' ? entry.label.match(/^Worked(?: for (.+))?$/) : null
        const isUserMessage = entry.kind === 'message' && entry.role === 'user'
        if (isUserMessage || entry.kind === 'mode_change' || (entry.kind === 'final_separator' && !worked)) {
            open = null
            turns.push({ kind: 'entry', entry })
            continue
        }
        if (!open) {
            open = { kind: 'assistant', id: entry.id, timestamp: entry.timestamp, worked: null, entries: [] }
            turns.push(open)
        }
        if (worked) {
            if (worked[1]) open.worked = `worked ${worked[1]}`
        } else {
            open.entries.push(entry)
        }
    }
    return turns
}

const classify = (entry: Entry): TranscriptItemClass => (
    entry.kind === 'tool_call'
        ? { kind: 'tool_call', toolCall: entry.toolCall }
        : isThinkingEntry(entry) ? { kind: 'thinking' } : { kind: 'other' }
)

/** The live "Thinking" marker shown while thinking is hidden and the agent is still at it. */
const ThinkingIndicatorRow = memo(function ThinkingIndicatorRow({
    entry,
    formatTimestamp,
}: {
    entry: MessageEntry
    formatTimestamp: (value: string) => string
}) {
    return (
        <li className="min-w-0 text-xs text-muted-foreground" title={formatTimestamp(entry.timestamp)}>
            <p className="italic">Thinking</p>
        </li>
    )
})

/** A truncated tool call's full output: `{}` while loading. */
type FullOutput = { output?: string; error?: string }

export function Transcript({
    entries,
    showThinking,
    formatTimestamp,
    testIdPrefix,
    turns = true,
    loading = false,
    empty = 'No conversation history for this thread yet.',
    conversation,
    expansion: controlledExpansion,
    attachmentUrl,
    onRetryTurn,
    renderEntry,
    className,
    ...props
}: Omit<ComponentProps<'ol'>, 'children'> & {
    entries: Entry[]
    showThinking: boolean
    formatTimestamp: (value: string) => string
    /** Feeds the row test ids, such as `${testIdPrefix}-tool-group-<call id>`. */
    testIdPrefix: string
    /** Split into user messages, notices and assistant turns with headers. false: one flat list (run nodes). */
    turns?: boolean
    /** "Restoring thread history..." while there are no entries. */
    loading?: boolean
    empty?: ReactNode
    /** Enables fetching a truncated tool call's full output when it opens. Absent: no fetch. */
    conversation?: { id: string; projectPath: string }
    /** Controlled expansion (the chat keeps it per conversation in the store). Absent: local state. */
    expansion?: TranscriptExpansionState
    attachmentUrl?: (attachmentId: string) => string
    onRetryTurn?: (turnId: string) => void
    /** A surface's own rendering for an entry, consulted first for every entry. undefined: the default row; null: hidden. */
    renderEntry?: (entry: Entry) => ReactNode | undefined
}) {
    const localExpansion = useTranscriptExpansion()
    const expansion = controlledExpansion ?? localExpansion
    const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({})
    // Held with its conversation id so a switched conversation reads none of the previous one's outputs.
    const [fullOutputs, setFullOutputs] = useState<{ conversationId: string | null; outputs: Record<string, FullOutput> }>({ conversationId: null, outputs: {} })
    const outputs = conversation && fullOutputs.conversationId === conversation.id ? fullOutputs.outputs : {}
    // eslint-disable-next-line react-hooks/purity -- render-time clock for turn dates; the transcript re-renders on every stream update
    const now = Date.now()

    if (entries.length === 0) {
        return loading ? (
            <p data-testid={`${testIdPrefix}-conversation-history-loading`} className="text-xs text-muted-foreground" aria-live="polite">
                Restoring thread history...
            </p>
        ) : (
            <Empty>
                <EmptyHeader>
                    <EmptyDescription>{empty}</EmptyDescription>
                </EmptyHeader>
            </Empty>
        )
    }

    const custom = (entry: Entry): ReactNode | undefined => {
        const rendered = renderEntry?.(entry)
        return rendered === undefined ? undefined : <Fragment key={entryKey(entry)}>{rendered}</Fragment>
    }

    // A truncated output is fetched in full the first time its call opens.
    const toggleTruncatedToolCall = (entry: ToolCallEntry) => {
        expansion.onToggleToolCall(entry.toolCall.id)
        if (!conversation || expansion.expandedToolCalls[entry.toolCall.id] || outputs[entry.id]) return
        const conversationId = conversation.id
        setFullOutputs((current) => ({
            conversationId,
            outputs: { ...(current.conversationId === conversationId ? current.outputs : {}), [entry.id]: {} },
        }))
        const settle = (output: FullOutput) => setFullOutputs((current) => (
            current.conversationId === conversationId ? { conversationId, outputs: { ...current.outputs, [entry.id]: output } } : current
        ))
        void loadConversationSegmentToolOutput(conversationId, entry.id, conversation.projectPath)
            .then((payload) => settle({ output: payload.output }))
            .catch(() => settle({ error: 'Unable to load full output.' }))
    }

    /** A tool call with any subagent calls nested under it as lines. */
    const renderToolCall = (entry: ToolCallEntry, variant: 'row' | 'line', children: TranscriptToolCallNode<Entry>[]): ReactNode => {
        const rendered = custom(entry)
        if (rendered !== undefined) return rendered
        const output = outputs[entry.id]
        return (
            <ToolCallRow
                key={entryKey(entry)}
                entry={entry}
                variant={variant}
                testIdPrefix={testIdPrefix}
                isExpanded={expansion.expandedToolCalls[entry.toolCall.id] === true}
                // Only a truncated call needs its own handler; the rest share the stable toggle.
                onToggleToolCallExpanded={conversation && entry.toolCall.outputTruncated ? () => toggleTruncatedToolCall(entry) : expansion.onToggleToolCall}
                fullOutput={output?.output ?? null}
                isLoadingFullOutput={output !== undefined && output.output === undefined && output.error === undefined}
                loadFullOutputError={output?.error ?? null}
                childCalls={children.map((child) => child.item.kind === 'tool_call' ? renderToolCall(child.item, 'line', child.children) : null)}
            />
        )
    }

    const renderRow = (entry: Entry): ReactNode => {
        const rendered = custom(entry)
        if (rendered !== undefined) return rendered
        const key = entryKey(entry)
        switch (entry.kind) {
        case 'message':
            if (isThinkingEntry(entry)) {
                if (showThinking) {
                    return (
                        <ThinkingRow
                            key={key}
                            entry={entry}
                            formatConversationTimestamp={formatTimestamp}
                            isExpanded={expansion.expandedThinking[entry.id] === true}
                            onToggleThinkingEntryExpanded={expansion.onToggleThinking}
                            testIdPrefix={testIdPrefix}
                        />
                    )
                }
                // A thought still in progress (or a turn with nothing yet) keeps a bare marker so the agent never looks idle.
                const isLive = entry.status !== 'complete' && entry.status !== 'failed'
                return isLive || !entry.content.trim()
                    ? <ThinkingIndicatorRow key={key} entry={entry} formatTimestamp={formatTimestamp} />
                    : null
            }
            return (
                <MessageRow
                    key={key}
                    entry={entry}
                    enableCopy
                    formatConversationTimestamp={formatTimestamp}
                    attachmentUrl={attachmentUrl}
                    onRetryTurn={onRetryTurn}
                />
            )
        case 'plan':
            return <MessageRow key={key} entry={entry} formatConversationTimestamp={formatTimestamp} />
        case 'tool_call':
            return renderToolCall(entry, 'row', [])
        case 'final_separator':
            return <li key={key}><EventLine data-testid={`${testIdPrefix}-notice-row-${entry.id}`}>{entry.label}</EventLine></li>
        case 'mode_change':
            return (
                <li key={key}>
                    <EventLine data-testid={`${testIdPrefix}-mode-change-row-${entry.id}`}>
                        {entry.mode === 'plan' ? 'Switched to Plan mode' : 'Switched to Chat mode'}
                    </EventLine>
                </li>
            )
        case 'context_compaction':
            return <li key={key}><EventLine data-testid={`${testIdPrefix}-context-compaction-row-${entry.id}`}>{entry.content}</EventLine></li>
        case 'warning':
            return <li key={key}><EventLine data-testid={`${testIdPrefix}-warning-row-${entry.id}`} icon={<TriangleAlert className={statusTextClassName('warning')} />}>{entry.content}</EventLine></li>
        default:
            // Questions and flow requests are only shown by surfaces that act on them.
            return null
        }
    }

    const renderBody = (items: Entry[]) => groupTranscript(items, classify).map((block) => {
        if (block.kind === 'item') return renderRow(block.item)
        const [first] = block.members
        if (block.members.length === 1 && first.kind === 'tool_call' && first.item.kind === 'tool_call') {
            return renderToolCall(first.item, 'row', first.children)
        }
        return (
            <ToolCallGroupRow
                key={block.id}
                summary={block.summary}
                isExpanded={openGroups[block.id] === true}
                onToggle={() => setOpenGroups((current) => ({ ...current, [block.id]: !current[block.id] }))}
                testId={`${testIdPrefix}-tool-group-${first.kind === 'tool_call' ? first.toolCall.id : first.item.id}`}
            >
                {block.members.map((member) => member.kind === 'tool_call'
                    ? (member.item.kind === 'tool_call' ? renderToolCall(member.item, 'line', member.children) : null)
                    : renderRow(member.item))}
            </ToolCallGroupRow>
        )
    })

    return (
        <ol className={cn('flex min-w-0 flex-col', turns ? 'gap-5' : 'gap-3', className)} {...props}>
            {turns ? groupTurns(entries).map((turn) => {
                if (turn.kind === 'entry') return renderRow(turn.entry)
                return (
                    <li key={`turn:${turn.id}`} data-testid={`${testIdPrefix}-assistant-turn-${turn.id}`} className="flex min-w-0 flex-col gap-3">
                        <AssistantTurnHeader
                            meta={[formatRunDate(turn.timestamp, now), turn.worked].filter(Boolean).join(' · ')}
                            title={formatTimestamp(turn.timestamp)}
                        />
                        <ol className="flex min-w-0 flex-col gap-3 pl-8">{renderBody(turn.entries)}</ol>
                    </li>
                )
            }) : renderBody(entries)}
        </ol>
    )
}
