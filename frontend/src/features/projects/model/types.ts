import type {
    ConversationTurnResponse,
} from '@/lib/workspaceClient'

export interface ConversationTimelineToolCall {
    id: string
    kind: 'command_execution' | 'file_change' | 'dynamic_tool'
    status: 'running' | 'completed' | 'failed'
    title: string
    command?: string | null
    output?: string | null
    filePaths: string[]
}

export type ConversationTimelineEntry =
    | {
        id: string
        kind: 'message'
        role: 'user' | 'assistant'
        content: string
        timestamp: string
        status: ConversationTurnResponse['status']
        error?: string | null
        presentation?: 'default' | 'thinking'
    }
    | {
        id: string
        kind: 'tool_call'
        role: 'system'
        timestamp: string
        toolCall: ConversationTimelineToolCall
    }
    | {
        id: string
        kind: 'final_separator'
        role: 'system'
        timestamp: string
        label: string
    }
    | {
    id: string
    kind: 'spec_edit_proposal' | 'flow_run_request' | 'flow_launch' | 'execution_card'
    role: 'system'
    artifactId: string
    timestamp: string
}
