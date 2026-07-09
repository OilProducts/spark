import { memo, useCallback, useState } from 'react'
import { fetchConversationSegmentToolOutputValidated } from '@/lib/workspaceClient'
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
    getSurfaceToneClassName,
} from '../model/presentation'
import { MessageRow, ThinkingRow, ToolCallRow } from '@/components/app/transcript/SegmentRows'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ProjectConversationMarkdown } from './ProjectConversationMarkdown'

type ToolCallEntry = Extract<ConversationTimelineEntry, { kind: 'tool_call' }>
type RequestUserInputEntry = Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>
type PlanEntry = Extract<ConversationTimelineEntry, { kind: 'plan' }>
type FinalSeparatorEntry = Extract<ConversationTimelineEntry, { kind: 'final_separator' }>
type ModeChangeEntry = Extract<ConversationTimelineEntry, { kind: 'mode_change' }>
type ContextCompactionEntry = Extract<ConversationTimelineEntry, { kind: 'context_compaction' }>

interface ProjectConversationHistoryProps {
    activeConversationId: string | null
    activeProjectPath: string | null
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
}

function conversationEntryKey(entry: ConversationTimelineEntry) {
    return `${entry.kind}:${entry.id}:${entry.timestamp}`
}

const FinalSeparatorRow = memo(function FinalSeparatorRow({ entry }: { entry: FinalSeparatorEntry }) {
    return (
        <li className="flex justify-center">
            <div className="flex w-full max-w-[85%] items-center gap-3 py-1 text-[11px] text-muted-foreground">
                <span className="h-px flex-1 bg-border" />
                <span className="shrink-0 whitespace-nowrap">{entry.label}</span>
                <span className="h-px flex-1 bg-border" />
            </div>
        </li>
    )
})

const ModeChangeRow = memo(function ModeChangeRow({ entry }: { entry: ModeChangeEntry }) {
    return (
        <li className="flex justify-center">
            <div
                data-testid={`project-mode-change-row-${entry.id}`}
                className="flex w-full max-w-[85%] items-center gap-3 py-1 text-[11px] text-muted-foreground"
            >
                <span className="h-px flex-1 bg-border" />
                <span className="shrink-0 whitespace-nowrap">
                    {entry.mode === 'plan' ? 'Switched to Plan mode' : 'Switched to Chat mode'}
                </span>
                <span className="h-px flex-1 bg-border" />
            </div>
        </li>
    )
})

const ContextCompactionRow = memo(function ContextCompactionRow({ entry }: { entry: ContextCompactionEntry }) {
    return (
        <li className="flex justify-center">
            <div
                data-testid={`project-context-compaction-row-${entry.id}`}
                className="flex w-full max-w-[85%] items-center gap-3 py-1 text-[11px] text-muted-foreground"
            >
                <span className="h-px flex-1 bg-border" />
                <span className="shrink-0 whitespace-nowrap">
                    {entry.content}
                </span>
                <span className="h-px flex-1 bg-border" />
            </div>
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
            className="flex justify-start"
        >
            <ProjectFlowRunRequestEntry
                flowRunRequest={flowRunRequest}
                isLatestFlowRunRequest={isLatestFlowRunRequest}
                pendingFlowRunRequestId={pendingFlowRunRequestId}
                onReviewFlowRunRequest={onReviewFlowRunRequest}
                onOpenFlowRun={onOpenFlowRun}
                formatConversationTimestamp={formatConversationTimestamp}
                getFlowRunRequestStatusPresentation={getFlowRunRequestStatusPresentation}
                getSurfaceToneClassName={getSurfaceToneClassName}
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
            className="flex justify-start"
        >
            <ProjectFlowLaunchEntry
                flowLaunch={flowLaunch}
                isLatestFlowLaunch={isLatestFlowLaunch}
                onOpenFlowRun={onOpenFlowRun}
                formatConversationTimestamp={formatConversationTimestamp}
                getFlowLaunchStatusPresentation={getFlowLaunchStatusPresentation}
                getSurfaceToneClassName={getSurfaceToneClassName}
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
    const launchFlowName = planLaunch?.flow_name || 'software-development/implement-change-request.yaml'

    return (
        <li className="flex min-w-0 justify-start">
            <div
                data-testid={`project-plan-card-${entry.id}`}
                className="min-w-0 max-w-[85%] rounded-md border border-emerald-400/40 bg-emerald-50/60 px-3 py-2 text-foreground"
            >
                <div className="flex flex-wrap items-center gap-2">
                    <p className="text-[10px] font-semibold uppercase tracking-wide text-emerald-800/80">
                        Proposed Plan
                    </p>
                    {statusPresentation ? (
                        <span className={getSurfaceToneClassName(statusPresentation.tone)}>
                            {statusPresentation.label}
                        </span>
                    ) : null}
                </div>
                {entry.status === 'failed' ? (
                    <p className="whitespace-pre-wrap break-words text-xs leading-5 [overflow-wrap:anywhere]">{content}</p>
                ) : (
                    <ProjectConversationMarkdown content={content} />
                )}
                {proposedPlan ? (
                    <div className="mt-2 space-y-2 text-[11px] text-emerald-950/75">
                        {proposedPlan.review_note ? (
                            <p>
                                Review note: <span className="text-foreground">{proposedPlan.review_note}</span>
                            </p>
                        ) : null}
                        {proposedPlan.written_change_request_path ? (
                            <p className="break-all font-mono text-[10px] text-emerald-950/70">
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
                                    variant="outline"
                                    size="xs"
                                    className="px-2 text-xs"
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
                                    className="h-8 border-emerald-500/20 bg-background/80 text-xs"
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
                                        className="px-2 text-xs"
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
                                        variant="outline"
                                        size="xs"
                                        className="px-2 text-xs"
                                    >
                                        Disapprove
                                    </Button>
                                </div>
                            </div>
                        ) : null}
                    </div>
                ) : null}
                <p className="mt-1 text-[10px] text-emerald-900/70">
                    {formatConversationTimestamp(entry.timestamp)}
                </p>
            </div>
        </li>
    )
})

export function ProjectConversationHistory({
    activeConversationId,
    activeProjectPath,
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
}: ProjectConversationHistoryProps) {
    const [planReviewNotes, setPlanReviewNotes] = useState<Record<string, string>>({})
    const [fullToolOutputs, setFullToolOutputs] = useState<Record<string, string>>({})
    const [loadingFullToolOutputs, setLoadingFullToolOutputs] = useState<Record<string, boolean>>({})
    const [fullToolOutputErrors, setFullToolOutputErrors] = useState<Record<string, string>>({})
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
        void fetchConversationSegmentToolOutputValidated(activeConversationId, entry.id, activeProjectPath)
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

    return (
        <div data-testid="project-ai-conversation-history" className="flex min-h-0 flex-col">
            {isConversationHistoryLoading && !hasRenderableConversationHistory ? (
                <Alert
                    data-testid="project-conversation-history-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                >
                    <AlertDescription className="text-inherit">
                        Restoring thread history...
                    </AlertDescription>
                </Alert>
            ) : !hasRenderableConversationHistory ? (
                <p className="rounded-md border border-dashed border-border px-3 py-4 text-sm text-muted-foreground">
                    {activeConversationId
                        ? 'No conversation history for this thread yet.'
                        : 'Create or select a thread to begin chatting.'}
                </p>
            ) : (
                <ol data-testid="project-ai-conversation-history-list" className="space-y-3">
                    {activeConversationHistory.map((entry) => {
                        const key = conversationEntryKey(entry)
                        if (entry.kind === 'tool_call') {
                            return (
                                <ToolCallRow
                                    key={key}
                                    entry={entry}
                                    fullOutput={fullToolOutputs[entry.id] ?? null}
                                    isLoadingFullOutput={loadingFullToolOutputs[entry.id] === true}
                                    isExpanded={expandedToolCalls[entry.toolCall.id] === true}
                                    loadFullOutputError={fullToolOutputErrors[entry.id] ?? null}
                                    onToggleToolCallExpanded={() => onToolCallExpanded(entry)}
                                />
                            )
                        }

                        if (entry.kind === 'final_separator') {
                            return <FinalSeparatorRow key={key} entry={entry} />
                        }

                        if (entry.kind === 'mode_change') {
                            return <ModeChangeRow key={key} entry={entry} />
                        }

                        if (entry.kind === 'context_compaction') {
                            return <ContextCompactionRow key={key} entry={entry} />
                        }

                        if (entry.kind === 'request_user_input') {
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
                        }

                        if (entry.kind === 'message' && entry.role === 'assistant' && entry.presentation === 'thinking') {
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

                        if (entry.kind === 'flow_run_request') {
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

                        if (entry.kind === 'flow_launch') {
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

                        if (entry.kind === 'plan') {
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

                        if (entry.kind !== 'message') {
                            return null
                        }

                        return (
                            <MessageRow
                                key={key}
                                entry={entry}
                                formatConversationTimestamp={formatConversationTimestamp}
                            />
                        )
                    })}
                </ol>
            )}
        </div>
    )
}
