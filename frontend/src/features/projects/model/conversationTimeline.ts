import type {
    ConversationChatMode,
    ConversationSegmentResponse,
    ConversationTurnResponse,
} from '@/lib/workspaceClient'
import type { ConversationTimelineEntry } from './types'

function formatWorkedDuration(elapsedSeconds: number): string {
    if (elapsedSeconds < 60) {
        return `${elapsedSeconds}s`
    }
    if (elapsedSeconds < 3600) {
        const minutes = Math.floor(elapsedSeconds / 60)
        const seconds = elapsedSeconds % 60
        return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`
    }
    const hours = Math.floor(elapsedSeconds / 3600)
    const minutes = Math.floor((elapsedSeconds % 3600) / 60)
    return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`
}

function resolveWorkedElapsedSeconds(
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
    completedTimestamp: string,
): number | null {
    const completedMs = Date.parse(completedTimestamp)
    if (Number.isNaN(completedMs)) {
        return null
    }
    const candidateTimestamps = [turn.timestamp, ...turnSegments.map((segment) => segment.timestamp)]
        .map((value) => Date.parse(value))
        .filter((value) => !Number.isNaN(value) && value <= completedMs)
    if (candidateTimestamps.length === 0) {
        return null
    }
    const startedMs = Math.min(...candidateTimestamps)
    return Math.max(0, Math.round((completedMs - startedMs) / 1000))
}

function buildAssistantTimelineEntries(
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
): ConversationTimelineEntry[] {
    const entries: ConversationTimelineEntry[] = []
    let hadWorkActivity = false
    let insertedFinalSeparator = false
    const sortedSegments = [...turnSegments].sort((left, right) => left.order - right.order)

    sortedSegments.forEach((segment) => {
        if (!insertedFinalSeparator && hadWorkActivity && (segment.kind === 'assistant_message' || segment.kind === 'plan')) {
            const elapsedSeconds = resolveWorkedElapsedSeconds(turn, turnSegments, segment.timestamp)
            const label = elapsedSeconds === null
                ? 'Worked'
                : `Worked for ${formatWorkedDuration(elapsedSeconds)}`
            entries.push({
                id: `${turn.id}:final-separator:${segment.id}`,
                kind: 'final_separator',
                role: 'system',
                timestamp: segment.timestamp,
                label,
            })
            insertedFinalSeparator = true
        }
        if (segment.kind === 'plan') {
            entries.push({
                id: segment.id,
                kind: 'plan',
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'running' ? 'streaming' : segment.status,
                artifactId: segment.artifact_id ?? null,
                error: segment.error ?? null,
            })
            return
        }
        if (segment.kind === 'assistant_message') {
            entries.push({
                id: segment.id,
                kind: 'message',
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'running' ? 'streaming' : segment.status,
                error: segment.error ?? null,
            })
            return
        }
        if (segment.kind === 'reasoning') {
            entries.push({
                id: segment.id,
                kind: 'message',
                role: 'assistant',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status === 'running' ? 'streaming' : segment.status,
                error: segment.error ?? null,
                presentation: 'thinking',
            })
            return
        }
        if (segment.kind === 'context_compaction') {
            entries.push({
                id: segment.id,
                kind: 'context_compaction',
                role: 'system',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status,
            })
            return
        }
        if (segment.kind === 'request_user_input' && segment.request_user_input) {
            entries.push({
                id: segment.id,
                kind: 'request_user_input',
                role: 'system',
                content: segment.content,
                timestamp: segment.timestamp,
                status: segment.status,
                requestUserInput: {
                    requestId: segment.request_user_input.request_id,
                    status: segment.request_user_input.status,
                    questions: segment.request_user_input.questions.map((question) => ({
                        id: question.id,
                        header: question.header,
                        question: question.question,
                        questionType: question.question_type,
                        options: question.options.map((option) => ({
                            label: option.label,
                            description: option.description ?? null,
                        })),
                        allowOther: question.allow_other,
                        isSecret: question.is_secret,
                    })),
                    answers: segment.request_user_input.answers,
                    submittedAt: segment.request_user_input.submitted_at ?? null,
                },
            })
            return
        }
        if (segment.kind === 'tool_call' && segment.tool_call) {
            entries.push({
                id: segment.id,
                kind: 'tool_call',
                role: 'system',
                timestamp: segment.timestamp,
                toolCall: {
                    id: segment.tool_call.id,
                    kind: segment.tool_call.kind,
                    status: segment.tool_call.status,
                    title: segment.tool_call.title,
                    command: segment.tool_call.command ?? null,
                    output: segment.tool_call.output ?? null,
                    outputSize: segment.tool_call.output_size ?? null,
                    outputTruncated: segment.tool_call.output_truncated === true,
                    filePaths: segment.tool_call.file_paths,
                },
            })
            hadWorkActivity = true
            return
        }
        if (segment.kind === 'flow_run_request' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'flow_run_request',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
        if (segment.kind === 'flow_launch' && segment.artifact_id) {
            entries.push({
                id: segment.id,
                kind: 'flow_launch',
                role: 'system',
                artifactId: segment.artifact_id,
                timestamp: segment.timestamp,
            })
            return
        }
    })

    if (entries.length === 0) {
        const presentation = turn.status === 'complete' || turn.status === 'failed' ? 'default' : 'thinking'
        entries.push({
            id: `${turn.id}:${presentation}:placeholder`,
            kind: 'message',
            role: 'assistant',
            content: turn.content,
            timestamp: turn.timestamp,
            status: turn.status,
            error: turn.error ?? null,
            presentation,
        })
    }

    return entries
}

function buildModeChangeTimelineEntry(turn: ConversationTurnResponse): ConversationTimelineEntry {
    const mode: ConversationChatMode = turn.content === 'plan' ? 'plan' : 'chat'
    return {
        id: turn.id,
        kind: 'mode_change',
        role: 'system',
        timestamp: turn.timestamp,
        mode,
    }
}

export function buildConversationTimelineEntriesForTurn(
    turn: ConversationTurnResponse,
    turnSegments: ConversationSegmentResponse[],
): ConversationTimelineEntry[] {
    if (turn.kind === 'mode_change') {
        return [buildModeChangeTimelineEntry(turn)]
    }
    if (turn.role === 'assistant') {
        return buildAssistantTimelineEntries(turn, turnSegments)
    }
    if (turn.role === 'user') {
        return [{
            id: turn.id,
            kind: 'message',
            role: turn.role,
            content: turn.content,
            timestamp: turn.timestamp,
            status: turn.status,
            error: turn.error ?? null,
        }]
    }
    return []
}
