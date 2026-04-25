import type {
    ConversationChatMode,
    ConversationSegmentResponse,
    ConversationSnapshotResponse,
    ConversationTurnResponse,
} from '@/lib/workspaceClient'
import type { ConversationTimelineEntry } from './types'
import type { OptimisticSendState } from './conversationState'

export interface ConversationTimelineStabilizationScope {
    conversationId: string | null
    entries: ConversationTimelineEntry[]
}

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
                id: `${turn.id}:final-separator:${entries.length}`,
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

export function buildConversationTimelineEntries(
    snapshot: ConversationSnapshotResponse | null,
    optimisticSend: OptimisticSendState | null,
): ConversationTimelineEntry[] {
    if (!snapshot) {
        if (!optimisticSend) {
            return []
        }
        return [
            {
                id: `${optimisticSend.conversationId}:optimistic:user`,
                kind: 'message',
                role: 'user',
                content: optimisticSend.message,
                timestamp: optimisticSend.createdAt,
                status: 'complete',
            },
        ]
    }

    const timeline: ConversationTimelineEntry[] = []
    const segmentsByTurn = new Map<string, ConversationSegmentResponse[]>()
    snapshot.segments.forEach((segment) => {
        const entries = segmentsByTurn.get(segment.turn_id) || []
        entries.push(segment)
        segmentsByTurn.set(segment.turn_id, entries)
    })
    snapshot.turns.forEach((turn) => {
        if (turn.kind === 'mode_change') {
            timeline.push(buildModeChangeTimelineEntry(turn))
            return
        }
        if (turn.role === 'user' || turn.role === 'assistant') {
            if (turn.role === 'assistant') {
                timeline.push(...buildAssistantTimelineEntries(turn, segmentsByTurn.get(turn.id) || []))
                return
            }
            timeline.push({
                id: turn.id,
                kind: 'message',
                role: turn.role,
                content: turn.content,
                timestamp: turn.timestamp,
                status: turn.status,
                error: turn.error ?? null,
            })
        }
    })

    if (!optimisticSend) {
        return timeline
    }

    return [
        ...timeline,
        {
            id: `${optimisticSend.conversationId}:optimistic:user`,
            kind: 'message',
            role: 'user',
            content: optimisticSend.message,
            timestamp: optimisticSend.createdAt,
            status: 'complete',
        },
    ]
}

function optionalStringEqual(left?: string | null, right?: string | null) {
    return (left ?? null) === (right ?? null)
}

function stringRecordEqual(left: Record<string, string>, right: Record<string, string>) {
    const leftKeys = Object.keys(left)
    const rightKeys = Object.keys(right)
    return leftKeys.length === rightKeys.length
        && leftKeys.every((key) => left[key] === right[key])
}

function stringArrayEqual(left: string[], right: string[]) {
    return left.length === right.length && left.every((value, index) => value === right[index])
}

function requestUserInputEqual(
    left: Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>['requestUserInput'],
    right: Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>['requestUserInput'],
) {
    return left.requestId === right.requestId
        && left.status === right.status
        && optionalStringEqual(left.submittedAt, right.submittedAt)
        && stringRecordEqual(left.answers, right.answers)
        && left.questions.length === right.questions.length
        && left.questions.every((leftQuestion, questionIndex) => {
            const rightQuestion = right.questions[questionIndex]
            return rightQuestion
                && leftQuestion.id === rightQuestion.id
                && leftQuestion.header === rightQuestion.header
                && leftQuestion.question === rightQuestion.question
                && leftQuestion.questionType === rightQuestion.questionType
                && leftQuestion.allowOther === rightQuestion.allowOther
                && leftQuestion.isSecret === rightQuestion.isSecret
                && leftQuestion.options.length === rightQuestion.options.length
                && leftQuestion.options.every((leftOption, optionIndex) => {
                    const rightOption = rightQuestion.options[optionIndex]
                    return rightOption
                        && leftOption.label === rightOption.label
                        && optionalStringEqual(leftOption.description, rightOption.description)
                })
        })
}

function conversationTimelineEntryKey(entry: ConversationTimelineEntry) {
    return `${entry.kind}:${entry.id}`
}

function conversationTimelineEntryRenderedFieldsEqual(
    left: ConversationTimelineEntry,
    right: ConversationTimelineEntry,
) {
    if (left.kind !== right.kind || left.role !== right.role || left.id !== right.id || left.timestamp !== right.timestamp) {
        return false
    }

    switch (left.kind) {
    case 'message': {
        const rightMessage = right as Extract<ConversationTimelineEntry, { kind: 'message' }>
        return left.content === rightMessage.content
            && left.status === rightMessage.status
            && optionalStringEqual(left.error, rightMessage.error)
            && (left.presentation ?? 'default') === (rightMessage.presentation ?? 'default')
    }
    case 'plan': {
        const rightPlan = right as Extract<ConversationTimelineEntry, { kind: 'plan' }>
        return left.content === rightPlan.content
            && left.status === rightPlan.status
            && optionalStringEqual(left.artifactId, rightPlan.artifactId)
            && optionalStringEqual(left.error, rightPlan.error)
    }
    case 'mode_change': {
        const rightModeChange = right as Extract<ConversationTimelineEntry, { kind: 'mode_change' }>
        return left.mode === rightModeChange.mode
    }
    case 'context_compaction': {
        const rightContextCompaction = right as Extract<ConversationTimelineEntry, { kind: 'context_compaction' }>
        return left.content === rightContextCompaction.content
            && left.status === rightContextCompaction.status
    }
    case 'request_user_input': {
        const rightRequestUserInput = right as Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>
        return left.content === rightRequestUserInput.content
            && left.status === rightRequestUserInput.status
            && requestUserInputEqual(left.requestUserInput, rightRequestUserInput.requestUserInput)
    }
    case 'tool_call': {
        const rightToolCall = right as Extract<ConversationTimelineEntry, { kind: 'tool_call' }>
        return left.toolCall.id === rightToolCall.toolCall.id
            && left.toolCall.kind === rightToolCall.toolCall.kind
            && left.toolCall.status === rightToolCall.toolCall.status
            && left.toolCall.title === rightToolCall.toolCall.title
            && optionalStringEqual(left.toolCall.command, rightToolCall.toolCall.command)
            && optionalStringEqual(left.toolCall.output, rightToolCall.toolCall.output)
            && stringArrayEqual(left.toolCall.filePaths, rightToolCall.toolCall.filePaths)
    }
    case 'final_separator': {
        const rightFinalSeparator = right as Extract<ConversationTimelineEntry, { kind: 'final_separator' }>
        return left.label === rightFinalSeparator.label
    }
    case 'flow_run_request':
    case 'flow_launch': {
        const rightArtifact = right as Extract<ConversationTimelineEntry, { kind: 'flow_run_request' | 'flow_launch' }>
        return left.artifactId === rightArtifact.artifactId
    }
    }
}

export function stabilizeConversationTimelineEntries(
    conversationId: string | null,
    entries: ConversationTimelineEntry[],
    previousScope: ConversationTimelineStabilizationScope | null,
): ConversationTimelineEntry[] {
    if (!conversationId || previousScope?.conversationId !== conversationId || previousScope.entries.length === 0) {
        return entries
    }

    const previousEntriesByKey = new Map(
        previousScope.entries.map((entry) => [conversationTimelineEntryKey(entry), entry]),
    )
    let reusedAnyEntry = false
    const stabilizedEntries = entries.map((entry) => {
        const previousEntry = previousEntriesByKey.get(conversationTimelineEntryKey(entry))
        if (!previousEntry || !conversationTimelineEntryRenderedFieldsEqual(previousEntry, entry)) {
            return entry
        }
        reusedAnyEntry = true
        return previousEntry
    })

    return reusedAnyEntry ? stabilizedEntries : entries
}
