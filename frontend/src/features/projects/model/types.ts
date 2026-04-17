import type {
    ConversationSegmentResponse,
    ConversationSummaryResponse,
    ConversationTurnResponse,
    FlowLaunchResponse,
    FlowRunRequestResponse,
    ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'

export type ProjectConversationSummary = ConversationSummaryResponse
export type ProjectFlowLaunch = FlowLaunchResponse
export type ProjectFlowRunRequest = FlowRunRequestResponse
export type ProjectProposedPlan = ProposedPlanArtifactResponse
export type ConversationTurnStatus = ConversationTurnResponse['status']
export type ConversationSegmentStatus = ConversationSegmentResponse['status']

export interface ConversationTimelineToolCall {
    id: string
    kind: 'command_execution' | 'file_change' | 'dynamic_tool'
    status: 'running' | 'completed' | 'failed'
    title: string
    command?: string | null
    output?: string | null
    filePaths: string[]
}

export interface ConversationTimelineRequestUserInputQuestion {
    id: string
    header: string
    question: string
    questionType: 'MULTIPLE_CHOICE' | 'FREEFORM'
    options: Array<{
        label: string
        description?: string | null
    }>
    allowOther: boolean
    isSecret: boolean
}

export interface ConversationTimelineRequestUserInput {
    requestId: string
    status: 'pending' | 'answered' | 'expired'
    questions: ConversationTimelineRequestUserInputQuestion[]
    answers: Record<string, string>
    submittedAt?: string | null
}

export type ConversationTimelineEntry =
    | {
        id: string
        kind: 'message'
        role: 'user' | 'assistant'
        content: string
        timestamp: string
        status: ConversationTurnStatus
        error?: string | null
        presentation?: 'default' | 'thinking'
    }
    | {
        id: string
        kind: 'plan'
        role: 'assistant'
        content: string
        timestamp: string
        status: ConversationTurnStatus
        artifactId?: string | null
        error?: string | null
    }
    | {
        id: string
        kind: 'mode_change'
        role: 'system'
        timestamp: string
        mode: 'chat' | 'plan'
    }
    | {
        id: string
        kind: 'context_compaction'
        role: 'system'
        timestamp: string
        content: string
        status: ConversationSegmentStatus
    }
    | {
        id: string
        kind: 'request_user_input'
        role: 'system'
        timestamp: string
        content: string
        status: ConversationSegmentStatus
        requestUserInput: ConversationTimelineRequestUserInput
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
    kind: 'flow_run_request' | 'flow_launch'
    role: 'system'
    artifactId: string
    timestamp: string
}
