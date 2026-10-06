import { useEffect, useState } from 'react'
import { InlineError } from '@/components/app/inline-error'
import { statusTextClassName } from '@/components/app/status'
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

const cardClassName = 'w-full max-w-[72ch] rounded-xl border border-border px-4 py-3 text-foreground'

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
    }, [entry.id])

    if (entry.requestUserInput.status === 'answered') {
        return (
            <div
                data-testid={`project-request-user-input-summary-${entry.id}`}
                className={cardClassName}
            >
                <p className="text-xs font-medium text-muted-foreground">
                    Answered request
                </p>
                <div className="mt-2 space-y-2">
                    {entry.requestUserInput.questions.map((question) => {
                        const answer = entry.requestUserInput.answers[question.id] ?? ''
                        return (
                            <div key={question.id} className="space-y-0.5">
                                <p className="text-sm font-medium text-foreground">{question.question}</p>
                                <p className="text-sm text-muted-foreground">
                                    {answeredSummaryValue(question, answer)}
                                </p>
                            </div>
                        )
                    })}
                </div>
                <p className="mt-2 text-xs text-muted-foreground">
                    {formatConversationTimestamp(entry.requestUserInput.submittedAt ?? entry.timestamp)}
                </p>
            </div>
        )
    }

    if (entry.requestUserInput.status === 'expired') {
        return (
            <div
                data-testid={`project-request-user-input-expired-${entry.id}`}
                className={cardClassName}
            >
                <p className={`text-xs font-medium ${statusTextClassName('destructive')}`}>
                    Expired request
                </p>
                <p className="mt-2 text-sm text-foreground">
                    The request expired before this answer could be used. Send a new message to continue.
                </p>
                <div className="mt-2 space-y-2">
                    {entry.requestUserInput.questions.map((question) => {
                        const answer = entry.requestUserInput.answers[question.id] ?? ''
                        return (
                            <div key={question.id} className="space-y-0.5">
                                <p className="text-sm font-medium text-foreground">{question.question}</p>
                                <p className="text-sm text-muted-foreground">
                                    {answer ? answeredSummaryValue(question, answer) : 'No answer submitted'}
                                </p>
                            </div>
                        )
                    })}
                </div>
                <p className="mt-2 text-xs text-muted-foreground">
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
            className={cardClassName}
        >
            <p className={`text-xs font-medium ${statusTextClassName('warning')}`}>
                Needs input
            </p>
            {validationError || actionError ? (
                <InlineError data-testid={`project-request-user-input-error-${entry.id}`} className="mt-2" dense>{validationError ?? actionError}</InlineError>
            ) : null}
            <div className="mt-2 space-y-3">
                {entry.requestUserInput.questions.map((question) => {
                    const currentAnswer = draftAnswers[question.id] ?? ''
                    const hasSelectedOption = question.options.some((option) => option.label === currentAnswer)
                    return (
                        <div key={question.id} className="space-y-2">
                            <div className="space-y-0.5">
                                <p className="text-xs font-medium text-muted-foreground">
                                    {question.header}
                                </p>
                                <p className="text-sm text-foreground">{question.question}</p>
                            </div>
                            {question.questionType === 'MULTIPLE_CHOICE' && question.options.length > 0 ? (
                                <div className="flex flex-wrap gap-1.5">
                                    {question.options.map((option) => {
                                        const isSelected = currentAnswer === option.label
                                        const optionTestId = option.value
                                            ? `run-pending-human-gate-answer-${option.value}`
                                            : `project-request-user-input-option-${question.id}-${option.label}`
                                        return (
                                            <Button
                                                key={`${question.id}-${option.label}`}
                                                type="button"
                                                data-testid={optionTestId}
                                                aria-pressed={isSelected}
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
                                                className={`h-7 ${isSelected ? 'border-primary bg-accent text-accent-foreground' : ''}`}
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
                                    className="h-8 bg-background text-sm text-foreground"
                                />
                            ) : null}
                            {question.options.length > 0 ? (
                                <div className="space-y-1">
                                    {question.options.map((option) => (
                                        option.description ? (
                                            <p
                                                key={`${question.id}-${option.label}-description`}
                                                className="text-xs text-muted-foreground"
                                            >
                                                <span className="font-medium text-foreground">{option.label}:</span>{' '}
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
                <p className="text-xs text-muted-foreground">{formatConversationTimestamp(entry.timestamp)}</p>
                <Button
                    type="button"
                    data-testid={`project-request-user-input-submit-${entry.requestUserInput.requestId}`}
                    onClick={submitAnswers}
                    disabled={isSubmitting}
                    variant="outline"
                    size="xs"
                    className="h-7"
                >
                    {isSubmitting ? 'Submitting...' : 'Submit'}
                </Button>
            </div>
        </div>
    )
}
