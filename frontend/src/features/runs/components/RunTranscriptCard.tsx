import { useState } from 'react'
import type { RunBoundaryMeta, RunTranscriptEntry, PendingInterviewGate } from '../model/shared'
import { formatTimestamp, RUN_JOURNAL_WINDOW_SIZE, TIMELINE_SEVERITY_STYLES } from '../model/shared'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { ProjectConversationRequestUserInputCard } from '@/features/projects/components/ProjectConversationRequestUserInputCard'
import { ToolCallRow } from '@/features/projects/components/ProjectConversationHistory'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug'
import { TIMELINE_UPDATE_BUDGET_MS } from '@/lib/performanceBudgets'

interface RunTranscriptCardProps {
    entries: RunTranscriptEntry[]
    isTimelineLive: boolean
    onSubmitPendingGateAnswer: (gate: PendingInterviewGate, selectedValue: string) => void
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
    timelineError: string | null
    timelineEventCount: number
    isNarrowViewport?: boolean
}

function sourceLabel(meta: RunBoundaryMeta) {
    if (meta.source_scope !== 'child') {
        return null
    }
    const flowLabel = meta.source_flow_name ? `Child flow ${meta.source_flow_name}` : 'Child flow'
    return meta.source_parent_node_id ? `${flowLabel} via ${meta.source_parent_node_id}` : flowLabel
}

function BoundaryRow({ entry }: { entry: RunTranscriptEntry }) {
    const meta = entry.boundary
    if (!meta) {
        return null
    }
    const label = meta.node_id ?? 'run'
    const source = sourceLabel(meta)
    return (
        <li className="flex justify-center">
            <div
                data-testid="run-transcript-boundary"
                className="w-full rounded-md border border-border/70 bg-background px-3 py-2"
            >
                <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="min-w-0">
                        <p className="truncate text-xs font-semibold text-foreground">
                            {label}
                            {meta.stage_index !== null && meta.stage_index !== undefined ? ` · index ${meta.stage_index}` : ''}
                            {meta.attempt !== null && meta.attempt !== undefined ? ` · attempt ${meta.attempt}` : ''}
                        </p>
                        {source ? (
                            <p className="text-[11px] text-muted-foreground">{source}</p>
                        ) : null}
                    </div>
                    <span
                        data-testid="run-transcript-boundary-status"
                        className={`rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                            entry.status === 'failed'
                                ? 'bg-destructive/10 text-destructive'
                                : entry.status === 'completed'
                                    ? 'bg-emerald-500/15 text-emerald-800'
                                    : entry.status === 'retrying'
                                        ? 'bg-amber-500/15 text-amber-800'
                                        : 'bg-sky-500/15 text-sky-700'
                        }`}
                    >
                        {entry.status}
                    </span>
                </div>
                <p className="mt-1 text-[10px] text-muted-foreground">
                    {formatTimestamp(meta.started_at ?? meta.ended_at ?? null)}
                    {meta.ended_at && meta.started_at ? ` - ${formatTimestamp(meta.ended_at)}` : ''}
                    {meta.model ? ` · ${meta.model}` : ''}
                </p>
            </div>
        </li>
    )
}

type RunMessageSegmentEntry = RunTranscriptEntry & { kind: 'assistant_message' | 'plan' | 'reasoning' }
type RunNoticeSegmentEntry = RunTranscriptEntry & { kind: 'context_compaction' }
type RunRequestUserInputSegmentEntry = RunTranscriptEntry & { kind: 'request_user_input' }
type RunToolCallSegmentEntry = RunTranscriptEntry & { kind: 'tool_call' }

function MessageRow({ entry }: { entry: RunMessageSegmentEntry }) {
    const isThinking = entry.kind === 'reasoning'
    return (
        <li className="flex justify-start">
            <div
                data-testid="run-transcript-message"
                className={`max-w-[85%] rounded border px-3 py-2 ${
                    isThinking
                        ? 'border-border/80 bg-background text-muted-foreground'
                        : 'border-border bg-muted/40 text-foreground'
                }`}
            >
                <p className="text-[10px] font-semibold uppercase tracking-wide opacity-70">
                    {entry.kind === 'assistant_message' ? 'Spark' : entry.kind === 'plan' ? 'Plan' : 'Thinking'}
                    {entry.status === 'streaming' || entry.status === 'running' ? ' · Streaming' : ''}
                </p>
                {isThinking ? (
                    <p className="whitespace-pre-wrap text-xs italic leading-5">{entry.content}</p>
                ) : (
                    <ProjectConversationMarkdown content={entry.content} />
                )}
                <p className="mt-1 text-[10px] opacity-70">{formatTimestamp(entry.updated_at || entry.timestamp)}</p>
            </div>
        </li>
    )
}

function NoticeRow({ entry }: { entry: RunNoticeSegmentEntry }) {
    const severity = entry.status === 'failed' ? 'error' : 'info'
    return (
        <li className="flex justify-start">
            <div
                data-testid="run-transcript-notice"
                className={`max-w-[85%] rounded border px-3 py-2 text-xs ${TIMELINE_SEVERITY_STYLES[severity]}`}
            >
                <p className="whitespace-pre-wrap leading-5">{entry.content}</p>
                <p className="mt-1 text-[10px] opacity-70">{formatTimestamp(entry.timestamp || entry.updated_at)}</p>
            </div>
        </li>
    )
}

function toRequestUserInputEntry(entry: RunRequestUserInputSegmentEntry): Extract<ConversationTimelineEntry, { kind: 'request_user_input' }> {
    const request = entry.request_user_input
    return {
        id: entry.id,
        kind: 'request_user_input',
        role: 'system',
        timestamp: entry.timestamp,
        content: entry.content,
        status: entry.status === 'running' || entry.status === 'streaming'
            ? 'streaming'
            : entry.status === 'pending'
                ? 'pending'
                : entry.status === 'failed'
                    ? 'failed'
                    : 'complete',
        requestUserInput: {
            requestId: request?.request_id ?? entry.id,
            status: request?.status ?? 'pending',
            questions: request?.questions.map((question) => ({
                id: question.id,
                header: question.header,
                question: question.question,
                questionType: question.question_type,
                options: question.options.map((option) => ({
                    label: option.label,
                    value: option.value ?? undefined,
                    description: option.description ?? null,
                })),
                allowOther: question.allow_other,
                isSecret: question.is_secret,
            })) ?? [],
            answers: request?.answers ?? {},
            submittedAt: request?.submitted_at ?? null,
        },
    }
}

function toPendingInterviewGate(entry: RunRequestUserInputSegmentEntry): PendingInterviewGate {
    const request = entry.request_user_input
    const firstQuestion = request?.questions[0]
    const source = entry.source ?? {}
    return {
        eventId: entry.id,
        sequence: entry.order,
        receivedAt: entry.timestamp,
        nodeId: typeof source.node_id === 'string' ? source.node_id : null,
        stageIndex: null,
        sourceScope: source.source_scope === 'child' ? 'child' : 'root',
        sourceParentNodeId: typeof source.source_parent_node_id === 'string' ? source.source_parent_node_id : null,
        sourceFlowName: typeof source.source_flow_name === 'string' ? source.source_flow_name : null,
        prompt: firstQuestion?.question ?? entry.content,
        questionId: request?.request_id ?? firstQuestion?.id ?? null,
        questionType: firstQuestion?.question_type ?? null,
        options: firstQuestion?.options.map((option) => ({
            label: option.label,
            value: option.value ?? option.label,
            key: null,
            description: option.description ?? null,
        })) ?? [],
    }
}

function InputRow({
    entry,
    onSubmitPendingGateAnswer,
    pendingGateActionError,
    submittingGateIds,
}: {
    entry: RunRequestUserInputSegmentEntry
    onSubmitPendingGateAnswer: (gate: PendingInterviewGate, selectedValue: string) => void
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
}) {
    const requestEntry = toRequestUserInputEntry(entry)
    const gate = toPendingInterviewGate(entry)
    return (
        <li data-testid="run-transcript-input" className="flex justify-start">
            <ProjectConversationRequestUserInputCard
                actionError={pendingGateActionError}
                entry={requestEntry}
                formatConversationTimestamp={formatTimestamp}
                isSubmitting={gate.questionId ? submittingGateIds[gate.questionId] === true : false}
                onSubmitRequestUserInput={(_requestId, answers) => {
                    const answerKey = gate.questionId ?? gate.eventId
                    const answer = answers[answerKey] ?? ''
                    const selectedOption = gate.options.find((option) => option.label === answer || option.value === answer)
                    onSubmitPendingGateAnswer(gate, selectedOption?.value ?? answer)
                }}
            />
        </li>
    )
}

function ToolCallTranscriptRow({
    entry,
    isExpanded,
    onToggleToolCallExpanded,
}: {
    entry: RunToolCallSegmentEntry
    isExpanded: boolean
    onToggleToolCallExpanded: (toolCallId: string) => void
}) {
    const toolCall = entry.tool_call
    if (!toolCall) {
        return null
    }
    const conversationEntry: Extract<ConversationTimelineEntry, { kind: 'tool_call' }> = {
        id: entry.id,
        kind: 'tool_call',
        role: 'system',
        timestamp: entry.timestamp,
        toolCall: {
            id: toolCall.id,
            kind: toolCall.kind,
            status: toolCall.status,
            title: toolCall.title,
            command: toolCall.command ?? null,
            output: toolCall.output ?? null,
            outputSize: toolCall.output_size ?? null,
            outputTruncated: toolCall.output_truncated === true,
            filePaths: toolCall.file_paths,
        },
    }
    return (
        <ToolCallRow
            entry={conversationEntry}
            fullOutput={null}
            isLoadingFullOutput={false}
            isExpanded={isExpanded}
            loadFullOutputError={null}
            onToggleToolCallExpanded={onToggleToolCallExpanded}
        />
    )
}

function transcriptSearchText(entry: RunTranscriptEntry): string {
    if (entry.kind === 'boundary') {
        return entry.boundary?.summary ?? entry.content
    }
    if (entry.kind === 'tool_call') {
        const toolCall = entry.tool_call
        if (!toolCall) {
            return ''
        }
        return [
            toolCall.title,
            toolCall.command,
            toolCall.output,
            ...toolCall.file_paths,
        ].filter(Boolean).join(' ')
    }
    if (entry.kind === 'request_user_input') {
        return entry.request_user_input?.questions[0]?.question ?? entry.content
    }
    return entry.content
}

export function RunTranscriptCard({
    entries,
    isTimelineLive,
    onSubmitPendingGateAnswer,
    pendingGateActionError,
    submittingGateIds,
    timelineError,
    timelineEventCount,
    isNarrowViewport = false,
}: RunTranscriptCardProps) {
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const showPerformanceDebug = isPerformanceDebugEnabled()
    const compatibilityRows = entries
        .filter((entry) => entry.kind === 'boundary' || entry.kind === 'context_compaction')
        .slice(-RUN_JOURNAL_WINDOW_SIZE)
        .reverse()
    const visibleEntries = showPerformanceDebug ? compatibilityRows : entries
    return (
        <Card data-testid="run-transcript-panel" className="gap-4 py-4">
            <CardHeader className="gap-1 px-4">
                <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Workflow Transcript</h3>
                        <p className="text-xs leading-5 text-muted-foreground">
                            Run output grouped by workflow node.
                        </p>
                    </div>
                    <span
                        className={`inline-flex rounded border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                            isTimelineLive
                                ? 'border-sky-500/40 bg-sky-500/10 text-sky-700'
                                : 'border-border bg-muted text-muted-foreground'
                        }`}
                    >
                        {isTimelineLive ? 'Live' : 'Idle'}
                    </span>
                </div>
            </CardHeader>
            <CardContent
                data-testid="run-transcript-content"
                data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
                className="space-y-3 px-4"
            >
                {showPerformanceDebug ? (
                    <>
                        <div
                            data-testid="run-transcript-performance-budget"
                            data-budget-ms={TIMELINE_UPDATE_BUDGET_MS}
                            className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                        >
                            Transcript update budget: {TIMELINE_UPDATE_BUDGET_MS}ms max per live update batch.
                        </div>
                        <div
                            data-testid="run-transcript-throughput"
                            data-loaded-count={timelineEventCount}
                            data-rendered-count={compatibilityRows.length}
                            data-window-size={RUN_JOURNAL_WINDOW_SIZE}
                            className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                        >
                            Loaded {timelineEventCount} journal entries. Rendering {compatibilityRows.length} transcript rows.
                        </div>
                    </>
                ) : null}
                {timelineError ? (
                    <Alert
                        data-testid="run-transcript-error"
                        className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive"
                    >
                        <AlertDescription className="text-inherit">{timelineError}</AlertDescription>
                    </Alert>
                ) : null}
                {!timelineError && visibleEntries.length === 0 ? (
                    <p data-testid="run-transcript-empty" className="rounded-md border border-dashed border-border px-3 py-4 text-sm text-muted-foreground">
                        No transcript output has been recorded for this run yet.
                    </p>
                ) : null}
                {visibleEntries.length > 0 ? (
                    <ol data-testid="run-transcript-rows" className="space-y-3">
                        {showPerformanceDebug ? (
                            <span className="sr-only">
                                {compatibilityRows.map((entry) => (
                                    <span key={`compat-${entry.id}`} data-testid="run-event-timeline-row">
                                        {transcriptSearchText(entry)}
                                    </span>
                                ))}
                            </span>
                        ) : null}
                        <span data-testid="run-transcript-list" className="sr-only">
                            {visibleEntries.map(transcriptSearchText).join(' ')}
                        </span>
                        {visibleEntries.map((entry) => {
                            if (entry.kind === 'boundary') {
                                return <BoundaryRow key={entry.id} entry={entry} />
                            }
                            if (entry.kind === 'assistant_message' || entry.kind === 'plan' || entry.kind === 'reasoning') {
                                return <MessageRow key={entry.id} entry={entry as RunMessageSegmentEntry} />
                            }
                            if (entry.kind === 'context_compaction') {
                                return <NoticeRow key={entry.id} entry={entry as RunNoticeSegmentEntry} />
                            }
                            if (entry.kind === 'tool_call') {
                                const toolEntry = entry as RunToolCallSegmentEntry
                                return (
                                    <ToolCallTranscriptRow
                                        key={entry.id}
                                        entry={toolEntry}
                                        isExpanded={toolEntry.tool_call ? expandedToolCalls[toolEntry.tool_call.id] === true : false}
                                        onToggleToolCallExpanded={(toolCallId) => {
                                            setExpandedToolCalls((current) => ({
                                                ...current,
                                                [toolCallId]: current[toolCallId] !== true,
                                            }))
                                        }}
                                    />
                                )
                            }
                            return (
                                <InputRow
                                    key={entry.id}
                                    entry={entry as RunRequestUserInputSegmentEntry}
                                    onSubmitPendingGateAnswer={onSubmitPendingGateAnswer}
                                    pendingGateActionError={pendingGateActionError}
                                    submittingGateIds={submittingGateIds}
                                />
                            )
                        })}
                    </ol>
                ) : null}
            </CardContent>
        </Card>
    )
}
