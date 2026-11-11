import { conversationThinkingKey, LiveThinkingIndicator } from '@/components/app/transcript/liveThinking'
import { memo, useCallback, useMemo, useState, type ReactNode } from 'react'
import type {
    ConversationTimelineEntry,
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
    ProjectProposedPlan,
    ProjectRunRecovery,
} from '../model/types'
import {
    ProjectFlowLaunchEntry,
    ProjectFlowRunRequestEntry,
    ProjectRunRecoveryEntry,
} from './ProjectArtifactEntries'
import { ProjectConversationRequestUserInputCard } from './ProjectConversationRequestUserInputCard'
import {
    getFlowLaunchStatusPresentation,
    getFlowRunRequestStatusPresentation,
    getProposedPlanStatusPresentation,
} from '../model/presentation'
import { Transcript, type TranscriptExpansionState } from '@/components/app/transcript/Transcript'
import { SheetEmpty } from '@/components/app/view-layout'
import { MessageSquare } from 'lucide-react'
import { statusTextClassName } from '@/components/app/status'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ProjectConversationMarkdown } from './ProjectConversationMarkdown'

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
    activeRunRecoveriesById: Map<string, ProjectRunRecovery>
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
    /** The chat was deleted, or is gone after a reset. */
    isConversationUnavailable?: boolean
    /** An older page exists before the loaded messages. */
    hasOlderMessages?: boolean
    isLoadingOlderMessages?: boolean
    olderMessagesError?: string | null
    onLoadOlderMessages?: () => void | Promise<void>
}

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

const RunRecoveryRow = memo(function RunRecoveryRow({
    runRecovery,
    formatConversationTimestamp,
    onOpenFlowRun,
}: {
    runRecovery: ProjectRunRecovery | null
    formatConversationTimestamp: (value: string) => string
    onOpenFlowRun: (request: { run_id?: string | null; flow_name: string }) => void
}) {
    return (
        <li className="flex min-w-0 justify-start">
            <ProjectRunRecoveryEntry
                runRecovery={runRecovery}
                onOpenFlowRun={onOpenFlowRun}
                formatConversationTimestamp={formatConversationTimestamp}
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
    const launchFlowName = planLaunch?.flow_name || 'software-development/implement-change'

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
    activeConversationHistory,
    activeFlowRunRequestsById,
    activeFlowLaunchesById,
    activeProposedPlansById,
    activeRunRecoveriesById,
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
    isConversationUnavailable = false,
    hasOlderMessages = false,
    isLoadingOlderMessages = false,
    olderMessagesError = null,
    onLoadOlderMessages,
}: ProjectConversationHistoryProps) {
    const [planReviewNotes, setPlanReviewNotes] = useState<Record<string, string>>({})
    const onPlanReviewNoteChange = useCallback((proposedPlanId: string, value: string) => {
        setPlanReviewNotes((current) => ({
            ...current,
            [proposedPlanId]: value,
        }))
    }, [])
    const expansion = useMemo<TranscriptExpansionState>(() => ({
        expandedToolCalls,
        expandedThinking: expandedThinkingEntries,
        onToggleToolCall: onToggleToolCallExpanded,
        onToggleThinking: onToggleThinkingEntryExpanded,
    }), [expandedToolCalls, expandedThinkingEntries, onToggleToolCallExpanded, onToggleThinkingEntryExpanded])
    const conversation = useMemo(() => (
        activeConversationId && activeProjectPath ? { id: activeConversationId, projectPath: activeProjectPath } : undefined
    ), [activeConversationId, activeProjectPath])

    // The rows only the chat acts on: plan review, flow requests, launches and run recoveries, and questions.
    const renderEntry = (entry: ConversationTimelineEntry): ReactNode | undefined => {
        switch (entry.kind) {
        case 'request_user_input':
            return (
                <RequestUserInputRow
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
                    flowLaunch={flowLaunch}
                    formatConversationTimestamp={formatConversationTimestamp}
                    isLatestFlowLaunch={flowLaunch?.id === latestFlowLaunchId}
                    onOpenFlowRun={onOpenFlowRun}
                />
            )
        }
        case 'run_recovery':
            return (
                <RunRecoveryRow
                    runRecovery={activeRunRecoveriesById.get(entry.artifactId) || null}
                    formatConversationTimestamp={formatConversationTimestamp}
                    onOpenFlowRun={onOpenFlowRun}
                />
            )
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
        default:
            return undefined
        }
    }

    if (!activeConversationId && !isConversationUnavailable && !isConversationHistoryLoading && activeConversationHistory.length === 0) {
        return (
            <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-1 flex-col">
                <SheetEmpty icon={MessageSquare} title="No chat open">Create or select a thread to begin chatting.</SheetEmpty>
            </div>
        )
    }

    return (
        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col gap-3">
            <OlderMessagesControl
                hasOlderMessages={hasOlderMessages}
                isLoading={isLoadingOlderMessages}
                error={olderMessagesError}
                onLoad={onLoadOlderMessages}
            />
            <Transcript
                data-testid="project-ai-conversation-history-list"
                testIdPrefix="project"
                entries={activeConversationHistory}
                showThinking={showThinking}
                formatTimestamp={formatConversationTimestamp}
                loading={isConversationHistoryLoading}
                empty={isConversationUnavailable
                    ? 'This chat is no longer available.'
                    : undefined}
                conversation={conversation}
                expansion={expansion}
                attachmentUrl={attachmentUrl}
                onRetryTurn={onRetryTurn}
                renderEntry={renderEntry}
            />
            {activeConversationId ? (
                <LiveThinkingIndicator thinkingKey={conversationThinkingKey(activeConversationId)} testId="project-live-thinking" />
            ) : null}
        </div>
    )
}

/** Loads the page before the oldest loaded message: a button while there is one, busy while
 * loading, and an alert with Retry when loading failed. */
export function OlderMessagesControl({ hasOlderMessages, isLoading, error, onLoad, testId = 'project-load-older-messages' }: {
    hasOlderMessages: boolean
    isLoading: boolean
    error: string | null
    onLoad?: () => void | Promise<void>
    testId?: string
}) {
    if (error) {
        return (
            <div role="alert" className="flex flex-wrap items-center justify-center gap-2 text-sm text-destructive">
                <span>{error}</span>
                <Button type="button" variant="outline" size="xs" onClick={() => { void onLoad?.() }}>
                    Retry
                </Button>
            </div>
        )
    }
    if (!hasOlderMessages) return null
    return (
        <div className="flex justify-center">
            <Button
                type="button"
                data-testid={testId}
                variant="ghost"
                size="xs"
                disabled={isLoading}
                aria-busy={isLoading}
                onClick={() => { void onLoad?.() }}
            >
                {isLoading ? 'Loading older messages…' : 'Load older messages'}
            </Button>
        </div>
    )
}
