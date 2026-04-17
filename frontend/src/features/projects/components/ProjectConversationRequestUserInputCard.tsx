import { useEffect, useState } from 'react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { ConversationTimelineEntry } from '../model/types'

type RequestUserInputEntry = Extract<ConversationTimelineEntry, { kind: 'request_user_input' }>

interface ProjectConversationRequestUserInputCardProps {
    actionError: string | null
    entry: RequestUserInputEntry
    formatConversationTimestamp: (value: string) => string
    isSubmitting: boolean
    onSubmitRequestUserInput: (requestId: string, answers: Record<string, string>) => void | Promise<void>
}

function answeredSummaryValue(
    question: RequestUserInputEntry['requestUserInput']['questions'][number],
    answer: string,
): string {
    if (question.isSecret) {
        return 'Answer submitted'
    }
    return answer
}

export function ProjectConversationRequestUserInputCard({
    actionError,
    entry,
    formatConversationTimestamp,
    isSubmitting,
    onSubmitRequestUserInput,
}: ProjectConversationRequestUserInputCardProps) {
    const [draftAnswers, setDraftAnswers] = useState<Record<string, string>>({})
    const [validationError, setValidationError] = useState<string | null>(null)

    useEffect(() => {
        setValidationError(null)
        setDraftAnswers(entry.requestUserInput.answers)
    }, [entry.id, entry.requestUserInput.answers])

    if (entry.requestUserInput.status === 'answered') {
        return (
            <div
                data-testid={`project-request-user-input-summary-${entry.id}`}
                className="max-w-[85%] rounded-md border border-amber-500/40 bg-amber-50/70 px-3 py-2 text-foreground"
            >
                <p className="text-[10px] font-semibold uppercase tracking-wide text-amber-900/80">
                    Answered Request
                </p>
                <div className="mt-2 space-y-2">
                    {entry.requestUserInput.questions.map((question) => {
                        const answer = entry.requestUserInput.answers[question.id] ?? ''
                        return (
                            <div key={question.id} className="space-y-0.5">
                                <p className="text-xs font-medium text-foreground">{question.question}</p>
                                <p className="text-xs text-muted-foreground">
                                    {answeredSummaryValue(question, answer)}
                                </p>
                            </div>
                        )
                    })}
                </div>
                <p className="mt-2 text-[10px] text-amber-900/70">
                    {formatConversationTimestamp(entry.requestUserInput.submittedAt ?? entry.timestamp)}
                </p>
            </div>
        )
    }

    if (entry.requestUserInput.status === 'expired') {
        return (
            <div
                data-testid={`project-request-user-input-expired-${entry.id}`}
                className="max-w-[85%] rounded-md border border-rose-500/40 bg-rose-50/70 px-3 py-2 text-foreground"
            >
                <p className="text-[10px] font-semibold uppercase tracking-wide text-rose-900/80">
                    Expired Request
                </p>
                <p className="mt-2 text-xs text-rose-950/85">
                    The request expired before this answer could be used. Send a new message to continue.
                </p>
                <div className="mt-2 space-y-2">
                    {entry.requestUserInput.questions.map((question) => {
                        const answer = entry.requestUserInput.answers[question.id] ?? ''
                        return (
                            <div key={question.id} className="space-y-0.5">
                                <p className="text-xs font-medium text-foreground">{question.question}</p>
                                <p className="text-xs text-muted-foreground">
                                    {answer ? answeredSummaryValue(question, answer) : 'No answer submitted'}
                                </p>
                            </div>
                        )
                    })}
                </div>
                <p className="mt-2 text-[10px] text-rose-900/70">
                    {formatConversationTimestamp(entry.requestUserInput.submittedAt ?? entry.timestamp)}
                </p>
            </div>
        )
    }

    const submitAnswers = () => {
        const normalizedAnswers = Object.fromEntries(
            entry.requestUserInput.questions
                .map((question) => [question.id, (draftAnswers[question.id] ?? '').trim()] as const)
                .filter(([, answer]) => answer.length > 0),
        )
        const missingQuestion = entry.requestUserInput.questions.find((question) => !normalizedAnswers[question.id])
        if (missingQuestion) {
            setValidationError(`Answer "${missingQuestion.header}" before submitting.`)
            return
        }
        setValidationError(null)
        void onSubmitRequestUserInput(entry.requestUserInput.requestId, normalizedAnswers)
    }

    return (
        <div
            data-testid={`project-request-user-input-card-${entry.id}`}
            className="w-full max-w-[85%] rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-foreground"
        >
            <p className="text-[10px] font-semibold uppercase tracking-wide text-amber-900/80">
                Needs Input
            </p>
            {validationError || actionError ? (
                <Alert
                    data-testid={`project-request-user-input-error-${entry.id}`}
                    className="mt-2 border-destructive/40 bg-destructive/10 px-2 py-1 text-xs text-destructive"
                >
                    <AlertDescription className="text-inherit">
                        {validationError ?? actionError}
                    </AlertDescription>
                </Alert>
            ) : null}
            <div className="mt-2 space-y-3">
                {entry.requestUserInput.questions.map((question) => {
                    const currentAnswer = draftAnswers[question.id] ?? ''
                    const hasSelectedOption = question.options.some((option) => option.label === currentAnswer)
                    return (
                        <div key={question.id} className="space-y-2">
                            <div className="space-y-0.5">
                                <p className="text-[10px] font-semibold uppercase tracking-wide text-amber-900/80">
                                    {question.header}
                                </p>
                                <p className="text-xs text-foreground">{question.question}</p>
                            </div>
                            {question.questionType === 'MULTIPLE_CHOICE' && question.options.length > 0 ? (
                                <div className="flex flex-wrap gap-1.5">
                                    {question.options.map((option) => {
                                        const isSelected = currentAnswer === option.label
                                        return (
                                            <Button
                                                key={`${question.id}-${option.label}`}
                                                type="button"
                                                data-testid={`project-request-user-input-option-${question.id}-${option.label}`}
                                                onClick={() => {
                                                    setValidationError(null)
                                                    setDraftAnswers((current) => ({
                                                        ...current,
                                                        [question.id]: option.label,
                                                    }))
                                                }}
                                                disabled={isSubmitting}
                                                variant="outline"
                                                size="xs"
                                                className={`h-7 text-[11px] ${
                                                    isSelected
                                                        ? 'border-amber-700 bg-amber-100 text-amber-950'
                                                        : 'border-amber-500/50 bg-white text-amber-900 hover:bg-amber-100'
                                                }`}
                                            >
                                                {option.label}
                                            </Button>
                                        )
                                    })}
                                </div>
                            ) : null}
                            {question.questionType === 'FREEFORM' || question.allowOther ? (
                                <Input
                                    type={question.isSecret ? 'password' : 'text'}
                                    data-testid={`project-request-user-input-field-${question.id}`}
                                    value={hasSelectedOption && question.questionType === 'MULTIPLE_CHOICE' ? '' : currentAnswer}
                                    onChange={(event) => {
                                        setValidationError(null)
                                        setDraftAnswers((current) => ({
                                            ...current,
                                            [question.id]: event.target.value,
                                        }))
                                    }}
                                    disabled={isSubmitting}
                                    placeholder={question.allowOther ? 'Or enter another answer...' : 'Type answer...'}
                                    className="h-8 border-amber-500/40 bg-white text-[11px] text-amber-950 focus-visible:ring-amber-500/40"
                                />
                            ) : null}
                            {question.options.length > 0 ? (
                                <div className="space-y-1">
                                    {question.options.map((option) => (
                                        option.description ? (
                                            <p
                                                key={`${question.id}-${option.label}-description`}
                                                className="text-[10px] text-amber-900/80"
                                            >
                                                <span className="font-medium text-amber-900">{option.label}:</span>{' '}
                                                {option.description}
                                            </p>
                                        ) : null
                                    ))}
                                </div>
                            ) : null}
                        </div>
                    )
                })}
            </div>
            <div className="mt-3 flex items-center justify-between gap-2">
                <p className="text-[10px] text-amber-900/70">{formatConversationTimestamp(entry.timestamp)}</p>
                <Button
                    type="button"
                    data-testid={`project-request-user-input-submit-${entry.requestUserInput.requestId}`}
                    onClick={submitAnswers}
                    disabled={isSubmitting}
                    variant="outline"
                    size="xs"
                    className="h-7 border-amber-600/60 bg-white text-[11px] font-medium text-amber-950 hover:bg-amber-100"
                >
                    {isSubmitting ? 'Submitting...' : 'Submit'}
                </Button>
            </div>
        </div>
    )
}
