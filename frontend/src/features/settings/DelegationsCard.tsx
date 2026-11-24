import { useCallback, useEffect, useRef, useState } from 'react'
import { create } from 'zustand'
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
import { getActiveEnvironmentId, useActiveEnvironment } from '@/lib/activeEnvironment'

type Filter = 'all' | 'open' | 'paused'

/** A question's unsent answer, whether it is being sent, and a check that kept it from being sent. A form's own fields stay in FormQuestionFields. */
type AnswerDraft = { text: string; answers: Record<string, string>; sending: boolean; error: string | null }

const EMPTY_DRAFT: AnswerDraft = { text: '', answers: {}, sending: false, error: null }

/** A reply the linked environment refused. */
type Failure = { requestKey: string; message: string }

/** An environment's drafts, the questions answered there, and its refused replies, each by question. */
type Answers = { drafts: Record<string, AnswerDraft>; acknowledged: ReadonlySet<string>; failures: Record<string, Failure> }

const NO_ANSWERS: Answers = { drafts: {}, acknowledged: new Set(), failures: {} }

// Answers outlive the card, so leaving Settings and coming back keeps unsent drafts, a reply still
// in flight and its failure, and which questions were answered (hidden until a snapshot drops them).
// They are kept by environment ('' is this machine): one environment's lists never prune another's.
// A refused reply stays until its answer is edited or sent again, so one whose question closed stays on the card.
// ponytail: module-level like the other unsent answers; lost on reload. Add a dismiss if closed questions' failures pile up.
export const useDelegationAnswers = create<Record<string, Answers>>(() => ({}))
const updateAnswers = (env: string, update: (answers: Answers) => Partial<Answers>) => useDelegationAnswers.setState((all) => {
    const answers = all[env] ?? NO_ANSWERS
    return { [env]: { ...answers, ...update(answers) } }
})
// Editing or resending an answer retires its refused reply.
const editDraft = (env: string, key: string, patch: Partial<AnswerDraft>) => updateAnswers(env, ({ drafts, failures: { [key]: _retired, ...failures } }) => ({
    drafts: { ...drafts, [key]: { ...(drafts[key] ?? EMPTY_DRAFT), ...patch } },
    failures,
}))
// The environment a request goes to, read when it is sent.
const activeEnv = () => getActiveEnvironmentId() ?? ''
// List requests still out, by environment, superseded ones and those of a card since closed included:
// a live refresh waits for those of the environment shown, never for another environment's, so one
// stalled in an environment left behind doesn't stop live updates in the one shown now.
const inFlightCounts: Record<string, number> = {}
// The card showing each environment, so a reply sent from a card since closed reports and reloads where it is seen.
const shownCards = new Map<string, (done: string) => Promise<void>>()
const FILTER_QUERY = { all: undefined, open: { open: true }, paused: { paused: true } } as const
// Delegations still in flight are re-read this often, so new questions and phases show up.
const LIVE_REFRESH_MS = 4000

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)
// Delegation IDs are UUIDs, so the first ':' ends the delegation part.
const ackKey = (delegationId: string, questionId: string) => `${delegationId}:${questionId}`
// Mirrors the server's list filter (spark-workspace delegations::list).
const inFilter = (record: DelegationOut, filter: Filter) => (
    filter === 'all' || (filter === 'open' ? record.phase !== 'sealed' : record.paused !== null)
)
// The row's visible location line; its target tells apart delegations sharing a request key.
const whereText = (record: DelegationOut) => [
    record.project_path,
    `env ${record.environment_id}`,
    record.target_id ? `target ${record.target_id}` : null,
].filter(Boolean).join(' · ')

// A sub-question's answer so far; an agent's ID such as "constructor" is never read off the prototype.
const answerOf = (answers: Record<string, string>, id: string) => (Object.hasOwn(answers, id) ? answers[id] : '')

// Drops the drafts (secrets included) and acknowledgements of questions the environment's records no
// longer ask: a listed delegation that sealed, though its last snapshot may still list them, or that
// dropped them, and, when the records cover every open delegation (All, Open), one they leave out. A
// delegation that only a Paused list or a single record leaves out keeps its keys, still open to a retry.
const forgetUnasked = (env: string, records: DelegationOut[], coversOpen: boolean) => {
    const asked = (key: string) => {
        const record = records.find((row) => key.startsWith(`${row.delegation_id}:`))
        if (!record) return !coversOpen
        return record.phase !== 'sealed'
            && (record.last_state?.questions ?? []).some((question) => ackKey(record.delegation_id, question.question_id) === key)
    }
    updateAnswers(env, ({ drafts, acknowledged }) => ({
        drafts: Object.fromEntries(Object.entries(drafts).filter(([key]) => asked(key))),
        acknowledged: [...acknowledged].every(asked) ? acknowledged : new Set([...acknowledged].filter(asked)),
    }))
}

function delegationStatus(record: DelegationOut, acknowledged: ReadonlySet<string>): { label: string; tone: StatusTone } {
    if (record.phase === 'sealed') {
        const resultStatus = typeof record.result?.status === 'string' ? record.result.status : 'sealed'
        if (resultStatus === 'completed' || resultStatus === 'complete') return { label: resultStatus, tone: 'success' }
        if (resultStatus === 'canceled') return { label: 'canceled', tone: 'neutral' }
        return { label: resultStatus, tone: 'destructive' }
    }
    // A requested cancellation outranks a pause and questions the last snapshot still lists.
    if (record.cancel_requested_at) return { label: 'canceling', tone: 'neutral' }
    if (record.paused) return { label: 'paused', tone: 'warning' }
    // A question answered here waits on nothing, though a snapshot may still list it.
    if ((record.last_state?.questions ?? []).some((question) => !acknowledged.has(ackKey(record.delegation_id, question.question_id)))) {
        return { label: 'waiting on answer', tone: 'warning' }
    }
    if (record.last_state?.status) return { label: record.last_state.status, tone: 'info' }
    return { label: record.phase, tone: 'info' }
}

function QuestionResponder({
    draftKey,
    requestKey,
    where,
    question,
    disabled,
    draft,
    error,
    onDraft,
    onAnswer,
}: {
    draftKey: string
    requestKey: string
    where: string
    question: DelegationQuestion
    disabled: boolean
    draft: AnswerDraft
    error: string | null
    onDraft: (patch: Partial<AnswerDraft>) => void
    onAnswer: (body: DelegationAnswerBody) => Promise<boolean>
}) {
    const label = question.prompt || question.form?.message || question.questions?.map((sub) => sub.question).join(' ') || 'Question'

    const submitAnswers = (subs: NonNullable<DelegationQuestion['questions']>) => {
        const trimmed = Object.fromEntries(subs.map((sub) => [sub.id, answerOf(draft.answers, sub.id).trim()]))
        const missing = subs.find((sub) => !trimmed[sub.id])
        if (missing) {
            onDraft({ error: `Answer "${missing.question}" before submitting.` })
            return
        }
        onAnswer({ answers: trimmed })
    }

    const options = question.options ?? []
    // A run gate with declared choices takes only one of them (attractor-api's answer check).
    const restricted = options.length > 0 && question.origin !== 'agent_clarification' && question.question_type !== 'FREEFORM'

    return (
        <div role="group" aria-label={`${requestKey}: ${label} (${where})`} className="space-y-2 border-t border-border pt-2">
            {/* FormQuestionFields keeps its stored draft until the reply is accepted. */}
            {question.form ? (
                <FormQuestionFields
                    actionError={error}
                    draftKey={`delegation:${draftKey}`}
                    form={question.form}
                    isSubmitting={disabled}
                    testId={`delegation-form-${question.question_id}`}
                    onReply={(reply) => onAnswer({ form: reply })}
                />
            ) : question.questions ? (
                <>
                    {question.questions.map((sub) => {
                        const value = answerOf(draft.answers, sub.id)
                        const subOptions = sub.options ?? []
                        // The field holds the answer as is: a click fills in its choice, typing keeps every character.
                        const setAnswer = (next: string) => onDraft({ answers: { ...draft.answers, [sub.id]: next }, error: null })
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
                                        value={value}
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
                                        onClick={() => onAnswer({ selected_value: value })}
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
                                value={draft.text}
                                disabled={disabled}
                                onChange={(event) => onDraft({ text: event.target.value, error: null })}
                                placeholder="Free-text answer"
                                className="min-w-[12rem] flex-1"
                            />
                            <Button
                                size="sm"
                                variant="outline"
                                disabled={disabled || !draft.text.trim()}
                                onClick={() => onAnswer({ selected_value: draft.text.trim() })}
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
    // The last rows loaded and the filter they answer: another filter's rows are never shown.
    const [loaded, setLoaded] = useState<{ filter: Filter; records: DelegationOut[] } | null>(null)
    const env = useActiveEnvironment().activeEnvironmentId ?? ''
    const { drafts, acknowledged, failures } = useDelegationAnswers((all) => all[env] ?? NO_ANSWERS)
    // Every delegation's latest record: live refresh falls back on it while a filter hasn't loaded, and
    // the ref lets a cancel confirmation act on what is true when it closes.
    const [seen, setSeen] = useState<Record<string, DelegationOut>>({})
    const seenRef = useRef(seen)
    // A card left behind when Settings closes neither shows nor prunes with what its requests return.
    const mountedRef = useRef(false)
    useEffect(() => {
        mountedRef.current = true
        return () => { mountedRef.current = false }
    }, [])
    const [loading, setLoading] = useState(false)
    const [busy, setBusy] = useState(false)
    const [loadError, setLoadError] = useState<string | null>(null)
    // A failed cancel's error, by delegation, shown on its row until a refresh shows it canceling or sealed.
    const [cancelErrors, setCancelErrors] = useState<Record<string, string>>({})
    const [status, setStatus] = useState('')

    // Loads the selected filter. A response to an older request, or for a filter no longer
    // selected, is dropped; a current failure is thrown for the caller to report.
    const load = useCallback(async (polling = false) => {
        const requested = filterRef.current
        // Sealing clears a pause (spark-storage seal_out_on), so the Open list holds every paused
        // delegation: Paused's live refresh reads it alone, for its rows and to see one pause or seal.
        const fetched = polling && requested === 'paused' ? 'open' : requested
        const env = activeEnv()
        const seq = ++requestSeqRef.current
        const current = () => mountedRef.current && seq === requestSeqRef.current && requested === filterRef.current
        inFlightCounts[env] = (inFlightCounts[env] ?? 0) + 1
        setLoading(true)
        try {
            const records = await fetchDelegations(FILTER_QUERY[fetched])
            if (!current()) return
            setLoaded({ filter: requested, records: fetched === requested ? records : records.filter((row) => inFilter(row, requested)) })
            // A list of every open delegation (All, Open) shows that one seen open but left out has sealed since.
            const coversOpen = fetched !== 'paused'
            const kept = coversOpen
                ? Object.fromEntries(Object.entries(seenRef.current).filter(([, row]) => row.phase === 'sealed'))
                : seenRef.current
            seenRef.current = { ...kept, ...Object.fromEntries(records.map((row) => [row.delegation_id, row])) }
            setSeen(seenRef.current)
            setLoadError(null)
            setCancelErrors((errors) => Object.fromEntries(Object.entries(errors).filter(([id]) => {
                const record = records.find((row) => row.delegation_id === id)
                return !record || (!record.cancel_requested_at && record.phase !== 'sealed')
            })))
            forgetUnasked(env, records, coversOpen)
        } catch (err) {
            if (current()) throw err
        } finally {
            inFlightCounts[env] -= 1
            if (seq === requestSeqRef.current) setLoading(false)
        }
    }, [])

    const refresh = useCallback((polling = false) => load(polling).catch((err: unknown) => {
        setLoadError(errorText(err, 'Could not load delegations.'))
        setStatus('')
    }), [load])

    // After a change went through, a failed reload is reported in place of the change's status.
    const reloadAfterChange = useCallback((done: string) => load().catch((err: unknown) => {
        setLoadError(`${done} Could not refresh delegations: ${errorText(err, 'the request failed.')}`)
        setStatus('')
    }), [load])

    useEffect(() => {
        const report = async (done: string) => {
            setStatus(done)
            await reloadAfterChange(done)
        }
        shownCards.set(env, report)
        return () => {
            if (shownCards.get(env) === report) shownCards.delete(env)
        }
    }, [env, reloadAfterChange])

    useEffect(() => {
        if (active) void refresh()
    }, [active, filter, refresh])

    const rows = loaded?.filter === filter ? loaded.records : null
    const questionsOf = (record: DelegationOut) => (record.phase === 'accepted'
        ? (record.last_state?.questions ?? []).filter((question) => !acknowledged.has(ackKey(record.delegation_id, question.question_id)))
        : [])
    // A refused reply shows on its responder, or on the card when its question closed or the filter hides it.
    const shownKeys = new Set(rows?.flatMap((record) => questionsOf(record).map((question) => ackKey(record.delegation_id, question.question_id))))
    // The Paused view can't see the open delegations that may pause, so every delegation seen under
    // any filter decides there, as it does until a filter's first load succeeds.
    const live = (filter === 'paused' || rows === null ? Object.values(seen) : rows).some((record) => record.phase !== 'sealed')
    useEffect(() => {
        if (!active || !live) return
        const timer = window.setInterval(() => {
            if (!inFlightCounts[activeEnv()]) void refresh(true)
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
        // A refresh while the dialog is open can find the delegation paused (sealed locally on cancel)
        // or linked again (asked remotely): confirm again until the prompt matches what will happen.
        // ponytail: a pause the server sees between this check and the request still seals locally.
        let paused: boolean | null = null
        for (;;) {
            const latest = seenRef.current[record.delegation_id] ?? record
            // A remote cancel already pending needs no second request; a paused one can still be sealed locally.
            if (latest.cancel_requested_at && !latest.paused) {
                setStatus(`Cancellation was already requested for ${record.request_key}.`)
                return
            }
            if ((latest.paused !== null) === paused) break
            paused = latest.paused !== null
            const confirmed = await dialogs.confirm({
                title: `Cancel delegation ${record.request_key}?`,
                description: paused
                    ? `${record.request_key} is paused, so it will be sealed locally as canceled.`
                    : `Spark asks the linked environment to stop ${record.request_key}.`,
                confirmLabel: 'Cancel delegation',
                cancelLabel: 'Keep delegation',
                confirmVariant: 'destructive',
            })
            if (!confirmed) return
        }
        setBusy(true)
        setCancelErrors(({ [record.delegation_id]: _retried, ...errors }) => errors)
        const env = activeEnv()
        let updated: DelegationOut
        try {
            updated = await cancelDelegation(record.delegation_id)
        } catch (err) {
            setCancelErrors((errors) => ({ ...errors, [record.delegation_id]: errorText(err, 'Could not cancel delegation.') }))
            setStatus('')
            setBusy(false)
            return
        }
        // The row shows the server's answer now, whether or not the reload below succeeds.
        seenRef.current = { ...seenRef.current, [updated.delegation_id]: updated }
        setSeen(seenRef.current)
        forgetUnasked(env, [updated], false)
        setLoaded((current) => current && {
            ...current,
            records: current.records.flatMap((row) => (
                row.delegation_id !== updated.delegation_id ? [row] : inFilter(updated, current.filter) ? [updated] : []
            )),
        })
        const outcome = delegationStatus(updated, acknowledged).label
        const done = updated.phase !== 'sealed'
            ? `Cancellation requested for ${record.request_key}.`
            : outcome === 'canceled'
                ? `Canceled delegation ${record.request_key}.`
                : `Delegation ${record.request_key} had already ended: ${outcome}.`
        // The card showing this environment, if any, reports it and reloads: one Settings left meanwhile
        // never sends a list request to the environment shown now.
        await shownCards.get(env)?.(done)
        setBusy(false)
    }

    const handleAnswer = async (record: DelegationOut, questionId: string, body: DelegationAnswerBody) => {
        const env = activeEnv()
        const key = ackKey(record.delegation_id, questionId)
        editDraft(env, key, { sending: true, error: null })
        setBusy(true)
        try {
            await answerDelegationQuestion(record.delegation_id, questionId, body)
        } catch (err) {
            // The draft stays to fix and resend. A refresh that dropped the question meanwhile (it closed
            // or was replaced) took its draft: the question stays closed, and the failure shows on the card.
            const failure = { requestKey: record.request_key, message: errorText(err, 'Could not submit answer.') }
            updateAnswers(env, ({ drafts, failures }) => ({
                drafts: Object.hasOwn(drafts, key) ? { ...drafts, [key]: { ...drafts[key], sending: false } } : drafts,
                failures: { ...failures, [key]: failure },
            }))
            setStatus('')
            setBusy(false)
            return false
        }
        updateAnswers(env, ({ drafts: { [key]: _sent, ...drafts }, acknowledged }) => ({ drafts, acknowledged: new Set(acknowledged).add(key) }))
        // The reply is in, so the caller releases its draft now. The card showing this environment, which
        // Settings may have reopened meanwhile, reports it and reloads; a failed reload is reported on its own.
        void shownCards.get(env)?.(`Sent answer for ${record.request_key}.`).then(() => setBusy(false))
        return true
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
                            const badge = delegationStatus(record, acknowledged)
                            const where = whereText(record)
                            const questions = questionsOf(record)
                            // Answers go through the link to the other environment, which a paused
                            // delegation has lost, and a delegation being canceled no longer waits on them.
                            const answerable = !record.paused && !record.cancel_requested_at
                            return (
                                <li key={record.delegation_id}>
                                    <Well role="group" aria-label={`Delegation ${record.request_key} (${where})`} className="space-y-2 px-3 py-2">
                                        <div className="flex flex-wrap items-start justify-between gap-2">
                                            <div className="min-w-0 space-y-0.5">
                                                <div className="flex flex-wrap items-center gap-2 font-medium text-foreground">
                                                    <StatusDot tone={badge.tone} />
                                                    <span>{record.request_key}</span>
                                                    <span className="text-xs font-normal text-muted-foreground">
                                                        {record.kind} · {badge.label}
                                                    </span>
                                                </div>
                                                <div className="break-all text-xs text-muted-foreground">{where}</div>
                                            </div>
                                            {record.phase !== 'sealed' ? (
                                                <div className="flex flex-wrap gap-1">
                                                    {/* A requested cancel can't reach a paused delegation's environment;
                                                        canceling it again seals it locally (spark-workspace delegations::cancel). */}
                                                    {record.cancel_requested_at && record.paused ? (
                                                        <Button
                                                            size="sm"
                                                            variant="ghost"
                                                            aria-label={`Seal delegation ${record.request_key} as canceled`}
                                                            disabled={busy}
                                                            onClick={() => void handleCancel(record)}
                                                        >
                                                            Seal as canceled
                                                        </Button>
                                                    ) : null}
                                                    <Button
                                                        size="sm"
                                                        variant="ghost"
                                                        aria-label={`Cancel delegation ${record.request_key}`}
                                                        disabled={busy || Boolean(record.cancel_requested_at)}
                                                        onClick={() => void handleCancel(record)}
                                                    >
                                                        Cancel delegation
                                                    </Button>
                                                </div>
                                            ) : null}
                                        </div>
                                        {record.paused ? (
                                            <div className="text-xs text-warning">{record.paused}</div>
                                        ) : null}
                                        {cancelErrors[record.delegation_id] ? <InlineError>{cancelErrors[record.delegation_id]}</InlineError> : null}
                                        {questions.length > 0 && !answerable ? (
                                            <p className="text-xs text-muted-foreground">
                                                {record.cancel_requested_at
                                                    ? 'Answering is unavailable while cancellation is pending.'
                                                    : 'Answering is unavailable while this delegation is paused or its link is missing.'}
                                            </p>
                                        ) : null}
                                        {questions.map((question) => {
                                            const key = ackKey(record.delegation_id, question.question_id)
                                            const draft = drafts[key] ?? EMPTY_DRAFT
                                            return (
                                                <QuestionResponder
                                                    key={question.question_id}
                                                    // Linked homes may share delegation and question IDs, so stored form values are kept by environment.
                                                    draftKey={JSON.stringify([env, key])}
                                                    requestKey={record.request_key}
                                                    where={where}
                                                    question={question}
                                                    disabled={busy || draft.sending || !answerable}
                                                    draft={draft}
                                                    error={draft.error ?? failures[key]?.message ?? null}
                                                    onDraft={(patch) => editDraft(env, key, patch)}
                                                    onAnswer={(body) => handleAnswer(record, question.question_id, body)}
                                                />
                                            )
                                        })}
                                    </Well>
                                </li>
                            )
                        })}
                    </ul>
                )}
                {status ? <p role="status" className="text-xs text-muted-foreground">{status}</p> : null}
                {Object.entries(failures).filter(([key]) => !shownKeys.has(key)).map(([key, failure]) => (
                    <InlineError key={key}>{`Could not answer ${failure.requestKey}: ${failure.message}`}</InlineError>
                ))}
                {loadError ? <InlineError>{loadError}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
