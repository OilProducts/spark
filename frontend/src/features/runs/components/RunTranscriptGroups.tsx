import { useCallback, useMemo, useState, type ComponentProps } from 'react'

import { groupTranscript, type TranscriptItemClass } from '@/components/app/transcript/groupTranscript'
import {
    MessageRow,
    ThinkingRow,
    ToolCallGroupRow,
    ToolCallRow,
} from '@/components/app/transcript/SegmentRows'
import { cn } from '@/lib/utils'
import { formatTimestamp } from '../model/shared'
import type { RunTranscriptRow } from '../model/transcriptModel'

// Run transcripts render the same shared segment rows the chat surface uses, in the
// shared grammar: consecutive tool calls fold into one group row, thinking is hidden
// until asked for, and prose stands on its own.

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

export function RunTranscriptRowItem({
    row,
    expansion,
}: {
    row: RunTranscriptRow
    expansion: TranscriptExpansionState
}) {
    if (row.kind === 'tool_call') {
        return (
            <ToolCallRow
                entry={row.entry}
                isExpanded={expansion.expandedToolCalls[row.entry.toolCall.id] === true}
                onToggleToolCallExpanded={expansion.onToggleToolCall}
                testIdPrefix="run"
            />
        )
    }
    if (row.kind === 'thinking') {
        return (
            <ThinkingRow
                entry={row.entry}
                formatConversationTimestamp={formatTimestamp}
                isExpanded={expansion.expandedThinking[row.entry.id] === true}
                onToggleThinkingEntryExpanded={expansion.onToggleThinking}
                testIdPrefix="run"
            />
        )
    }
    return <MessageRow entry={row.entry} formatConversationTimestamp={formatTimestamp} />
}

const classifyRow = (row: RunTranscriptRow): TranscriptItemClass => (
    row.kind === 'tool_call'
        ? { kind: 'tool_call', toolCall: row.entry.toolCall }
        : row.kind === 'thinking'
            ? { kind: 'thinking' }
            : { kind: 'other' }
)

/** One visit's transcript in the shared grammar; `showThinking` is owned by the caller's toggle. */
export function RunTranscript({
    rows,
    showThinking,
    className,
    ...props
}: Omit<ComponentProps<'ul'>, 'children'> & { rows: RunTranscriptRow[]; showThinking: boolean }) {
    const expansion = useTranscriptExpansion()
    const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({})
    const blocks = useMemo(() => groupTranscript(rows, classifyRow), [rows])
    return (
        <ul className={cn('flex max-w-[80ch] list-none flex-col gap-3', className)} {...props}>
            {blocks.map((block) => {
                if (block.kind === 'tool_group') {
                    return (
                        <ToolCallGroupRow
                            key={block.id}
                            summary={block.summary}
                            isExpanded={openGroups[block.id] === true}
                            onToggle={() => setOpenGroups((current) => ({ ...current, [block.id]: !current[block.id] }))}
                            testId={`run-tool-group-${block.id}`}
                        >
                            {block.members.map((member) => (
                                member.item.kind === 'tool_call' ? (
                                    <ToolCallRow
                                        key={member.item.segment.id}
                                        variant="line"
                                        entry={member.item.entry}
                                        isExpanded={expansion.expandedToolCalls[member.item.entry.toolCall.id] === true}
                                        onToggleToolCallExpanded={expansion.onToggleToolCall}
                                        testIdPrefix="run"
                                    />
                                ) : showThinking ? (
                                    <RunTranscriptRowItem key={member.item.segment.id} row={member.item} expansion={expansion} />
                                ) : null
                            ))}
                        </ToolCallGroupRow>
                    )
                }
                if (block.item.kind === 'thinking' && !showThinking) {
                    return null
                }
                return <RunTranscriptRowItem key={block.item.segment.id} row={block.item} expansion={expansion} />
            })}
        </ul>
    )
}
