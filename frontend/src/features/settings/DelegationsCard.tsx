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
import {
    ENVIRONMENT_LINKS_CHANGED_EVENT,
    fetchLocalEnvironmentLinks,
    getActiveEnvironmentId,
    getActiveEnvironmentState,
    useActiveEnvironment,
    type EnvironmentLink,
} from '@/lib/activeEnvironment'

type Filter = 'all' | 'open' | 'paused'

/** A question's unsent answer, whether it is being sent, and a check that kept it from being sent. A form's own fields stay in FormQuestionFields. */
type AnswerDraft = { text: string; answers: Record<string, string>; sending: boolean; error: string | null }

const EMPTY_DRAFT: AnswerDraft = { text: '', answers: {}, sending: false, error: null }

/** A reply the linked environment refused. */
type Failure = { requestKey: string; message: string }

/** A failed cancel. Each failure is its own entry, so a list retires only the one it was requested beside, not a later retry's. */
type CancelError = { message: string }

/**
 * An environment's drafts, the questions answered there, and its refused replies, each by question,
 * its failed cancels by delegation, each shown on its row until a refresh requested since shows it sealed, or canceling
 * with no seal to retry where it showed a cancel to retry,
 * and a reload after a change that failed while no card showed the environment, for the next one to show.
 */
type Answers = {
    drafts: Record<string, AnswerDraft>
    acknowledged: ReadonlySet<string>
    failures: Record<string, Failure>
    cancelErrors: Record<string, CancelError>
    missedReload?: string
}

const NO_ANSWERS: Answers = { drafts: {}, acknowledged: new Set(), failures: {}, cancelErrors: {} }

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
// stalled in an environment left behind doesn't stop live updates in the one shown now. Per environment
// by the owner's decision, after reviews asked both for this and for waiting on every environment's.
const inFlightCounts: Record<string, number> = {}
// The card showing each environment, so a change sent from a card since closed reports, shows its
// updated row and reloads where it is seen, and a reload that fails after Settings was reopened shows there.
type ShownCard = { report: (done: string, updated?: DelegationOut) => Promise<void>; fail: (message: string) => void }
const shownCards = new Map<string, ShownCard>()
// This machine's links as last read: the shared snapshot, or a card's own reread of them sent beside that
// snapshot until it changes. The shared snapshot keeps only its latest read's answer, so the card rereads
// them itself rather than through it: a read that succeeds isn't lost to another that is slow or fails. A
// shared read that lands while the card's is out may have been sent later, so the shared one wins then.
// ponytail: it may also have been sent earlier; the next list that still finds the link missing rereads.
type LinksRead = { links: EnvironmentLink[]; over: EnvironmentLink[] }
const knownLinks = (shared: EnvironmentLink[], read: LinksRead | null) => (read?.over === shared ? read.links : shared)
// The last cancel sent for each delegation, by environment: a failure that a newer try overtook isn't reported.
const latestCancels = new Map<string, symbol>()
const FILTER_QUERY = { all: undefined, open: { open: true }, paused: { paused: true } } as const
// Delegations still in flight are re-read this often, so new questions and phases show up.
const LIVE_REFRESH_MS = 4000

const errorText = (err: unknown, fallback: string) => (err instanceof Error ? err.message : fallback)
// Delegation IDs are UUIDs, so the first ':' ends the delegation part. Child runs may share question IDs,
// so a question is known by its run as well.
const ackKey = (delegationId: string, question: DelegationQuestion) => `${delegationId}:${JSON.stringify([question.run_id ?? '', question.question_id])}`
// Canceling seals locally only a paused delegation its environment never saw (spark-storage seal_out_on);
// one it may have keeps its cancel pending until a link resumes it.
const sealable = (record: DelegationOut) => Boolean(record.paused) && !record.submitted_at
// A delegation can be canceled, or sealed locally with its cancel pending, until it seals.
const cancelable = (record: DelegationOut) => record.phase !== 'sealed' && (!record.cancel_requested_at || sealable(record))
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
            && (record.last_state?.questions ?? []).some((question) => ackKey(record.delegation_id, question) === key)
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
    if ((record.last_state?.questions ?? []).some((question) => !acknowledged.has(ackKey(record.delegation_id, question)))) {
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
    // A child run's gates may share a prompt, so its flow, node and run tell them apart.
    const context = [
        question.flow_name && `flow ${question.flow_name}`,
        question.node_id && `node ${question.node_id}`,
        question.run_id && `run ${question.run_id}`,
        where,
    ].filter(Boolean).join(' · ')

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
        <div role="group" aria-label={`${requestKey}: ${label} (${context})`} className="space-y-2 border-t border-border pt-2">
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
    // Whether a list of every open delegation (All, Open) has loaded since the card last showed a change's result:
    // until one has, the card can't tell none is in flight.
    const [openListed, setOpenListed] = useState(false)
    const { activeEnvironmentId, links: sharedLinks, linksLoaded } = useActiveEnvironment()
    const env = activeEnvironmentId ?? ''
    const { drafts, acknowledged, failures, cancelErrors } = useDelegationAnswers((all) => all[env] ?? NO_ANSWERS)
    // Every delegation's latest record: live refresh falls back on it while a filter hasn't loaded, and
    // the ref lets a cancel confirmation act on what is true when it closes.
    const [seen, setSeen] = useState<Record<string, DelegationOut>>({})
    const seenRef = useRef(seen)
    // The rows this card shows a failed cancel on while System is shown, as shown, with that failure.
    const errorRowsRef = useRef<Record<string, { row: DelegationOut; error: CancelError }>>({})
    // A card left behind when Settings closes neither shows nor prunes with what its requests return.
    const mountedRef = useRef(false)
    useEffect(() => {
        mountedRef.current = true
        return () => { mountedRef.current = false }
    }, [])
    const [loading, setLoading] = useState(false)
    const [busy, setBusy] = useState(false)
    const [loadError, setLoadError] = useState<string | null>(null)
    // The last change's outcome; a failed reload after it stays until the next one.
    const [status, setStatus] = useState<{ text: string; failed?: boolean } | null>(null)
    // This machine's links when one was last added or removed, and the last list request then, until a
    // list requested since loads. The change may concern any delegation, so none is answered until those
    // links reload or that list shows which ones the server paused (spark-storage remove_link).
    const [linkChange, setLinkChange] = useState<{ links: EnvironmentLink[]; seq: number } | null>(null)
    // The card's last reread of this machine's links, and the one out, if any: one at a time, so slow ones
    // don't pile up behind live refreshes, and one sent before the last link change is dropped.
    const [linksRead, setLinksRead] = useState<LinksRead | null>(null)
    const linksReadRef = useRef(linksRead)
    const rereadRef = useRef<object | null>(null)

    // Loads the selected filter. A response to an older request, or for a filter no longer
    // selected, is dropped; a current failure, or any failure of a reload after a change, is thrown
    // for the caller to report.
    const load = useCallback(async (polling = false, afterChange = false) => {
        const requested = filterRef.current
        // Sealing clears a pause (spark-storage seal_out_on), so the Open list holds every paused
        // delegation: Paused's live refresh reads it alone, for its rows and to see one pause or seal.
        const fetched = polling && requested === 'paused' ? 'open' : requested
        const env = activeEnv()
        const seq = ++requestSeqRef.current
        const errorRows = errorRowsRef.current
        const current = () => mountedRef.current && seq === requestSeqRef.current && requested === filterRef.current
        inFlightCounts[env] = (inFlightCounts[env] ?? 0) + 1
        setLoading(true)
        try {
            const records = await fetchDelegations(FILTER_QUERY[fetched])
            if (!current()) return
            setLoaded({ filter: requested, records: fetched === requested ? records : records.filter((row) => inFilter(row, requested)) })
            // A list of every open delegation (All, Open) shows that one seen open but left out has sealed since.
            const coversOpen = fetched !== 'paused'
            if (coversOpen) setOpenListed(true)
            const kept = coversOpen
                ? Object.fromEntries(Object.entries(seenRef.current).filter(([, row]) => row.phase === 'sealed'))
                : seenRef.current
            seenRef.current = { ...kept, ...Object.fromEntries(records.map((row) => [row.delegation_id, row])) }
            setSeen(seenRef.current)
            setLinkChange((change) => (change && change.seq >= seq ? change : null))
            setLoadError(null)
            // Only a failure the card showed when this list was requested retires: not one that arrived since,
            // a retry's included, or before a reopened card's first list.
            updateAnswers(env, ({ cancelErrors }) => ({
                cancelErrors: Object.fromEntries(Object.entries(cancelErrors).filter(([id, error]) => {
                    const record = records.find((row) => row.delegation_id === id)
                    const shown = errorRows[id]
                    return !record || cancelable(record) || shown?.error !== error || (record.phase !== 'sealed' && !cancelable(shown.row))
                })),
            }))
            // A delegation listed as resumed through a link this machine's links lack shows them stale, as
            // when their reload failed after a link was added: each refresh rereads them until they show it.
            const local = getActiveEnvironmentState()
            if (!rereadRef.current && env === '' && local.linksLoaded && records.some((row) => row.phase !== 'sealed' && !row.paused
                && !knownLinks(local.links, linksReadRef.current).some((link) => link.environment_id === row.environment_id))) {
                const reread = {}
                rereadRef.current = reread
                // A failed reread leaves the link shown missing, and the next list that still lacks it rereads.
                const done = (read?: EnvironmentLink[]) => {
                    if (rereadRef.current !== reread) return
                    rereadRef.current = null
                    if (!read) return
                    linksReadRef.current = { links: read, over: local.links }
                    setLinksRead(linksReadRef.current)
                }
                fetchLocalEnvironmentLinks().then(done, () => done())
            }
            forgetUnasked(env, records, coversOpen)
        } catch (err) {
            if (current() || afterChange) throw err
        } finally {
            inFlightCounts[env] -= 1
            if (seq === requestSeqRef.current) setLoading(false)
        }
    }, [])

    const refresh = useCallback((polling = false) => load(polling).catch((err: unknown) => {
        setLoadError(errorText(err, 'Could not load delegations.'))
        setStatus(null)
    }), [load])

    // After a change went through, a failed reload is reported in place of the change's status, beside
    // the rows of whichever filter is selected by then, on the card showing this environment when it
    // fails or, with Settings closed, the next one opened there.
    const reloadAfterChange = useCallback((done: string) => load(false, true).catch((err: unknown) => {
        const message = `${done} Could not refresh delegations: ${errorText(err, 'the request failed.')}`
        const card = shownCards.get(env)
        if (card) card.fail(message)
        else updateAnswers(env, () => ({ missedReload: message }))
    }), [env, load])

    useEffect(() => {
        const report: ShownCard['report'] = async (done, updated) => {
            // The row shows the server's answer now, whether or not the reload below succeeds, unless the
            // card already shows a newer record: one sealed is final. An unsealed answer older than the row has
            // nothing to add: the server records a cancel before it answers, so the row shows it, and the row and
            // anything announced since stay. A sealed one still announces the outcome the sealed row shows. A pause
            // shown from the answer's second stays over an unpaused answer, which may be the older, until a list
            // drops it; a seal, which clears pauses, still ends it.
            // ponytail: updated_at has whole seconds, so another change in the answer's second loses to it until the next refresh.
            const shown = updated && seenRef.current[updated.delegation_id]
            const stale = updated && shown && (shown.phase === 'sealed' || Date.parse(shown.updated_at) > Date.parse(updated.updated_at))
            if (stale && updated.phase !== 'sealed') return
            if (updated && !stale) {
                const next = shown && shown.paused && !updated.paused && updated.phase !== 'sealed'
                    && Date.parse(shown.updated_at) >= Date.parse(updated.updated_at)
                    ? { ...updated, paused: shown.paused }
                    : updated
                seenRef.current = { ...seenRef.current, [next.delegation_id]: next }
                setSeen(seenRef.current)
                // Other delegations may have started since the last All or Open list, so a change shown here stops
                // live refresh only once another such list shows none in flight.
                setOpenListed(false)
                setLoaded((current) => current && {
                    ...current,
                    records: current.records.flatMap((row) => (
                        row.delegation_id !== next.delegation_id ? [row] : inFilter(next, current.filter) ? [next] : []
                    )),
                })
            }
            setStatus({ text: done })
            await reloadAfterChange(done)
        }
        const card: ShownCard = { report, fail: (message) => setStatus({ text: message, failed: true }) }
        shownCards.set(env, card)
        const missed = useDelegationAnswers.getState()[env]?.missedReload
        if (missed) {
            updateAnswers(env, () => ({ missedReload: undefined }))
            card.fail(missed)
        }
        return () => {
            if (shownCards.get(env) === card) shownCards.delete(env)
        }
    }, [env, reloadAfterChange])

    useEffect(() => {
        if (active) void refresh()
    }, [active, filter, refresh])

    // Adding or removing one of this machine's links pauses or resumes the delegations through it. A card
    // on another Settings tab keeps its rows, so it notes the change too, and reloads once shown again.
    useEffect(() => {
        if (activeEnvironmentId !== null) return
        const onLinksChanged = () => {
            rereadRef.current = null
            setLinkChange({ links: knownLinks(getActiveEnvironmentState().links, linksReadRef.current), seq: requestSeqRef.current })
            if (active) void refresh()
        }
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, onLinksChanged)
        return () => window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, onLinksChanged)
    }, [active, activeEnvironmentId, refresh])

    const rows = loaded?.filter === filter ? loaded.records : null
    // Only failures shown on System count. Declared after the effect that refreshes as System is shown, so
    // that refresh, like a reopened card's first, retires only failures shown before it.
    useEffect(() => {
        errorRowsRef.current = active
            ? Object.fromEntries((rows ?? []).flatMap((row) => {
                const error = cancelErrors[row.delegation_id]
                return error ? [[row.delegation_id, { row, error }]] : []
            }))
            : {}
    }, [active, rows, cancelErrors])
    const links = knownLinks(sharedLinks, linksRead)
    const questionsOf = (record: DelegationOut) => (record.phase === 'accepted'
        ? (record.last_state?.questions ?? []).filter((question) => !acknowledged.has(ackKey(record.delegation_id, question)))
        : [])
    // A refused reply shows on its responder, or on the card when its question closed or the filter hides it.
    const shownKeys = new Set(rows?.flatMap((record) => questionsOf(record).map((question) => ackKey(record.delegation_id, question))))
    // The Paused view can't see the open delegations that may pause, so every delegation seen under
    // any filter decides there, as it does until a filter's first load succeeds. A card no All or Open
    // list has loaded for yet can't tell, so it keeps trying.
    const live = !openListed || (filter === 'paused' || rows === null ? Object.values(seen) : rows).some((record) => record.phase !== 'sealed')
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
        // A refresh while the dialog is open can find the delegation paused (sealed locally on cancel if
        // never submitted) or linked again (asked remotely): confirm again until the prompt matches what will happen.
        // ponytail: a pause the server sees between this check and the request still seals locally.
        let asked: string | null = null
        for (;;) {
            const latest = seenRef.current[record.delegation_id] ?? record
            // A cancel already pending needs no second request; one that can seal locally can still be sealed.
            if (latest.cancel_requested_at && !sealable(latest)) {
                setStatus({ text: `Cancellation was already requested for ${record.request_key}.` })
                return
            }
            const description = sealable(latest)
                ? `${record.request_key} is paused, so it will be sealed locally as canceled.`
                : latest.paused
                    ? `${record.request_key} is paused, so its cancellation stays pending until it is linked again.`
                    : `Spark asks the linked environment to stop ${record.request_key}.`
            if (description === asked) break
            asked = description
            const confirmed = await dialogs.confirm({
                title: `Cancel delegation ${record.request_key}?`,
                description,
                confirmLabel: 'Cancel delegation',
                cancelLabel: 'Keep delegation',
                confirmVariant: 'destructive',
            })
            if (!confirmed) return
        }
        setBusy(true)
        const env = activeEnv()
        const attempt = Symbol(record.delegation_id)
        const attemptKey = JSON.stringify([env, record.delegation_id])
        latestCancels.set(attemptKey, attempt)
        updateAnswers(env, ({ cancelErrors: { [record.delegation_id]: _retried, ...cancelErrors } }) => ({ cancelErrors }))
        let updated: DelegationOut
        try {
            updated = await cancelDelegation(record.delegation_id)
        } catch (err) {
            // Kept by environment, so a card reopened meanwhile shows it on the row, sealed or not.
            const message = errorText(err, 'Could not cancel delegation.')
            if (latestCancels.get(attemptKey) === attempt) {
                updateAnswers(env, ({ cancelErrors }) => ({ cancelErrors: { ...cancelErrors, [record.delegation_id]: { message } } }))
            }
            setStatus(null)
            setBusy(false)
            return
        }
        forgetUnasked(env, [updated], false)
        const outcome = delegationStatus(updated, acknowledged).label
        const done = updated.phase !== 'sealed'
            ? `Cancellation requested for ${record.request_key}.`
            : outcome === 'canceled'
                ? `Canceled delegation ${record.request_key}.`
                : `Delegation ${record.request_key} had already ended: ${outcome}.`
        // The card showing this environment, which Settings may have reopened meanwhile, shows the result
        // and reloads; a card since closed never touches the environment shown now. A stalled reload
        // locks no controls.
        setBusy(false)
        void shownCards.get(env)?.report(done, updated)
    }

    const handleAnswer = async (record: DelegationOut, question: DelegationQuestion, body: DelegationAnswerBody) => {
        const env = activeEnv()
        const key = ackKey(record.delegation_id, question)
        editDraft(env, key, { sending: true, error: null })
        setBusy(true)
        try {
            // ponytail: the answer route carries no run ID, so the linked environment answers the first open gate
            // with this ID, which may be another child run's (spark-workspace delegations::answer_incoming). The
            // owner deferred the route fix to a follow-up change.
            await answerDelegationQuestion(record.delegation_id, question.question_id, body)
        } catch (err) {
            // The draft stays to fix and resend. A refresh that dropped the question meanwhile (it closed
            // or was replaced) took its draft: the question stays closed, and the failure shows on the card.
            const failure = { requestKey: record.request_key, message: errorText(err, 'Could not submit answer.') }
            updateAnswers(env, ({ drafts, failures }) => ({
                drafts: Object.hasOwn(drafts, key) ? { ...drafts, [key]: { ...drafts[key], sending: false } } : drafts,
                failures: { ...failures, [key]: failure },
            }))
            setStatus(null)
            setBusy(false)
            return false
        }
        updateAnswers(env, ({ drafts: { [key]: _sent, ...drafts }, acknowledged }) => ({ drafts, acknowledged: new Set(acknowledged).add(key) }))
        // The reply is in, so the caller releases its draft and the card its controls now. The card showing
        // this environment, which Settings may have reopened meanwhile, reports it and reloads; a failed
        // reload is reported on its own.
        setBusy(false)
        void shownCards.get(env)?.report(`Sent answer for ${record.request_key}.`)
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
                    // Only a request still out shows Loading: with none, a failed load or reload says why, and Refresh retries.
                    loading && !loadError ? <p role="status" className="text-muted-foreground">Loading delegations…</p> : null
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
                            // This machine's links show one removed before the delegation's record does.
                            const unlinked = activeEnvironmentId === null && linksLoaded
                                && !links.some((link) => link.environment_id === record.environment_id)
                            const answerable = !record.paused && !record.cancel_requested_at && !unlinked && linkChange?.links !== links
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
                                                    {/* A requested cancel can't reach a paused delegation's environment; canceling
                                                        again seals one it never saw locally (spark-workspace delegations::cancel). */}
                                                    {record.cancel_requested_at && sealable(record) ? (
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
                                        {cancelErrors[record.delegation_id] ? <InlineError>{cancelErrors[record.delegation_id].message}</InlineError> : null}
                                        {questions.length > 0 && !answerable ? (
                                            <p className="text-xs text-muted-foreground">
                                                {unlinked
                                                    ? `Answering is unavailable: the link to ${record.environment_id} is missing.`
                                                    : record.cancel_requested_at
                                                        ? 'Answering is unavailable while cancellation is pending.'
                                                        : record.paused
                                                            ? 'Answering is unavailable while this delegation is paused or its link is missing.'
                                                            : 'Answering is unavailable while a link change is checked.'}
                                            </p>
                                        ) : null}
                                        {questions.map((question) => {
                                            const key = ackKey(record.delegation_id, question)
                                            const draft = drafts[key] ?? EMPTY_DRAFT
                                            return (
                                                <QuestionResponder
                                                    key={key}
                                                    // Linked homes may share delegation and question IDs, so stored form values are kept by environment.
                                                    draftKey={JSON.stringify([env, key])}
                                                    requestKey={record.request_key}
                                                    where={where}
                                                    question={question}
                                                    disabled={busy || draft.sending || !answerable}
                                                    draft={draft}
                                                    error={draft.error ?? failures[key]?.message ?? null}
                                                    onDraft={(patch) => editDraft(env, key, patch)}
                                                    onAnswer={(body) => handleAnswer(record, question, body)}
                                                />
                                            )
                                        })}
                                    </Well>
                                </li>
                            )
                        })}
                    </ul>
                )}
                {status?.failed ? <InlineError>{status.text}</InlineError> : null}
                {status && !status.failed ? <p role="status" className="text-xs text-muted-foreground">{status.text}</p> : null}
                {Object.entries(failures).filter(([key]) => !shownKeys.has(key)).map(([key, failure]) => (
                    <InlineError key={key}>{`Could not answer ${failure.requestKey}: ${failure.message}`}</InlineError>
                ))}
                {loadError ? <InlineError>{loadError}</InlineError> : null}
            </CardContent>
        </Card>
    )
}
