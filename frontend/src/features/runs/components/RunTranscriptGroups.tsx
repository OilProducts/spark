import { useCallback, useState } from 'react'

import { MessageRow, ThinkingRow, ToolCallRow } from '@/components/app/transcript/SegmentRows'
import { formatTimestamp } from '../model/shared'
import type { RunTranscriptGroup, RunTranscriptRow } from '../model/transcriptModel'
import { runTranscriptGroupLabel } from '../model/transcriptModel'

// Run transcripts render the same shared segment rows the chat surface uses;
// this module adds the run-specific grouping (node, attempt, child flow).

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

export function RunTranscriptGroupSection({
    group,
    expansion,
}: {
    group: RunTranscriptGroup
    expansion: TranscriptExpansionState
}) {
    return (
        <section
            data-testid="run-transcript-group"
            data-node-id={group.nodeId ?? undefined}
            data-attempt={group.attempt}
            className="space-y-2"
        >
            <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                {runTranscriptGroupLabel(group)}
            </p>
            <ul className="list-none space-y-2">
                {group.rows.map((row) => (
                    <RunTranscriptRowItem
                        key={row.segment.id}
                        row={row}
                        expansion={expansion}
                    />
                ))}
            </ul>
        </section>
    )
}
