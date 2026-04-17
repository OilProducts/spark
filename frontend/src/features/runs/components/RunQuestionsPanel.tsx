import type { PendingInterviewGate, PendingInterviewGateGroup } from '../model/shared'
import { pendingGateSemanticHint, formatTimestamp } from '../model/shared'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
interface RunQuestionsPanelProps {
    className?: string
    freeformAnswersByGateId: Record<string, string>
    groupedPendingInterviewGates: PendingInterviewGateGroup[]
    onFreeformAnswerChange: (questionId: string, value: string) => void
    onSubmitPendingGateAnswer: (gate: PendingInterviewGate, selectedValue: string) => void
    pendingGateActionError: string | null
    submittingGateIds: Record<string, boolean>
}

export function RunQuestionsPanel({
    className,
    freeformAnswersByGateId,
    groupedPendingInterviewGates,
    onFreeformAnswerChange,
    onSubmitPendingGateAnswer,
    pendingGateActionError,
    submittingGateIds,
}: RunQuestionsPanelProps) {
    if (groupedPendingInterviewGates.length === 0) {
        return null
    }

    return (
        <div
            data-testid="run-pending-human-gates-panel"
            className={`mb-3 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 ${className ?? ''}`.trim()}
        >
            <div className="text-xs font-semibold uppercase tracking-wide text-amber-800">
                Pending Questions
            </div>
            {pendingGateActionError && (
                <Alert
                    data-testid="run-pending-human-gate-answer-error"
                    className="mt-2 border-destructive/40 bg-destructive/10 px-2 py-1 text-xs text-destructive"
                >
                    <AlertDescription className="text-inherit">
                        {pendingGateActionError}
                    </AlertDescription>
                </Alert>
            )}
            <div className="mt-2 space-y-2">
                {groupedPendingInterviewGates.map((group) => (
                    <div
                        key={group.key}
                        data-testid="run-pending-human-gate-group"
                        className="rounded border border-amber-500/30 bg-amber-100/40 px-2 py-1.5"
                    >
                        <div
                            data-testid="run-pending-human-gate-group-heading"
                            className="text-[11px] font-semibold uppercase tracking-wide text-amber-800"
                        >
                            {group.heading}
                        </div>
                        <ul className="mt-1 space-y-1">
                            {group.gates.map((gate, gateIndex) => {
                                const freeformAnswer = gate.questionId
                                    ? freeformAnswersByGateId[gate.questionId] ?? ''
                                    : ''
                                return (
                                    <li key={gate.eventId} data-testid="run-pending-human-gate-item" className="text-xs text-amber-900">
                                        <div>{gate.prompt}</div>
                                        <div
                                            data-testid="run-pending-human-gate-item-audit"
                                            className="mt-0.5 flex flex-wrap items-center gap-2 text-[10px] text-amber-900/80"
                                        >
                                            <span className="font-mono">Order #{gateIndex + 1}</span>
                                            <span>Question ID: {gate.questionId ?? '—'}</span>
                                            <span>Received: {formatTimestamp(gate.receivedAt)}</span>
                                        </div>
                                        {gate.questionId && gate.questionType === 'FREEFORM' && (
                                            <div className="mt-1 flex flex-wrap items-center gap-2">
                                                <Input
                                                    type="text"
                                                    data-testid={`run-pending-human-gate-freeform-input-${gate.questionId}`}
                                                    value={freeformAnswer}
                                                    onChange={(event) => onFreeformAnswerChange(gate.questionId!, event.target.value)}
                                                    disabled={submittingGateIds[gate.questionId] === true}
                                                    placeholder="Type answer..."
                                                    className="h-7 min-w-[18rem] border-amber-500/40 bg-white px-2 text-[11px] text-amber-900 focus-visible:ring-amber-500/40"
                                                />
                                                <Button
                                                    type="button"
                                                    data-testid={`run-pending-human-gate-freeform-submit-${gate.questionId}`}
                                                    onClick={() => {
                                                        onSubmitPendingGateAnswer(gate, freeformAnswer)
                                                    }}
                                                    disabled={submittingGateIds[gate.questionId] === true || freeformAnswer.trim().length === 0}
                                                    variant="outline"
                                                    size="xs"
                                                    className="h-7 border-amber-500/50 bg-white text-[11px] font-medium text-amber-900 hover:bg-amber-100"
                                                >
                                                    Submit
                                                </Button>
                                            </div>
                                        )}
                                        {gate.questionId && gate.questionType !== 'FREEFORM' && gate.options.length > 0 && (
                                            <div className="mt-1 flex flex-wrap gap-1.5">
                                                {gate.options.map((option) => (
                                                    <div key={option.value} className="space-y-1">
                                                        <Button
                                                            type="button"
                                                            data-testid={`run-pending-human-gate-answer-${option.value}`}
                                                            onClick={() => {
                                                                onSubmitPendingGateAnswer(gate, option.value)
                                                            }}
                                                            disabled={submittingGateIds[gate.questionId!] === true}
                                                            variant="outline"
                                                            size="xs"
                                                            className="h-6 border-amber-500/50 bg-white text-[11px] font-medium text-amber-900 hover:bg-amber-100"
                                                        >
                                                            {option.label}
                                                        </Button>
                                                        {(() => {
                                                            const semanticHint = pendingGateSemanticHint(gate.questionType, option.value)
                                                            const showMultipleChoiceMetadata = gate.questionType === 'MULTIPLE_CHOICE'
                                                                && (option.key || option.description)
                                                            if (!showMultipleChoiceMetadata && !semanticHint) {
                                                                return null
                                                            }
                                                            return (
                                                                <div
                                                                    data-testid={`run-pending-human-gate-option-metadata-${option.value}`}
                                                                    className="flex items-center gap-1 text-[10px] text-amber-900/90"
                                                                >
                                                                    {showMultipleChoiceMetadata && option.key && <span className="font-mono">[{option.key}]</span>}
                                                                    {showMultipleChoiceMetadata && option.description && <span>{option.description}</span>}
                                                                    {semanticHint && <span>{semanticHint}</span>}
                                                                </div>
                                                            )
                                                        })()}
                                                    </div>
                                                ))}
                                            </div>
                                        )}
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
