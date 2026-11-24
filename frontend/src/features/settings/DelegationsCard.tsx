import { useCallback, useEffect, useRef, useState } from 'react'
import { useDialogController } from '@/components/app/dialog-controller'
import { FormQuestionFields } from '@/components/app/form-question'
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

const FILTER_QUERY = { all: undefined, open: { open: true }, paused: { paused: true } } as const
// Delegations still in flight are re-read this often, so new questions and phases show up.
const LIVE_REFRESH_MS = 4000

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)
// Delegation IDs are UUIDs, so the first ':' ends the delegation part.
const ackKey = (delegationId: string, questionId: string) => `${delegationId}:${questionId}`

function delegationStatus(record: DelegationOut): { label: string; tone: StatusTone } {
    if (record.phase === 'sealed') {
        const resultStatus = typeof record.result?.status === 'string' ? record.result.status : 'sealed'
        if (resultStatus === 'completed' || resultStatus === 'complete') return { label: resultStatus, tone: 'success' }
        if (resultStatus === 'canceled') return { label: 'canceled', tone: 'neutral' }
        return { label: resultStatus, tone: 'destructive' }
    }
    if (record.paused) return { label: 'paused', tone: 'warning' }
    // A requested cancellation outranks questions the last snapshot still lists.
    if (record.cancel_requested_at) return { label: 'canceling', tone: 'neutral' }
    if ((record.last_state?.questions?.length ?? 0) > 0) return { label: 'waiting on answer', tone: 'warning' }
    if (record.last_state?.status) return { label: record.last_state.status, tone: 'info' }
    return { label: record.phase, tone: 'info' }
}

function QuestionResponder({
    draftKey,
    requestKey,
    question,
    disabled,
    onAnswer,
}: {
    draftKey: string
    requestKey: string
    question: DelegationQuestion
    disabled: boolean
    /** Resolves to why the answer failed, or null once it was sent. */
    onAnswer: (body: DelegationAnswerBody) => Promise<string | null>
}) {
    const [text, setText] = useState('')
    const [answers, setAnswers] = useState<Record<string, string>>({})
    const [error, setError] = useState<string | null>(null)
    const label = question.prompt || question.form?.message || question.questions?.map((sub) => sub.question).join(' ') || 'Question'

    // A failed answer keeps the draft beside its error; a sent one clears it.
    const send = async (body: DelegationAnswerBody, clearDraft: () => void) => {
        setError(null)
        const failure = await onAnswer(body)
        if (failure) setError(failure)
        else clearDraft()
    }

    const submitAnswers = (subs: NonNullable<DelegationQuestion['questions']>) => {
        const trimmed = Object.fromEntries(subs.map((sub) => [sub.id, (answers[sub.id] ?? '').trim()]))
        const missing = subs.find((sub) => !trimmed[sub.id])
        if (missing) {
            setError(`Answer "${missing.question}" before submitting.`)
            return
        }
        void send({ answers: trimmed }, () => setAnswers({}))
    }

    const options = question.options ?? []
    // A run gate with declared choices takes only one of them (attractor-api's answer check).
    const restricted = options.length > 0 && question.origin !== 'agent_clarification' && question.question_type !== 'FREEFORM'

    return (
        <div role="group" aria-label={`${requestKey}: ${label}`} className="space-y-2 border-t border-border pt-2">
            {question.form ? (
                <FormQuestionFields
                    actionError={error}
                    draftKey={`delegation:${draftKey}`}
                    form={question.form}
                    isSubmitting={disabled}
                    testId={`delegation-form-${question.question_id}`}
                    onReply={(reply) => void send({ form: reply }, () => undefined)}
                />
            ) : question.questions ? (
                <>
                    {question.questions.map((sub) => {
                        const value = answers[sub.id] ?? ''
                        const subOptions = sub.options ?? []
                        const picked = subOptions.some((option) => (option.value ?? option.label) === value)
                        const setAnswer = (next: string) => {
                            setError(null)
                            setAnswers((current) => ({ ...current, [sub.id]: next }))
                        }
                        return (
                            <div key={sub.id} className="space-y-1">
                                <div className="text-xs font-medium text-foreground">{sub.question}</div>
                                {subOptions.length > 0 ? (
                                    <div role="group" aria-label={sub.question} className="flex flex-wrap gap-2">
                                        {subOptions.map((option) => {
                                            const optionValue = option.value ?? option.label
                                            return (
                                                <Button
                                                    key={optionValue}
                                                    type="button"
                                                    size="sm"
                                                    variant={value === optionValue ? 'secondary' : 'outline'}
                                                    aria-pressed={value === optionValue}
                                                    disabled={disabled}
                                                    onClick={() => setAnswer(optionValue)}
                                                >
                                                    {option.label}
                                                </Button>
                                            )
                                        })}
                                    </div>
                                ) : null}
                                {subOptions.length === 0 || sub.allow_other || sub.question_type === 'FREEFORM' ? (
                                    <Input
                                        type={sub.is_secret ? 'password' : 'text'}
                                        autoComplete="off"
                                        aria-label={sub.question}
                                        value={picked ? '' : value}
                                        disabled={disabled}
                                        onChange={(event) => setAnswer(event.target.value)}
                                        placeholder={subOptions.length > 0 ? 'Or enter another answer' : 'Answer'}
                                    />
                                ) : null}
                            </div>
                        )
                    })}
                    <Button size="sm" variant="outline" disabled={disabled} onClick={() => submitAnswers(question.questions ?? [])}>
                        Submit answers
                    </Button>
                </>
            ) : (
                <>
                    {question.prompt ? <div className="text-xs font-medium text-foreground">{question.prompt}</div> : null}
                    {options.length > 0 ? (
                        <div className="flex flex-wrap gap-2">
                            {options.map((option) => {
                                const value = option.value ?? option.label
                                return (
                                    <Button
                                        key={value}
                                        size="sm"
                                        variant="outline"
                                        disabled={disabled}
                                        onClick={() => void send({ selected_value: value }, () => undefined)}
                                    >
                                        {option.label}
                                    </Button>
                                )
                            })}
                        </div>
                    ) : null}
                    {restricted ? null : (
                        <div className="flex flex-wrap gap-2">
                            <Input
                                aria-label="Free-text answer"
                                value={text}
                                disabled={disabled}
                                onChange={(event) => {
                                    setError(null)
                                    setText(event.target.value)
                                }}
                                placeholder="Free-text answer"
                                className="min-w-[12rem] flex-1"
                            />
                            <Button
                                size="sm"
                                variant="outline"
                                disabled={disabled || !text.trim()}
                                onClick={() => void send({ selected_value: text.trim() }, () => setText(''))}
                            >
                                Answer
                            </Button>
                        </div>
                    )}
                </>
            )}
            {error && !question.form ? <InlineError>{error}</InlineError> : null}
        </div>
    )
}

export function DelegationsCard({ active = true }: { active?: boolean }) {
    const dialogs = useDialogController()
    const [filter, setFilter] = useState<Filter>('all')
    const filterRef = useRef<Filter>('all')
    const requestSeqRef = useRef(0)
    const loadingRef = useRef(false)
    // The last rows loaded and the filter they answer: another filter's rows are never shown.
    const [loaded, setLoaded] = useState<{ filter: Filter; records: DelegationOut[] } | null>(null)
    // Answered questions stay hidden until a snapshot of their delegation no longer lists them.
    const [acknowledged, setAcknowledged] = useState<ReadonlySet<string>>(() => new Set())
    const [loading, setLoading] = useState(false)
    const [busy, setBusy] = useState(false)
    const [loadError, setLoadError] = useState<string | null>(null)
    const [cancelError, setCancelError] = useState<string | null>(null)
    const [status, setStatus] = useState('')

    // Loads the selected filter. A response to an older request, or for a filter no longer
    // selected, is dropped; a current failure is thrown for the caller to report.
    const load = useCallback(async () => {
        const requested = filterRef.current
        const seq = ++requestSeqRef.current
        const current = () => seq === requestSeqRef.current && requested === filterRef.current
        loadingRef.current = true
        setLoading(true)
        try {
            const records = await fetchDelegations(FILTER_QUERY[requested])
            if (!current()) return
            setLoaded({ filter: requested, records })
            setLoadError(null)
            setAcknowledged((keys) => {
                const kept = [...keys].filter((key) => {
                    const record = records.find((row) => key.startsWith(`${row.delegation_id}:`))
                    return !record || (record.last_state?.questions ?? []).some((question) => ackKey(record.delegation_id, question.question_id) === key)
                })
                return kept.length === keys.size ? keys : new Set(kept)
            })
        } catch (err) {
            if (current()) throw err
        } finally {
            if (seq === requestSeqRef.current) {
                loadingRef.current = false
                setLoading(false)
            }
        }
    }, [])

    const refresh = useCallback(() => load().catch((err: unknown) => {
        setLoadError(errorText(err, 'Could not load delegations.'))
        setStatus('')
    }), [load])

    // After a change went through, a failed reload is its own error: the change's fresh status stays.
    const reloadAfterChange = () => load().catch((err: unknown) => {
        setLoadError(`Could not refresh delegations: ${errorText(err, 'the request failed.')}`)
    })

    useEffect(() => {
        if (active) void refresh()
    }, [active, filter, refresh])

    const rows = loaded?.filter === filter ? loaded.records : null
    const live = rows?.some((record) => record.phase !== 'sealed') ?? false
    useEffect(() => {
        if (!active || !live) return
        const timer = window.setInterval(() => {
            if (!loadingRef.current) void refresh()
        }, LIVE_REFRESH_MS)
        return () => window.clearInterval(timer)
    }, [active, live, refresh])

    const selectFilter = (choice: Filter) => {
        if (choice === filterRef.current) return
        filterRef.current = choice
        setLoadError(null)
        setFilter(choice)
    }

    const handleCancel = async (record: DelegationOut) => {
        const confirmed = await dialogs.confirm({
            title: `Cancel delegation ${record.request_key}?`,
            description: record.paused
                ? `${record.request_key} is paused, so it will be sealed locally as canceled.`
                : `Spark asks the linked environment to stop ${record.request_key}.`,
            confirmLabel: 'Cancel delegation',
            cancelLabel: 'Keep delegation',
            confirmVariant: 'destructive',
        })
        if (!confirmed) return
        setBusy(true)
        setCancelError(null)
        try {
            const updated = await cancelDelegation(record.delegation_id)
            setStatus(
                updated.phase === 'sealed'
                    ? `Canceled delegation ${record.request_key}.`
                    : `Cancellation requested for ${record.request_key}.`,
            )
        } catch (err) {
            setCancelError(errorText(err, 'Could not cancel delegation.'))
            setStatus('')
            setBusy(false)
            return
        }
        await reloadAfterChange()
        setBusy(false)
    }

    const handleAnswer = async (record: DelegationOut, questionId: string, body: DelegationAnswerBody): Promise<string | null> => {
        setBusy(true)
        try {
            await answerDelegationQuestion(record.delegation_id, questionId, body)
        } catch (err) {
            setStatus('')
            setBusy(false)
            return errorText(err, 'Could not submit answer.')
        }
        setAcknowledged((keys) => new Set(keys).add(ackKey(record.delegation_id, questionId)))
        setStatus(`Sent answer for ${record.request_key}.`)
        await reloadAfterChange()
        setBusy(false)
        return null
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
                                onClick={() => selectFilter(choice)}
                            >
                                {choice === 'all' ? 'All' : choice === 'open' ? 'Open' : 'Paused'}
                            </Button>
                        ))}
                        <Button size="sm" variant="ghost" disabled={loading || busy} onClick={() => void refresh()}>
                            Refresh
                        </Button>
                    </div>
                </div>
            </CardHeader>
            <CardContent className="space-y-3 px-4 text-sm">
                <p className="text-muted-foreground">
                    Tasks handed off from this environment to linked Spark machines. Paused delegations can be canceled locally or resumed by re-linking.
                </p>
                {rows === null ? (
                    loadError ? null : <p role="status" className="text-muted-foreground">Loading delegations…</p>
                ) : rows.length === 0 ? (
                    <p className="text-xs text-muted-foreground">No matching delegations.</p>
                ) : (
                    <ul className="space-y-2">
                        {rows.map((record) => {
                            const badge = delegationStatus(record)
                            const questions = record.phase === 'accepted'
                                ? (record.last_state?.questions ?? []).filter((question) => !acknowledged.has(ackKey(record.delegation_id, question.question_id)))
                                : []
                            // Answers go through the link to the other environment, which a paused
                            // delegation has lost, and a delegation being canceled no longer waits on them.
                            const answerable = !record.paused && !record.cancel_requested_at
                            return (
                                <li key={record.delegation_id}>
                                    <Well role="group" aria-label={`Delegation ${record.request_key}`} className="space-y-2 px-3 py-2">
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
                                                    // A requested cancel can't reach a paused delegation's environment; canceling
                                                    // again seals it locally (spark-workspace delegations::cancel), so it stays offered.
                                                    disabled={busy || (Boolean(record.cancel_requested_at) && !record.paused)}
                                                    onClick={() => void handleCancel(record)}
                                                >
                                                    Cancel delegation
                                                </Button>
                                            ) : null}
                                        </div>
                                        {record.paused ? (
                                            <div className="text-xs text-warning">{record.paused}</div>
                                        ) : null}
                                        {questions.length > 0 && !answerable ? (
                                            <p className="text-xs text-muted-foreground">
                                                {record.paused
                                                    ? 'Answering is unavailable while this delegation is paused or its link is missing.'
                                                    : 'Answering is unavailable while cancellation is pending.'}
                                            </p>
                                        ) : null}
                                        {questions.map((question) => (
                                            <QuestionResponder
                                                key={question.question_id}
                                                draftKey={ackKey(record.delegation_id, question.question_id)}
                                                requestKey={record.request_key}
                                                question={question}
                                                disabled={busy || !answerable}
                                                onAnswer={(body) => handleAnswer(record, question.question_id, body)}
                                            />
                                        ))}
                                    </Well>
                                </li>
                            )
                        })}
                    </ul>
                )}
                {status ? <p role="status" className="text-xs text-muted-foreground">{status}</p> : null}
                {loadError ? <InlineError>{loadError}</InlineError> : null}
                {cancelError ? <InlineError>{cancelError}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
