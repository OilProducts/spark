import type { PendingInterviewGate, PendingInterviewGateGroup } from '../model/shared'
import { pendingGateSemanticHint, formatTimestamp } from '../model/shared'
import { InlineError } from '@/components/app/inline-error'
import { ProjectConversationMarkdown } from '@/features/projects/components/ProjectConversationMarkdown'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
interface RunQuestionsPanelProps {
    confirmedQuestionIds: string[]
    className?: string
    freeformAnswersByGateId: Record<string, string>
    gateNotesByGateId: Record<string, string>
    groupedPendingInterviewGates: PendingInterviewGateGroup[]
    onFreeformAnswerChange: (questionId: string, value: string) => void
    onGateNoteChange: (questionId: string, value: string) => void
    onSubmitPendingGateAnswer: (gate: PendingInterviewGate, selectedValue: string, note?: string) => void
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
}

// Each question is a card: the prompt, its details, an optional note, then the answers as
// full-size buttons of equal weight: a flow's option order says nothing about which answer
// is expected, so none is styled as the default.
export function RunQuestionsPanel({
    confirmedQuestionIds,
    className,
    freeformAnswersByGateId,
    gateNotesByGateId,
    groupedPendingInterviewGates,
    onFreeformAnswerChange,
    onGateNoteChange,
    onSubmitPendingGateAnswer,
    pendingGateActionError,
    submittingGateIds,
}: RunQuestionsPanelProps) {
    if (groupedPendingInterviewGates.length === 0) {
        return null
    }
    const showGroupHeadings = groupedPendingInterviewGates.length > 1

    return (
        <div
            data-testid="run-pending-human-gates-panel"
            className={className}
        >
            {pendingGateActionError && (
                <InlineError data-testid="run-pending-human-gate-answer-error" className="mb-3" dense>{pendingGateActionError}</InlineError>
            )}
            <div className="space-y-5">
                {groupedPendingInterviewGates.map((group) => (
                    <div
                        key={group.key}
                        data-testid="run-pending-human-gate-group"
                    >
                        {showGroupHeadings ? (
                            <div
                                data-testid="run-pending-human-gate-group-heading"
                                className="mb-2 text-xs font-semibold text-muted-foreground"
                            >
                                {group.heading}
                            </div>
                        ) : null}
                        <ul className="space-y-3">
                            {group.gates.map((gate, gateIndex) => {
                                const questionId = gate.questionId
                                const freeformAnswer = questionId
                                    ? freeformAnswersByGateId[questionId] ?? ''
                                    : ''
                                const gateNote = questionId
                                    ? gateNotesByGateId[questionId] ?? ''
                                    : ''
                                const disabled = !questionId || !confirmedQuestionIds.includes(questionId) || submittingGateIds[questionId] === true
                                const isFreeform = gate.questionType === 'FREEFORM'
                                const showOptions = Boolean(questionId) && (!isFreeform || gate.origin === 'agent_clarification') && gate.options.length > 0
                                const noteId = questionId ? `run-gate-note-${questionId}` : undefined
                                return (
                                    <li
                                        key={gate.eventId}
                                        data-testid="run-pending-human-gate-item"
                                        className="flex max-w-[72ch] flex-col gap-3 rounded-xl border border-border bg-background p-4 text-sm text-foreground"
                                    >
                                        <p className="text-base font-medium leading-snug">{gate.prompt}</p>
                                        {gate.details && (
                                            <div
                                                data-testid="run-pending-human-gate-details"
                                                className="max-h-72 overflow-y-auto rounded-lg bg-muted px-3 py-2 text-sm text-foreground"
                                            >
                                                <ProjectConversationMarkdown content={gate.details} />
                                            </div>
                                        )}
                                        {questionId && isFreeform && (
                                            <div className="flex flex-wrap items-center gap-2">
                                                <Input
                                                    type="text"
                                                    data-testid={`run-pending-human-gate-freeform-input-${questionId}`}
                                                    value={freeformAnswer}
                                                    onChange={(event) => onFreeformAnswerChange(questionId, event.target.value)}
                                                    disabled={disabled}
                                                    aria-label="Answer"
                                                    placeholder="Type your answer…"
                                                    className="h-9 min-w-0 flex-[1_1_14rem] bg-background"
                                                />
                                                <Button
                                                    type="button"
                                                    data-testid={`run-pending-human-gate-freeform-submit-${questionId}`}
                                                    onClick={() => {
                                                        onSubmitPendingGateAnswer(gate, freeformAnswer)
                                                    }}
                                                    disabled={disabled || freeformAnswer.trim().length === 0}
                                                    size="sm"
                                                    className="h-9"
                                                >
                                                    Submit
                                                </Button>
                                            </div>
                                        )}
                                        {showOptions && questionId && (
                                            <>
                                                {gate.origin !== 'agent_clarification' && (
                                                    <div className="flex flex-col gap-1.5">
                                                        <label htmlFor={noteId} className="text-xs font-medium text-muted-foreground">
                                                            Note for the next step (optional)
                                                        </label>
                                                        <Input
                                                            id={noteId}
                                                            type="text"
                                                            data-testid={`run-pending-human-gate-note-input-${questionId}`}
                                                            value={gateNote}
                                                            onChange={(event) => onGateNoteChange(questionId, event.target.value)}
                                                            disabled={disabled}
                                                            placeholder="Add context the next step should see…"
                                                            className="h-9 w-full bg-background"
                                                        />
                                                    </div>
                                                )}
                                                <div className="flex flex-wrap items-start gap-2">
                                                    {gate.options.map((option) => {
                                                        const semanticHint = pendingGateSemanticHint(gate.questionType, option.value)
                                                        const showMultipleChoiceMetadata = (gate.questionType === 'MULTIPLE_CHOICE' || gate.origin === 'agent_clarification')
                                                            && Boolean(option.key || option.description)
                                                        return (
                                                            <div key={option.value} className="flex min-w-0 max-w-[min(20rem,100%)] flex-col gap-1">
                                                                <Button
                                                                    type="button"
                                                                    data-testid={`run-pending-human-gate-answer-${option.value}`}
                                                                    onClick={() => {
                                                                        onSubmitPendingGateAnswer(gate, option.value, gateNote)
                                                                    }}
                                                                    disabled={disabled}
                                                                    variant="outline"
                                                                    size="sm"
                                                                    className="h-auto min-h-8 max-w-full self-start whitespace-normal py-1.5 text-left"
                                                                >
                                                                    {option.label}
                                                                </Button>
                                                                {showMultipleChoiceMetadata || semanticHint ? (
                                                                    <div
                                                                        data-testid={`run-pending-human-gate-option-metadata-${option.value}`}
                                                                        className="text-xs text-muted-foreground"
                                                                    >
                                                                        {showMultipleChoiceMetadata && option.key ? <span className="mr-1 font-mono">[{option.key}]</span> : null}
                                                                        {showMultipleChoiceMetadata && option.description ? <span>{option.description}</span> : null}
                                                                        {semanticHint ? <span>{semanticHint}</span> : null}
                                                                    </div>
                                                                ) : null}
                                                            </div>
                                                        )
                                                    })}
                                                </div>
                                            </>
                                        )}
                                        <div
                                            data-testid="run-pending-human-gate-item-audit"
                                            className="flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground"
                                        >
                                            {group.gates.length > 1 ? <><span>Order #{gateIndex + 1}</span><span aria-hidden="true">·</span></> : null}
                                            <span>Received: {formatTimestamp(gate.receivedAt)}</span>
                                            <span aria-hidden="true">·</span>
                                            <span>Question ID: {questionId ?? '—'}</span>
                                        </div>
                                    </li>
                                )
                            })}
                        </ul>
                    </div>
                ))}
            </div>
        </div>
    )
}
