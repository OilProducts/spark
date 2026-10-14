import { memo, useCallback, useState, type ReactNode } from 'react'
import { loadConversationSegmentToolOutput } from '../services/conversationArtifacts'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
    ProjectProposedPlan,
} from '../model/types'
import {
    ProjectFlowLaunchEntry,
    ProjectFlowRunRequestEntry,
} from './ProjectArtifactEntries'
import { ProjectConversationRequestUserInputCard } from './ProjectConversationRequestUserInputCard'
import {
    getFlowLaunchStatusPresentation,
    getFlowRunRequestStatusPresentation,
    getProposedPlanStatusPresentation,
} from '../model/presentation'
import {
    AssistantTurnHeader,
    EventLine,
    MessageRow,
    ThinkingRow,
    ToolCallGroupRow,
    ToolCallRow,
} from '@/components/app/transcript/SegmentRows'
import { groupTranscript } from '@/components/app/transcript/groupTranscript'
import { statusTextClassName } from '@/components/app/status'
import { Empty, EmptyDescription, EmptyHeader } from '@/components/ui/empty'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { formatRunDate } from '@/features/runs/model/runOverviewModel'
import { isThinkingEntry } from '../model/conversationTimeline'
import { ProjectConversationMarkdown } from './ProjectConversationMarkdown'

type MessageEntry = Extract<ConversationTimelineEntry, { kind: 'message' }>
type ToolCallEntry = Extract<ConversationTimelineEntry, { kind: 'tool_call' }>
type RequestUserInputEntry = Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>
type PlanEntry = Extract<ConversationTimelineEntry, { kind: 'plan' }>

interface ProjectConversationHistoryProps {
    activeConversationId: string | null
    activeProjectPath: string | null
    /** Where sent attachments download from. */
    attachmentUrl?: (attachmentId: string) => string
    isConversationHistoryLoading: boolean
    hasRenderableConversationHistory: boolean
    activeConversationHistory: ConversationTimelineEntry[]
    activeFlowRunRequestsById: Map<string, ProjectFlowRunRequest>
    activeFlowLaunchesById: Map<string, ProjectFlowLaunch>
    activeProposedPlansById: Map<string, ProjectProposedPlan>
    latestFlowRunRequestId: string | null
    latestFlowLaunchId: string | null
    expandedToolCalls: Record<string, boolean>
    expandedThinkingEntries: Record<string, boolean>
    /** Owned by the sheet header's "Show thinking" toggle. */
    showThinking: boolean
    pendingFlowRunRequestId: string | null
    pendingProposedPlanId: string | null
    requestUserInputActionError: string | null
    submittingRequestUserInputIds: Record<string, boolean>
    formatConversationTimestamp: (value: string) => string
    onSubmitRequestUserInput: (requestId: string, answers: Record<string, string>) => void | Promise<void>
    onToggleToolCallExpanded: (toolCallId: string) => void
    onToggleThinkingEntryExpanded: (entryId: string) => void
    onReviewFlowRunRequest: (
        flowRunRequest: ProjectFlowRunRequest,
        disposition: 'approved' | 'rejected',
    ) => void | Promise<void>
    onReviewProposedPlan: (
        proposedPlan: ProjectProposedPlan,
        disposition: 'approved' | 'rejected',
        reviewNote?: string | null,
    ) => void | Promise<void>
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    /** Resends a failed turn's message. */
    onRetryTurn?: (turnId: string) => void | Promise<void>
}

function conversationEntryKey(entry: ConversationTimelineEntry) {
    return `${entry.kind}:${entry.id}:${entry.timestamp}`
}

const isToolCallEntry = (entry: ConversationTimelineEntry): entry is ToolCallEntry => entry.kind === 'tool_call'

type TranscriptTurn =
    | { kind: 'entry'; entry: ConversationTimelineEntry }
    | { kind: 'assistant'; id: string; timestamp: string; worked: string | null; entries: ConversationTimelineEntry[] }

/**
 * Splits the flat timeline into user messages, standalone events (mode changes, notices)
 * and assistant turns. The "Worked for 20s" separator the timeline inserts before a turn's
 * final answer becomes the turn header's meta instead of a row of its own.
 */
function groupConversationTurns(entries: readonly ConversationTimelineEntry[]): TranscriptTurn[] {
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

/** The live "Thinking" marker shown while thinking is hidden and the agent is still at it. */
const ThinkingIndicatorRow = memo(function ThinkingIndicatorRow({
    entry,
    formatConversationTimestamp,
}: {
    entry: MessageEntry
    formatConversationTimestamp: (value: string) => string
}) {
    return (
        <li className="min-w-0 text-xs text-muted-foreground" title={formatConversationTimestamp(entry.timestamp)}>
            <p className="italic">Thinking</p>
        </li>
    )
})

const RequestUserInputRow = memo(function RequestUserInputRow({
    actionError,
    entry,
    formatConversationTimestamp,
    isSubmitting,
    onSubmitRequestUserInput,
}: {
    actionError: string | null
    entry: RequestUserInputEntry
    formatConversationTimestamp: (value: string) => string
    isSubmitting: boolean
    onSubmitRequestUserInput: (requestId: string, answers: Record<string, string>) => void | Promise<void>
}) {
    return (
        <li className="flex min-w-0 justify-start">
            <ProjectConversationRequestUserInputCard
                actionError={actionError}
                entry={entry}
                formatConversationTimestamp={formatConversationTimestamp}
                isSubmitting={isSubmitting}
                onSubmitRequestUserInput={onSubmitRequestUserInput}
            />
        </li>
    )
})

const FlowRunRequestRow = memo(function FlowRunRequestRow({
    flowRunRequest,
    formatConversationTimestamp,
    isLatestFlowRunRequest,
    onOpenFlowRun,
    onReviewFlowRunRequest,
    pendingFlowRunRequestId,
}: {
    flowRunRequest: ProjectFlowRunRequest | null
    formatConversationTimestamp: (value: string) => string
    isLatestFlowRunRequest: boolean
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    onReviewFlowRunRequest: (flowRunRequest: ProjectFlowRunRequest, disposition: 'approved' | 'rejected') => void | Promise<void>
    pendingFlowRunRequestId: string | null
}) {
    return (
        <li
            data-testid={isLatestFlowRunRequest ? 'project-flow-run-request-history-row' : undefined}
            className="flex min-w-0 justify-start"
        >
            <ProjectFlowRunRequestEntry
                flowRunRequest={flowRunRequest}
                isLatestFlowRunRequest={isLatestFlowRunRequest}
                pendingFlowRunRequestId={pendingFlowRunRequestId}
                onReviewFlowRunRequest={onReviewFlowRunRequest}
                onOpenFlowRun={onOpenFlowRun}
                formatConversationTimestamp={formatConversationTimestamp}
                getFlowRunRequestStatusPresentation={getFlowRunRequestStatusPresentation}
            />
        </li>
    )
})

const FlowLaunchRow = memo(function FlowLaunchRow({
    flowLaunch,
    formatConversationTimestamp,
    isLatestFlowLaunch,
    onOpenFlowRun,
}: {
    flowLaunch: ProjectFlowLaunch | null
    formatConversationTimestamp: (value: string) => string
    isLatestFlowLaunch: boolean
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
}) {
    return (
        <li
            data-testid={isLatestFlowLaunch ? 'project-flow-launch-history-row' : undefined}
            className="flex min-w-0 justify-start"
        >
            <ProjectFlowLaunchEntry
                flowLaunch={flowLaunch}
                isLatestFlowLaunch={isLatestFlowLaunch}
                onOpenFlowRun={onOpenFlowRun}
                formatConversationTimestamp={formatConversationTimestamp}
                getFlowLaunchStatusPresentation={getFlowLaunchStatusPresentation}
            />
        </li>
    )
})

const PlanRow = memo(function PlanRow({
    entry,
    formatConversationTimestamp,
    onOpenFlowRun,
    onPlanReviewNoteChange,
    onReviewProposedPlan,
    pendingProposedPlanId,
    planLaunch,
    proposedPlan,
    reviewNoteValue,
}: {
    entry: PlanEntry
    formatConversationTimestamp: (value: string) => string
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
    onPlanReviewNoteChange: (proposedPlanId: string, value: string) => void
    onReviewProposedPlan: (
        proposedPlan: ProjectProposedPlan,
        disposition: 'approved' | 'rejected',
        reviewNote?: string | null,
    ) => void | Promise<void>
    pendingProposedPlanId: string | null
    planLaunch: ProjectFlowLaunch | null
    proposedPlan: ProjectProposedPlan | null
    reviewNoteValue: string
}) {
    const content = entry.status === 'failed' && !entry.content.trim()
        ? (entry.error || 'Plan generation failed.')
        : entry.content
    const statusPresentation = proposedPlan
        ? getProposedPlanStatusPresentation(proposedPlan.status)
        : null
    const launchRunId = proposedPlan?.run_id ?? planLaunch?.run_id ?? null
    const launchFlowName = planLaunch?.flow_name || 'software-development/implement-change.yaml'

    return (
        <li className="flex min-w-0 justify-start">
            <div
                data-testid={`project-plan-card-${entry.id}`}
                className="min-w-0 max-w-[72ch] rounded-xl border border-border px-4 py-3 text-foreground"
            >
                <div className="flex flex-wrap items-center gap-2">
                    <p className="text-sm font-semibold text-foreground">Proposed plan</p>
                    {statusPresentation ? (
                        <span className={`text-xs font-medium ${statusTextClassName(statusPresentation.tone)}`}>
                            {statusPresentation.label}
                        </span>
                    ) : null}
                </div>
                <div className="mt-2">
                    {entry.status === 'failed' ? (
                        <p className="whitespace-pre-wrap break-words text-sm leading-6 [overflow-wrap:anywhere]">{content}</p>
                    ) : (
                        <ProjectConversationMarkdown content={content} enableCodeCopy />
                    )}
                </div>
                {proposedPlan ? (
                    <div className="mt-2 space-y-2 text-xs text-muted-foreground">
                        {proposedPlan.review_note ? (
                            <p>
                                Review note: <span className="text-foreground">{proposedPlan.review_note}</span>
                            </p>
                        ) : null}
                        {proposedPlan.written_change_request_path ? (
                            <p className="break-all font-mono text-xs text-muted-foreground">
                                {proposedPlan.written_change_request_path}
                            </p>
                        ) : null}
                        {proposedPlan.launch_error ? (
                            <p className="text-destructive">
                                Launch error: {proposedPlan.launch_error}
                            </p>
                        ) : null}
                        {launchRunId ? (
                            <div className="flex flex-wrap items-center gap-2">
                                <span>
                                    Run: <span className="font-mono text-foreground">{launchRunId}</span>
                                </span>
                                <Button
                                    type="button"
                                    data-testid={`project-proposed-plan-open-run-button-${proposedPlan.id}`}
                                    onClick={() => onOpenFlowRun({ run_id: launchRunId, flow_name: launchFlowName })}
                                    variant="ghost"
                                    size="xs"
                                >
                                    Open run
                                </Button>
                            </div>
                        ) : null}
                        {proposedPlan.status === 'pending_review' ? (
                            <div className="space-y-2">
                                <Input
                                    data-testid={`project-proposed-plan-review-note-${proposedPlan.id}`}
                                    value={reviewNoteValue}
                                    onChange={(event) => onPlanReviewNoteChange(proposedPlan.id, event.target.value)}
                                    placeholder="Optional review note"
                                    className="h-8 bg-background text-sm"
                                />
                                <div className="flex flex-wrap items-center gap-2">
                                    <Button
                                        type="button"
                                        data-testid={`project-proposed-plan-approve-button-${proposedPlan.id}`}
                                        onClick={() => {
                                            void onReviewProposedPlan(proposedPlan, 'approved', reviewNoteValue)
                                        }}
                                        disabled={pendingProposedPlanId === proposedPlan.id}
                                        variant="outline"
                                        size="xs"
                                    >
                                        Approve
                                    </Button>
                                    <Button
                                        type="button"
                                        data-testid={`project-proposed-plan-reject-button-${proposedPlan.id}`}
                                        onClick={() => {
                                            void onReviewProposedPlan(proposedPlan, 'rejected', reviewNoteValue)
                                        }}
                                        disabled={pendingProposedPlanId === proposedPlan.id}
                                        variant="ghost"
                                        size="xs"
                                    >
                                        Disapprove
                                    </Button>
                                </div>
                            </div>
                        ) : null}
                    </div>
                ) : null}
                <p className="mt-1 text-xs text-muted-foreground">
                    {formatConversationTimestamp(entry.timestamp)}
                </p>
            </div>
        </li>
    )
})

export function ProjectConversationHistory({
    activeConversationId,
    activeProjectPath,
    attachmentUrl,
    isConversationHistoryLoading,
    hasRenderableConversationHistory,
    activeConversationHistory,
    activeFlowRunRequestsById,
    activeFlowLaunchesById,
    activeProposedPlansById,
    latestFlowRunRequestId,
    latestFlowLaunchId,
    expandedToolCalls,
    expandedThinkingEntries,
    showThinking,
    pendingFlowRunRequestId,
    pendingProposedPlanId,
    requestUserInputActionError,
    submittingRequestUserInputIds,
    formatConversationTimestamp,
    onSubmitRequestUserInput,
    onToggleToolCallExpanded,
    onToggleThinkingEntryExpanded,
    onReviewFlowRunRequest,
    onReviewProposedPlan,
    onOpenFlowRun,
    onRetryTurn,
}: ProjectConversationHistoryProps) {
    const [planReviewNotes, setPlanReviewNotes] = useState<Record<string, string>>({})
    const [fullToolOutputs, setFullToolOutputs] = useState<Record<string, string>>({})
    const [loadingFullToolOutputs, setLoadingFullToolOutputs] = useState<Record<string, boolean>>({})
    const [fullToolOutputErrors, setFullToolOutputErrors] = useState<Record<string, string>>({})
    const [openToolGroups, setOpenToolGroups] = useState<Record<string, boolean>>({})
    // eslint-disable-next-line react-hooks/purity -- render-time clock for turn dates; the history re-renders on every stream update
    const now = Date.now()
    const onPlanReviewNoteChange = useCallback((proposedPlanId: string, value: string) => {
        setPlanReviewNotes((current) => ({
            ...current,
            [proposedPlanId]: value,
        }))
    }, [])
    const onToolCallExpanded = useCallback((entry: ToolCallEntry) => {
        onToggleToolCallExpanded(entry.toolCall.id)
        if (
            !activeConversationId
            || !activeProjectPath
            || entry.toolCall.outputTruncated !== true
            || expandedToolCalls[entry.toolCall.id] === true
            || fullToolOutputs[entry.id] !== undefined
            || loadingFullToolOutputs[entry.id] === true
        ) {
            return
        }
        setLoadingFullToolOutputs((current) => ({
            ...current,
            [entry.id]: true,
        }))
        setFullToolOutputErrors((current) => {
            const next = { ...current }
            delete next[entry.id]
            return next
        })
        void loadConversationSegmentToolOutput(activeConversationId, entry.id, activeProjectPath)
            .then((payload) => {
                setFullToolOutputs((current) => ({
                    ...current,
                    [entry.id]: payload.output,
                }))
            })
            .catch(() => {
                setFullToolOutputErrors((current) => ({
                    ...current,
                    [entry.id]: 'Unable to load full output.',
                }))
            })
            .finally(() => {
                setLoadingFullToolOutputs((current) => {
                    const next = { ...current }
                    delete next[entry.id]
                    return next
                })
            })
    }, [
        activeConversationId,
        activeProjectPath,
        expandedToolCalls,
        fullToolOutputs,
        loadingFullToolOutputs,
        onToggleToolCallExpanded,
    ])

    const renderToolCall = (entry: ToolCallEntry, variant: 'row' | 'line' = 'row') => (
        <ToolCallRow
            key={conversationEntryKey(entry)}
            entry={entry}
            variant={variant}
            fullOutput={fullToolOutputs[entry.id] ?? null}
            isLoadingFullOutput={loadingFullToolOutputs[entry.id] === true}
            isExpanded={expandedToolCalls[entry.toolCall.id] === true}
            loadFullOutputError={fullToolOutputErrors[entry.id] ?? null}
            onToggleToolCallExpanded={() => onToolCallExpanded(entry)}
        />
    )

    // Thinking is hidden until the toggle reveals it; a thought still in progress (or a
    // turn that has produced nothing yet) keeps a bare "Thinking" marker so the agent
    // never looks idle.
    const renderThinking = (entry: MessageEntry) => {
        const key = conversationEntryKey(entry)
        if (showThinking) {
            return (
                <ThinkingRow
                    key={key}
                    entry={entry}
                    formatConversationTimestamp={formatConversationTimestamp}
                    isExpanded={expandedThinkingEntries[entry.id] === true}
                    onToggleThinkingEntryExpanded={onToggleThinkingEntryExpanded}
                />
            )
        }
        const isLive = entry.status !== 'complete' && entry.status !== 'failed'
        return isLive || !entry.content.trim()
            ? <ThinkingIndicatorRow key={key} entry={entry} formatConversationTimestamp={formatConversationTimestamp} />
            : null
    }

    const renderEntryRow = (entry: ConversationTimelineEntry): ReactNode => {
        const key = conversationEntryKey(entry)
        switch (entry.kind) {
        case 'tool_call':
            return renderToolCall(entry)
        case 'final_separator':
            return <li key={key}><EventLine data-testid={`project-notice-row-${entry.id}`}>{entry.label}</EventLine></li>
        case 'mode_change':
            return (
                <li key={key}>
                    <EventLine data-testid={`project-mode-change-row-${entry.id}`}>
                        {entry.mode === 'plan' ? 'Switched to Plan mode' : 'Switched to Chat mode'}
                    </EventLine>
                </li>
            )
        case 'context_compaction':
            return <li key={key}><EventLine data-testid={`project-context-compaction-row-${entry.id}`}>{entry.content}</EventLine></li>
        case 'request_user_input':
            return (
                <RequestUserInputRow
                    key={key}
                    actionError={requestUserInputActionError}
                    entry={entry}
                    formatConversationTimestamp={formatConversationTimestamp}
                    isSubmitting={submittingRequestUserInputIds[entry.requestUserInput.requestId] === true}
                    onSubmitRequestUserInput={onSubmitRequestUserInput}
                />
            )
        case 'flow_run_request': {
            const flowRunRequest = activeFlowRunRequestsById.get(entry.artifactId) || null
            return (
                <FlowRunRequestRow
                    key={key}
                    flowRunRequest={flowRunRequest}
                    formatConversationTimestamp={formatConversationTimestamp}
                    isLatestFlowRunRequest={flowRunRequest?.id === latestFlowRunRequestId}
                    onOpenFlowRun={onOpenFlowRun}
                    onReviewFlowRunRequest={onReviewFlowRunRequest}
                    pendingFlowRunRequestId={pendingFlowRunRequestId}
                />
            )
        }
        case 'flow_launch': {
            const flowLaunch = activeFlowLaunchesById.get(entry.artifactId) || null
            return (
                <FlowLaunchRow
                    key={key}
                    flowLaunch={flowLaunch}
                    formatConversationTimestamp={formatConversationTimestamp}
                    isLatestFlowLaunch={flowLaunch?.id === latestFlowLaunchId}
                    onOpenFlowRun={onOpenFlowRun}
                />
            )
        }
        case 'plan': {
            const proposedPlan = entry.artifactId
                ? (activeProposedPlansById.get(entry.artifactId) || null)
                : null
            const planLaunch = proposedPlan?.flow_launch_id
                ? (activeFlowLaunchesById.get(proposedPlan.flow_launch_id) || null)
                : null
            const reviewNoteValue = proposedPlan
                ? (planReviewNotes[proposedPlan.id] ?? proposedPlan.review_note ?? '')
                : ''
            return (
                <PlanRow
                    key={key}
                    entry={entry}
                    formatConversationTimestamp={formatConversationTimestamp}
                    onOpenFlowRun={onOpenFlowRun}
                    onPlanReviewNoteChange={onPlanReviewNoteChange}
                    onReviewProposedPlan={onReviewProposedPlan}
                    pendingProposedPlanId={pendingProposedPlanId}
                    planLaunch={planLaunch}
                    proposedPlan={proposedPlan}
                    reviewNoteValue={reviewNoteValue}
                />
            )
        }
        case 'message':
            if (isThinkingEntry(entry)) return renderThinking(entry)
            return (
                <MessageRow
                    key={key}
                    entry={entry}
                    enableCopy
                    formatConversationTimestamp={formatConversationTimestamp}
                    attachmentUrl={attachmentUrl}
                    onRetryTurn={onRetryTurn && ((turnId) => void onRetryTurn(turnId))}
                />
            )
        }
    }

    /** A turn's rows: consecutive tool calls fold into one group; a lone call stays a plain row. */
    const renderTurnBody = (entries: ConversationTimelineEntry[]) => {
        const blocks = groupTranscript(entries, (entry) => {
            if (entry.kind === 'tool_call') return { kind: 'tool_call', toolCall: entry.toolCall }
            return isThinkingEntry(entry) ? { kind: 'thinking' } : { kind: 'other' }
        })
        return blocks.map((block) => {
            if (block.kind === 'item') return renderEntryRow(block.item)
            const [firstMember] = block.members
            if (block.members.length === 1 && isToolCallEntry(firstMember.item)) {
                return renderToolCall(firstMember.item)
            }
            return (
                <ToolCallGroupRow
                    key={block.id}
                    summary={block.summary}
                    isExpanded={openToolGroups[block.id] === true}
                    onToggle={() => setOpenToolGroups((current) => ({ ...current, [block.id]: !current[block.id] }))}
                    testId={`project-tool-group-${firstMember.kind === 'tool_call' ? firstMember.toolCall.id : firstMember.item.id}`}
                >
                    {block.members.map(({ item }) => isToolCallEntry(item)
                        ? renderToolCall(item, 'line')
                        : isThinkingEntry(item) ? renderThinking(item) : null)}
                </ToolCallGroupRow>
            )
        })
    }

    return (
        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col gap-3">
            {isConversationHistoryLoading && !hasRenderableConversationHistory ? (
                <p data-testid="project-conversation-history-loading" className="text-xs text-muted-foreground" aria-live="polite">
                    Restoring thread history...
                </p>
            ) : !hasRenderableConversationHistory ? (
                <Empty>
                    <EmptyHeader>
                        <EmptyDescription>
                            {activeConversationId
                                ? 'No conversation history for this thread yet.'
                                : 'Create or select a thread to begin chatting.'}
                        </EmptyDescription>
                    </EmptyHeader>
                </Empty>
            ) : (
                <ol data-testid="project-ai-conversation-history-list" className="flex min-w-0 flex-col gap-5">
                    {groupConversationTurns(activeConversationHistory).map((turn) => {
                        if (turn.kind === 'entry') return renderEntryRow(turn.entry)
                        return (
                            <li key={`turn:${turn.id}`} data-testid={`project-assistant-turn-${turn.id}`} className="flex min-w-0 flex-col gap-3">
                                <AssistantTurnHeader
                                    meta={[formatRunDate(turn.timestamp, now), turn.worked].filter(Boolean).join(' · ')}
                                    title={formatConversationTimestamp(turn.timestamp)}
                                />
                                <ol className="flex min-w-0 flex-col gap-3 pl-8">{renderTurnBody(turn.entries)}</ol>
                            </li>
                        )
                    })}
                </ol>
            )}
        </div>
    )
}
