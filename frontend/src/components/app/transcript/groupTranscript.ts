import type { ConversationSegmentResponse } from '@/lib/workspaceClient'
import type { TranscriptToolCall } from './SegmentRows'

// Transcript grammar: consecutive tool calls fold into one group ("Ran 6 commands · 2 failed").
// Everything else, thinking included, stays its own block and ends the group.
// A subagent's own calls (`parentCallId`) nest under the call that spawned it instead.
// The helper is generic over the consumer's entry type; `classify` says what each is.

export type TranscriptItemClass =
    | { kind: 'tool_call'; toolCall: TranscriptToolCall }
    | { kind: 'other' }

/** A tool call and the calls nested under it. */
export interface TranscriptToolCallNode<T> {
    item: T
    toolCall: TranscriptToolCall
    children: TranscriptToolCallNode<T>[]
}

export interface ToolGroupSummary {
    commands: number
    files: number
    tools: number
    failed: number
    running: boolean
    /** "Ran 3 commands, edited 2 files"; failures and the running state are reported separately. */
    label: string
}

export type TranscriptBlock<T> =
    | { kind: 'tool_group'; id: string; members: TranscriptToolCallNode<T>[]; summary: ToolGroupSummary }
    | { kind: 'item'; item: T }

/** The shared row shape of a segment's tool call. */
export const toTranscriptToolCall = (toolCall: NonNullable<ConversationSegmentResponse['tool_call']>): TranscriptToolCall => ({
    id: toolCall.id,
    kind: toolCall.kind,
    status: toolCall.status,
    completionReason: toolCall.completion_reason ?? null,
    title: toolCall.title,
    command: toolCall.command ?? null,
    output: toolCall.output ?? null,
    outputSize: toolCall.output_size ?? null,
    outputTruncated: toolCall.output_truncated === true,
    filePaths: toolCall.file_paths,
    parentCallId: toolCall.parent_call_id ?? null,
    subagentType: toolCall.subagent_type ?? null,
    prompt: toolCall.prompt ?? null,
    progress: toolCall.summary ?? toolCall.progress ?? null,
    toolUses: toolCall.tool_uses ?? null,
})

export const plural = (count: number, noun: string) => `${count} ${noun}${count === 1 ? '' : 's'}`

export function summarizeToolGroup(toolCalls: readonly TranscriptToolCall[]): ToolGroupSummary {
    const commands = toolCalls.filter((call) => call.kind === 'command_execution').length
    const files = toolCalls.filter((call) => call.kind === 'file_change').length
    const tools = toolCalls.length - commands - files
    const parts = [
        commands > 0 ? `ran ${plural(commands, 'command')}` : null,
        files > 0 ? `edited ${plural(files, 'file')}` : null,
        tools > 0 ? `used ${plural(tools, 'tool')}` : null,
    ].filter((part): part is string => part !== null)
    const label = parts.join(', ')
    return {
        commands,
        files,
        tools,
        failed: toolCalls.filter((call) => call.status === 'failed').length,
        running: toolCalls.some((call) => call.status === 'running'),
        label: label.charAt(0).toUpperCase() + label.slice(1),
    }
}

export function groupTranscript<T>(items: readonly T[], classify: (item: T) => TranscriptItemClass): TranscriptBlock<T>[] {
    const blocks: TranscriptBlock<T>[] = []
    let open: TranscriptToolCallNode<T>[] | null = null

    const closeGroup = () => {
        if (!open) return
        blocks.push({ kind: 'tool_group', id: `tool-group:${open[0].toolCall.id}`, members: open, summary: summarizeToolGroup(open.map((member) => member.toolCall)) })
        open = null
    }

    const nodes = items.map((item): TranscriptToolCallNode<T> | null => {
        const classified = classify(item)
        return classified.kind === 'tool_call' ? { item, toolCall: classified.toolCall, children: [] } : null
    })
    const nodesById = new Map<string, TranscriptToolCallNode<T>>()
    for (const node of nodes) if (node && !nodesById.has(node.toolCall.id)) nodesById.set(node.toolCall.id, node)

    items.forEach((item, index) => {
        const node = nodes[index]
        if (node) {
            // A call whose parent is not in this transcript stays top-level.
            const parent = node.toolCall.parentCallId ? nodesById.get(node.toolCall.parentCallId) : undefined
            if (parent && parent !== node) {
                parent.children.push(node)
                return
            }
            open ??= []
            open.push(node)
        } else {
            closeGroup()
            blocks.push({ kind: 'item', item })
        }
    })
    closeGroup()
    return blocks
}
