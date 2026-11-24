import { useCallback, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { InlineError } from '@/components/app/inline-error'
import { StatusDot, type StatusTone } from '@/components/app/status'
import { Well } from '@/components/app/well'
import {
    answerDelegationQuestion,
    cancelDelegation,
    fetchDelegations,
    type DelegationAnswerBody,
    type DelegationOut,
    type DelegationQuestion,
} from '@/lib/api/environmentsApi'

type Filter = 'all' | 'open' | 'paused'

function delegationStatus(record: DelegationOut): { label: string; tone: StatusTone } {
    if (record.phase === 'sealed') {
        const resultStatus = typeof record.result?.status === 'string' ? record.result.status : 'sealed'
        if (resultStatus === 'completed' || resultStatus === 'complete') return { label: resultStatus, tone: 'success' }
        if (resultStatus === 'canceled') return { label: 'canceled', tone: 'neutral' }
        return { label: resultStatus, tone: 'destructive' }
    }
    if (record.paused) return { label: 'paused', tone: 'warning' }
    if ((record.last_state?.questions?.length ?? 0) > 0) return { label: 'waiting on answer', tone: 'warning' }
    if (record.cancel_requested_at) return { label: 'canceling', tone: 'neutral' }
    if (record.last_state?.status) return { label: record.last_state.status, tone: 'info' }
    return { label: record.phase, tone: 'info' }
}

function QuestionResponder({
    delegationId,
    question,
    busy,
    onAnswer,
}: {
    delegationId: string
    question: DelegationQuestion
    busy: boolean
    onAnswer: (delegationId: string, questionId: string, body: DelegationAnswerBody) => Promise<void>
}) {
    const [text, setText] = useState('')
    const [answers, setAnswers] = useState<Record<string, string>>({})
    const [formJson, setFormJson] = useState('')
    const [localError, setLocalError] = useState<string | null>(null)

    if (question.form) {
        const message = typeof question.form.message === 'string' ? question.form.message : 'Form response requested.'
        const submitForm = async () => {
            try {
                const content = JSON.parse(formJson || '{}') as Record<string, unknown>
                setLocalError(null)
                await onAnswer(delegationId, question.question_id, { form: { action: 'accept', content } })
                setFormJson('')
            } catch (err) {
                setLocalError(err instanceof Error ? err.message : 'Enter valid JSON for form content.')
            }
        }
        return (
            <div className="space-y-2 border-t border-border pt-2">
                <div className="text-xs font-medium text-foreground">{message}</div>
                <div className="flex flex-wrap gap-2">
                    <Input
                        aria-label={`Form JSON for ${question.question_id}`}
                        placeholder='{"field": "value"}'
                        value={formJson}
                        onChange={(event) => setFormJson(event.target.value)}
                        className="min-w-[12rem] flex-1 font-mono text-xs"
                    />
                    <Button size="sm" variant="outline" disabled={busy} onClick={() => void submitForm()}>Accept JSON</Button>
                    <Button size="sm" variant="ghost" disabled={busy} onClick={() => void onAnswer(delegationId, question.question_id, { form: { action: 'decline' } })}>Decline</Button>
                </div>
                {localError ? <InlineError>{localError}</InlineError> : null}
            </div>
        )
    }

    if (question.questions && question.questions.length > 0) {
        const submitMulti = async () => {
            await onAnswer(delegationId, question.question_id, { answers })
            setAnswers({})
        }
        return (
            <div className="space-y-2 border-t border-border pt-2">
                {question.questions.map((sub) => (
                    <div key={sub.id} className="space-y-1">
                        <div className="text-xs font-medium text-foreground">{sub.question}</div>
                        <Input
                            aria-label={sub.question}
                            value={answers[sub.id] ?? ''}
                            onChange={(event) => setAnswers((prev) => ({ ...prev, [sub.id]: event.target.value }))}
                            placeholder={sub.options?.map((o) => o.label).join(' / ') || 'Answer'}
                        />
                    </div>
                ))}
                <Button size="sm" variant="outline" disabled={busy} onClick={() => void submitMulti()}>
                    Submit answers
                </Button>
            </div>
        )
    }

    return (
        <div className="space-y-2 border-t border-border pt-2">
            {question.prompt ? <div className="text-xs font-medium text-foreground">{question.prompt}</div> : null}
            {question.options && question.options.length > 0 ? (
                <div className="flex flex-wrap gap-2">
                    {question.options.map((option) => {
                        const value = option.value ?? option.label
                        return (
                            <Button
                                key={value}
                                size="sm"
                                variant="outline"
                                disabled={busy}
                                onClick={() => void onAnswer(delegationId, question.question_id, { selected_value: value })}
                            >
                                {option.label}
                            </Button>
                        )
                    })}
                </div>
            ) : null}
            <div className="flex flex-wrap gap-2">
                <Input
                    aria-label={`Answer for ${question.question_id}`}
                    value={text}
                    onChange={(event) => setText(event.target.value)}
                    placeholder="Free-text answer"
                    className="min-w-[12rem] flex-1"
                />
                <Button
                    size="sm"
                    variant="outline"
                    disabled={busy || !text.trim()}
                    onClick={() => void onAnswer(delegationId, question.question_id, { selected_value: text.trim() }).then(() => setText(''))}
                >
                    Answer
                </Button>
            </div>
        </div>
    )
}

export function DelegationsCard({ active = true }: { active?: boolean }) {
    const [filter, setFilter] = useState<Filter>('all')
    const [records, setRecords] = useState<DelegationOut[]>([])
    const [loading, setLoading] = useState(false)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [status, setStatus] = useState('')

    const refresh = useCallback(async (nextFilter: Filter = filter) => {
        setLoading(true)
        try {
            const list = await fetchDelegations(
                nextFilter === 'open' ? { open: true } : nextFilter === 'paused' ? { paused: true } : undefined,
            )
            setRecords(list)
            setError(null)
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not load delegations.')
        } finally {
            setLoading(false)
        }
    }, [filter])

    useEffect(() => {
        if (!active) return
        void refresh(filter)
    }, [active, filter, refresh])

    const handleCancel = async (record: DelegationOut) => {
        setBusy(true)
        setError(null)
        try {
            const updated = await cancelDelegation(record.delegation_id)
            setStatus(
                updated.phase === 'sealed'
                    ? `Canceled delegation ${record.request_key}.`
                    : `Cancellation requested for ${record.request_key}.`,
            )
            await refresh(filter)
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not cancel delegation.')
        } finally {
            setBusy(false)
        }
    }

    const handleAnswer = async (delegationId: string, questionId: string, body: DelegationAnswerBody) => {
        setBusy(true)
        setError(null)
        try {
            await answerDelegationQuestion(delegationId, questionId, body)
            setStatus(`Sent answer for question ${questionId}.`)
            await refresh(filter)
        } catch (err) {
            setError(err instanceof Error ? err.message : 'Could not submit answer.')
        } finally {
            setBusy(false)
        }
    }

    return (
        <Card className="gap-4 py-4">
            <CardHeader className="px-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                    <h2 className="text-base font-semibold">Outgoing delegations</h2>
                    <div className="flex flex-wrap gap-1">
                        {(['all', 'open', 'paused'] as const).map((choice) => (
                            <Button
                                key={choice}
                                size="sm"
                                variant={filter === choice ? 'secondary' : 'ghost'}
                                aria-pressed={filter === choice}
                                onClick={() => setFilter(choice)}
                            >
                                {choice === 'all' ? 'All' : choice === 'open' ? 'Open' : 'Paused'}
                            </Button>
                        ))}
                        <Button size="sm" variant="ghost" disabled={loading || busy} onClick={() => void refresh(filter)}>
                            Refresh
                        </Button>
                    </div>
                </div>
            </CardHeader>
            <CardContent className="space-y-3 px-4 text-sm">
                <p className="text-muted-foreground">
                    Tasks handed off from this environment to linked Spark machines. Paused delegations resume when re-linked. Canceling one that was never sent ends it here; one the other machine may have keeps its cancel until re-linked.
                </p>
                {loading && records.length === 0 ? (
                    <p role="status" className="text-muted-foreground">Loading delegations…</p>
                ) : records.length === 0 ? (
                    <p className="text-xs text-muted-foreground">No matching delegations.</p>
                ) : (
                    <ul className="space-y-2">
                        {records.map((record) => {
                            const badge = delegationStatus(record)
                            const questions = record.phase === 'accepted' ? (record.last_state?.questions ?? []) : []
                            return (
                                <li key={record.delegation_id}>
                                    <Well className="space-y-2 px-3 py-2">
                                        <div className="flex flex-wrap items-start justify-between gap-2">
                                            <div className="min-w-0 space-y-0.5">
                                                <div className="flex flex-wrap items-center gap-2 font-medium text-foreground">
                                                    <StatusDot tone={badge.tone} />
                                                    <span>{record.request_key}</span>
                                                    <span className="text-xs font-normal text-muted-foreground">
                                                        {record.kind} · {badge.label}
                                                    </span>
                                                </div>
                                                <div className="break-all text-xs text-muted-foreground">
                                                    {record.project_path} · env {record.environment_id}
                                                    {record.target_id ? ` · target ${record.target_id}` : ''}
                                                </div>
                                            </div>
                                            {record.phase !== 'sealed' ? (
                                                <Button
                                                    size="sm"
                                                    variant="ghost"
                                                    aria-label={`Cancel delegation ${record.request_key}`}
                                                    disabled={busy}
                                                    onClick={() => void handleCancel(record)}
                                                >
                                                    Cancel delegation
                                                </Button>
                                            ) : null}
                                        </div>
                                        {record.paused ? (
                                            <div className="text-xs text-warning">{record.paused}</div>
                                        ) : null}
                                        {questions.map((question) => (
                                            <QuestionResponder
                                                key={question.question_id}
                                                delegationId={record.delegation_id}
                                                question={question}
                                                busy={busy}
                                                onAnswer={handleAnswer}
                                            />
                                        ))}
                                    </Well>
                                </li>
                            )
                        })}
                    </ul>
                )}
                {status ? <p role="status" className="text-xs text-muted-foreground">{status}</p> : null}
                {error ? <InlineError>{error}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
