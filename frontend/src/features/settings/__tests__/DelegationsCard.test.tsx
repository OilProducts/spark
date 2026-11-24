import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { DelegationsCard } from '../DelegationsCard'

// Requests go through the real environmentsApi client and parser to a stubbed fetch.
type Reply = () => Response | Promise<Response>
type User = ReturnType<typeof userEvent.setup>

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const ok = (body: unknown): Reply => () => json(body)
const fail = (status: number, detail: string): Reply => () => json({ detail }, status)
function later() {
    let settle!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => { settle = resolve })
    return {
        reply: (() => pending) as Reply,
        resolve: (body: unknown, status = 200) => act(async () => {
            settle(json(body, status))
            await new Promise((resolve) => setTimeout(resolve, 0))
        }),
    }
}

const LIST = 'GET /workspace/api/delegations'
const answerRoute = (delegationId: string, questionId: string) => `POST /workspace/api/delegations/${delegationId}/questions/${questionId}/answer`
const cancelRoute = (delegationId: string) => `POST /workspace/api/delegations/${delegationId}/cancel`

// Each route gives its replies in order, then repeats its last one.
let routes = new Map<string, Reply[]>()
let requests: Array<{ key: string; body: unknown }> = []
const serve = (table: Record<string, Reply[]>) => { routes = new Map(Object.entries(table)) }
const sent = (key: string) => requests.filter((request) => request.key === key).map((request) => request.body)

beforeEach(() => {
    requests = []
    // Only the card's live refresh is faked; Testing Library's waits keep real timeouts.
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://spark.test')
        const key = `${init?.method ?? 'GET'} ${url.pathname}${url.search}`
        requests.push({ key, body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined })
        const replies = routes.get(key)
        if (!replies?.length) return json({ detail: `Unexpected request ${key}` }, 500)
        return (replies.length > 1 ? replies.shift()! : replies[0])()
    }))
})

afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
})

function delegation(id: string, requestKey: string, questions: unknown[], extra: Record<string, unknown> = {}) {
    return {
        delegation_id: id,
        environment_id: 'env-b',
        project_path: '/srv/repo',
        kind: 'flow',
        request_key: requestKey,
        phase: 'accepted',
        target_id: `run-${id}`,
        paused: null,
        cancel_requested_at: null,
        created_at: '2026-11-23T00:00:00Z',
        updated_at: '2026-11-23T00:00:05Z',
        last_state: { phase: 'launched', status: 'waiting', questions },
        result: null,
        ...extra,
    }
}
const list = (...delegations: unknown[]) => ({ delegations })
const approveGate = (extra: Record<string, unknown> = {}) => ({
    question_id: 'q-approve',
    prompt: 'Ship this build?',
    question_type: 'MULTIPLE_CHOICE',
    options: [{ label: 'Approve', value: 'approve' }, { label: 'Reject', value: 'reject' }],
    ...extra,
})
const freeText = (questionId: string, prompt: string) => ({ question_id: questionId, prompt, question_type: 'FREEFORM' })
const deployForm = (questionId: string, message: string) => ({
    question_id: questionId,
    form: {
        request_id: `acp-${questionId}`,
        harness: 'acp:x',
        harness_label: 'X Agent',
        message,
        schema: {
            type: 'object',
            properties: { region: { type: 'string', title: 'Region' }, replicas: { type: 'integer', title: 'Replicas' } },
            required: ['region', 'replicas'],
        },
    },
})
const nightly = delegation('del-1', 'nightly-checks', [approveGate()])
const pausedTask = delegation('del-2', 'paused-task', [], { paused: 'The link to this environment was removed.', last_state: null })
const requested = { cancel_requested_at: '2026-11-23T00:01:00Z' }

// A responder's group name: request key and prompt, then the row's location line.
const asked = (requestKey: string, prompt: string, delegationId: string) => `${requestKey}: ${prompt} (/srv/repo · env env-b · target run-${delegationId})`

function renderCard() {
    const user = userEvent.setup()
    const view = render(<DialogProvider><DelegationsCard /></DialogProvider>)
    return { user, ...view }
}
const settled = () => waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
// Switches to Paused, where only paused-task is listed, while an answer is in flight, lets it fail
// with a 409, and comes back. The responder's row stays out of sight meanwhile.
async function failWhileAway(user: User, answer: ReturnType<typeof later>, detail: string, responderName: string) {
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByRole('group', { name: /^Delegation paused-task/ })).toBeInTheDocument()
    expect(screen.queryByRole('group', { name: responderName })).not.toBeInTheDocument()
    await answer.resolve({ detail }, 409)
    expect(screen.queryByRole('group', { name: responderName })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'All' }))
    return screen.findByRole('group', { name: responderName })
}
const confirmCancel = async (user: User, requestKey: string) => {
    await user.click(await screen.findByRole('button', { name: `Cancel delegation ${requestKey}` }))
    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel delegation' }))
}

it('keeps a free-text draft and its 409 across a filter switch, clears it on a sent retry, and reports a failed refresh separately', async () => {
    const row = delegation('del-1', 'nightly-checks', [freeText('q-note', 'Any release notes?')])
    const answer = later()
    serve({
        [LIST]: [ok(list(row)), ok(list(row)), fail(500, 'database is locked')],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-1', 'q-note')]: [answer.reply, ok({ status: 'ok' })],
    })
    const { user } = renderCard()
    const name = asked('nightly-checks', 'Any release notes?', 'del-1')
    const before = await screen.findByRole('group', { name })
    await user.type(within(before).getByRole('textbox', { name: 'Free-text answer' }), 'ship it')
    await user.click(within(before).getByRole('button', { name: 'Answer' }))

    const responder = await failWhileAway(user, answer, 'Question q-note is not open in delegation del-1.', name)
    expect(within(responder).getByText('Question q-note is not open in delegation del-1.')).toBeInTheDocument()
    expect(within(responder).getByRole('textbox', { name: 'Free-text answer' })).toHaveValue('ship it')
    expect(within(responder).getByRole('textbox', { name: 'Free-text answer' })).toBeEnabled()

    await user.click(within(responder).getByRole('button', { name: 'Answer' }))
    expect(await screen.findByText('Sent answer for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('Sent answer for nightly-checks.')).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()
    expect(screen.queryByText(/is not open in delegation/)).not.toBeInTheDocument()
    expect(sent(answerRoute('del-1', 'q-note'))).toEqual([{ selected_value: 'ship it' }, { selected_value: 'ship it' }])
})

it('keeps conversation answers and their 409 across a filter switch and clears them once a retry is sent', async () => {
    const request = {
        question_id: 'req-multi',
        questions: [
            { id: 'name', header: 'Name', question: 'Release name?', question_type: 'FREEFORM', options: [] },
            { id: 'notes', header: 'Notes', question: 'Release notes?', question_type: 'FREEFORM', options: [] },
        ],
    }
    const answer = later()
    serve({
        // Every snapshot still lists the request, including the stale one read after the answer.
        [LIST]: [ok(list(delegation('del-c', 'chat-task', [request], { kind: 'conversation' })))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-c', 'req-multi')]: [answer.reply, ok({})],
    })
    const { user } = renderCard()
    const name = asked('chat-task', 'Release name? Release notes?', 'del-c')
    const before = await screen.findByRole('group', { name })
    await user.type(within(before).getByRole('textbox', { name: 'Release name?' }), ' v2 ')
    await user.type(within(before).getByRole('textbox', { name: 'Release notes?' }), 'fixes')
    await user.click(within(before).getByRole('button', { name: 'Submit answers' }))

    const responder = await failWhileAway(user, answer, 'Request req-multi is not open.', name)
    expect(within(responder).getByText('Request req-multi is not open.')).toBeInTheDocument()
    expect(within(responder).getByRole('textbox', { name: 'Release name?' })).toHaveValue(' v2 ')
    expect(within(responder).getByRole('textbox', { name: 'Release notes?' })).toHaveValue('fixes')

    await user.click(within(responder).getByRole('button', { name: 'Submit answers' }))
    expect(await screen.findByText('Sent answer for chat-task.')).toBeInTheDocument()
    await settled()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()
    expect(sent(answerRoute('del-c', 'req-multi'))).toEqual([
        { answers: { name: 'v2', notes: 'fixes' } },
        { answers: { name: 'v2', notes: 'fixes' } },
    ])
})

it('validates a form with FormQuestionFields, keeps it and its 409 across a filter switch and sends a content map', async () => {
    const answer = later()
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [deployForm('form-retry', 'Deployment details')])))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-f', 'form-retry')]: [answer.reply, ok({})],
    })
    const { user } = renderCard()
    const name = asked('deploy-task', 'Deployment details', 'del-f')
    const before = await screen.findByRole('group', { name })
    await user.click(within(before).getByRole('button', { name: 'Submit' }))
    expect(await within(before).findByText('Replicas is required.')).toBeInTheDocument()
    expect(sent(answerRoute('del-f', 'form-retry'))).toEqual([])

    await user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '3')
    await user.click(within(before).getByRole('button', { name: 'Submit' }))

    const responder = await failWhileAway(user, answer, 'The form was already answered elsewhere.', name)
    expect(within(responder).getByText('The form was already answered elsewhere.')).toBeInTheDocument()
    expect(within(responder).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')
    expect(within(responder).getByRole('textbox', { name: 'Replicas' })).toHaveValue('3')

    await user.click(within(responder).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()
    await settled()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()
    const accept = { form: { action: 'accept', content: { region: 'eu-west', replicas: 3 } } }
    expect(sent(answerRoute('del-f', 'form-retry'))).toEqual([accept, accept])
})

it('keeps the exact form draft when the user leaves and returns before a refused accept', async () => {
    const answer = later()
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [deployForm('form-exact', 'Exact details')])))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-f', 'form-exact')]: [answer.reply],
    })
    const { user } = renderCard()
    const name = asked('deploy-task', 'Exact details', 'del-f')
    const before = await screen.findByRole('group', { name })
    await user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    // Past f64's exact integers: only the typed text keeps every digit.
    await user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '12345678901234567890')
    await user.click(within(before).getByRole('button', { name: 'Submit' }))

    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByRole('group', { name: /^Delegation paused-task/ })).toBeInTheDocument()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'All' }))
    const responder = await screen.findByRole('group', { name })
    await answer.resolve({ detail: 'The form was already answered elsewhere.' }, 409)

    expect(within(responder).getByText('The form was already answered elsewhere.')).toBeInTheDocument()
    expect(within(responder).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')
    expect(within(responder).getByRole('textbox', { name: 'Replicas' })).toHaveValue('12345678901234567890')
    expect(within(responder).getByRole('textbox', { name: 'Replicas' })).toBeEnabled()
})

it('keeps the form draft when a Decline or a Cancel is refused, across a filter switch', async () => {
    const cancel = later()
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [deployForm('form-keep', 'Keep details')])))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-f', 'form-keep')]: [fail(409, 'Decline refused.'), cancel.reply],
    })
    const { user } = renderCard()
    const name = asked('deploy-task', 'Keep details', 'del-f')
    const form = await screen.findByRole('group', { name })
    await user.type(within(form).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await user.click(within(form).getByRole('button', { name: 'Decline' }))
    expect(await within(form).findByText('Decline refused.')).toBeInTheDocument()
    expect(within(form).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')

    await user.click(within(form).getByRole('button', { name: 'Cancel' }))
    const responder = await failWhileAway(user, cancel, 'Cancel refused.', name)
    expect(within(responder).getByText('Cancel refused.')).toBeInTheDocument()
    expect(within(responder).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')
    expect(sent(answerRoute('del-f', 'form-keep'))).toEqual([{ form: { action: 'decline' } }, { form: { action: 'cancel' } }])
})

it('sends a form Cancel and Decline as their actions', async () => {
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [deployForm('form-a', 'First form'), deployForm('form-b', 'Second form')])))],
        [answerRoute('del-f', 'form-a')]: [ok({})],
        [answerRoute('del-f', 'form-b')]: [ok({})],
    })
    const { user } = renderCard()
    await user.click(within(await screen.findByRole('group', { name: asked('deploy-task', 'First form', 'del-f') })).getByRole('button', { name: 'Cancel' }))
    await settled()
    await user.click(within(screen.getByRole('group', { name: asked('deploy-task', 'Second form', 'del-f') })).getByRole('button', { name: 'Decline' }))
    await settled()
    expect(sent(answerRoute('del-f', 'form-a'))).toEqual([{ form: { action: 'cancel' } }])
    expect(sent(answerRoute('del-f', 'form-b'))).toEqual([{ form: { action: 'decline' } }])
})

it('hides an answered question across a stale snapshot and picks up a new one on the live refresh', async () => {
    const stale = list(nightly)
    serve({
        [LIST]: [ok(stale), ok(stale), ok(list(delegation('del-1', 'nightly-checks', [freeText('q-next', 'Anything else?')])))],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
    })
    const { user, rerender } = renderCard()
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    await user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    await settled()
    expect(sent(LIST)).toHaveLength(2)
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByRole('group', { name: asked('nightly-checks', 'Anything else?', 'del-1') })).toBeInTheDocument()

    // A tick while a refresh is still in flight is skipped.
    const pending = later()
    routes.set(LIST, [pending.reply])
    await act(async () => { vi.advanceTimersByTime(4000) })
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(LIST)).toHaveLength(4)
    await pending.resolve(list(nightly))

    rerender(<DialogProvider><DelegationsCard active={false} /></DialogProvider>)
    expect(vi.getTimerCount()).toBe(0)
})

it('keeps an empty Paused view polling so a delegation that loses its link shows up', async () => {
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [ok(list()), ok(list({ ...nightly, paused: 'No link to this environment; link it again to resume.' }))],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    expect(screen.getByText('nightly-checks')).toBeInTheDocument()
})

it('disables answering on a delegation whose link is missing', async () => {
    serve({
        [LIST]: [ok(list(delegation('del-2', 'paused-task', [approveGate(), freeText('q-why', 'Why?')], { paused: 'The link to this environment was removed.' })))],
    })
    renderCard()
    expect(await screen.findByText('Answering is unavailable while this delegation is paused or its link is missing.')).toBeInTheDocument()
    expect(screen.getByText('The link to this environment was removed.')).toBeInTheDocument()
    expect(screen.getByText('flow · paused')).toBeInTheDocument()
    expect(within(screen.getByRole('group', { name: asked('paused-task', 'Ship this build?', 'del-2') })).getByRole('button', { name: 'Approve' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name: asked('paused-task', 'Why?', 'del-2') })).getByRole('textbox', { name: 'Free-text answer' })).toBeDisabled()
})

it('shows a pending cancellation over stale questions and a pause, and disables a duplicate Cancel', async () => {
    serve({ [LIST]: [ok(list(delegation('del-1', 'nightly-checks', [approveGate()], requested), { ...pausedTask, ...requested }))] })
    renderCard()
    expect(await screen.findAllByText('flow · canceling')).toHaveLength(2)
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Cancel delegation paused-task' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })).toBeDisabled()
    expect(screen.getByText('Answering is unavailable while cancellation is pending.')).toBeInTheDocument()
})

it('offers a confirmed local seal once a delegation with a pending cancel loses its link', async () => {
    const canceling = delegation('del-1', 'nightly-checks', [approveGate()], requested)
    const unlinked = { ...canceling, paused: 'No link to this environment; link it again to resume.' }
    const sealed = { ...unlinked, phase: 'sealed', result: { status: 'canceled' } }
    serve({
        [LIST]: [ok(list(canceling)), ok(list(unlinked)), ok(list(sealed))],
        [cancelRoute('del-1')]: [ok(sealed)],
    })
    const { user } = renderCard()
    expect(await screen.findByText('flow · canceling')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Seal delegation nightly-checks as canceled' })).not.toBeInTheDocument()

    // The link goes away while the cancel is still pending: the live refresh sees it paused.
    await act(async () => { vi.advanceTimersByTime(4000) })
    const seal = await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' })
    expect(screen.getByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    expect(screen.getByText('flow · canceling')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })).toBeDisabled()

    await user.click(seal)
    const dialog = screen.getByRole('alertdialog')
    expect(within(dialog).getByText('nightly-checks is paused, so it will be sealed locally as canceled.')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Cancel delegation' }))
    expect(await screen.findByText('flow · canceled')).toBeInTheDocument()
    expect(screen.getByText('Canceled delegation nightly-checks.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Seal delegation nightly-checks as canceled' })).not.toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toHaveLength(1)
})

it('drops an Open response that resolves after Paused is selected', async () => {
    const open = later()
    serve({
        [LIST]: [ok(list(nightly, pausedTask))],
        [`${LIST}?open=true`]: [open.reply],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()

    await open.resolve(list(nightly))
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    expect(screen.getByText('paused-task')).toBeInTheDocument()
})

it('refreshes a cancel started under All with the filter selected when it completes', async () => {
    const cancel = later()
    serve({
        [LIST]: [ok(list(nightly, pausedTask))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const { user } = renderCard()
    await confirmCancel(user, 'nightly-checks')
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()

    await cancel.resolve({ ...nightly, ...requested })
    expect(await screen.findByText('Cancellation requested for nightly-checks.')).toBeInTheDocument()
    await settled()
    expect(requests.at(-1)?.key).toBe(`${LIST}?paused=true`)
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    expect(screen.getByText('paused-task')).toBeInTheDocument()
})

it('applies a cancel result to its row even when the reload after it fails', async () => {
    serve({
        [LIST]: [ok(list(nightly)), fail(500, 'database is locked')],
        [cancelRoute('del-1')]: [ok({ ...nightly, ...requested })],
    })
    const { user } = renderCard()
    await confirmCancel(user, 'nightly-checks')
    expect(await screen.findByText('Cancellation requested for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('Cancellation requested for nightly-checks.')).not.toBeInTheDocument()
    expect(screen.getByText('flow · canceling')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })).toBeDisabled()
    expect(sent(cancelRoute('del-1'))).toHaveLength(1)
})

it('reports the outcome of a delegation that had already completed when canceled', async () => {
    const completed = { ...nightly, phase: 'sealed', result: { status: 'completed' } }
    serve({
        [LIST]: [ok(list(nightly)), ok(list(completed))],
        [cancelRoute('del-1')]: [ok(completed)],
    })
    const { user } = renderCard()
    await confirmCancel(user, 'nightly-checks')
    expect(await screen.findByText('Delegation nightly-checks had already ended: completed.')).toBeInTheDocument()
    expect(screen.getByText('flow · completed')).toBeInTheDocument()
    expect(screen.queryByText(/Canceled delegation/)).not.toBeInTheDocument()
})

it('masks secret sub-questions, keeps typed custom answers, submits chosen option values and blocks a blank submit', async () => {
    const request = {
        question_id: 'req-env',
        questions: [
            { id: 'env', question: 'Which environment?', question_type: 'MULTIPLE_CHOICE', options: [{ label: 'Staging' }, { label: 'Production', value: 'prod' }] },
            { id: 'region', question: 'Which region?', question_type: 'MULTIPLE_CHOICE', options: [{ label: 'eu' }], isOther: true },
            { id: 'token', question: 'Deploy token?', question_type: 'FREEFORM', isSecret: true },
        ],
    }
    serve({
        [LIST]: [ok(list(delegation('del-c', 'chat-task', [request], { kind: 'conversation' })))],
        [answerRoute('del-c', 'req-env')]: [ok({})],
    })
    const { user } = renderCard()
    const responder = await screen.findByRole('group', { name: asked('chat-task', 'Which environment? Which region? Deploy token?', 'del-c') })
    await user.click(within(responder).getByRole('button', { name: 'Submit answers' }))
    expect(within(responder).getByText('Answer "Which environment?" before submitting.')).toBeInTheDocument()
    expect(sent(answerRoute('del-c', 'req-env'))).toEqual([])

    expect(within(responder).queryByRole('textbox', { name: 'Which environment?' })).not.toBeInTheDocument()
    const token = within(responder).getByLabelText('Deploy token?')
    expect(token).toHaveAttribute('type', 'password')
    expect(token).toHaveAttribute('autocomplete', 'off')

    await user.click(within(responder).getByRole('button', { name: 'Production' }))
    expect(within(responder).getByRole('button', { name: 'Production' })).toHaveAttribute('aria-pressed', 'true')

    // A click fills in the choice; typing past a choice's text keeps every character.
    const region = within(responder).getByRole('textbox', { name: 'Which region?' })
    await user.click(within(responder).getByRole('button', { name: 'eu' }))
    expect(region).toHaveValue('eu')
    expect(within(responder).getByRole('button', { name: 'eu' })).toHaveAttribute('aria-pressed', 'true')
    await user.type(region, '-west')
    expect(region).toHaveValue('eu-west')
    expect(within(responder).getByRole('button', { name: 'eu' })).toHaveAttribute('aria-pressed', 'false')
    await user.clear(region)
    await user.type(region, 'eu-west')
    expect(region).toHaveValue('eu-west')

    await user.type(token, ' s3cret ')
    await user.click(within(responder).getByRole('button', { name: 'Submit answers' }))
    await settled()
    expect(sent(answerRoute('del-c', 'req-env'))).toEqual([{ answers: { env: 'prod', region: 'eu-west', token: 's3cret' } }])
})

it('offers no free text on a restricted run gate but keeps it for an agent clarification', async () => {
    serve({
        [LIST]: [ok(list(
            nightly,
            delegation('del-3', 'clarify-task', [{
                question_id: 'q-clarify',
                prompt: 'Which branch?',
                origin: 'agent_clarification',
                question_type: 'MULTIPLE_CHOICE',
                options: [{ label: 'main' }, { label: 'release' }],
            }]),
        ))],
    })
    renderCard()
    const gate = await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })
    expect(within(gate).getByRole('button', { name: 'Approve' })).toBeInTheDocument()
    expect(within(gate).queryByRole('textbox')).not.toBeInTheDocument()
    const clarification = screen.getByRole('group', { name: asked('clarify-task', 'Which branch?', 'del-3') })
    expect(within(clarification).getByRole('button', { name: 'main' })).toBeInTheDocument()
    expect(within(clarification).getByRole('textbox', { name: 'Free-text answer' })).toBeInTheDocument()
})

it('tells apart Approve buttons of delegations sharing a request key and prompt', async () => {
    serve({
        [LIST]: [ok(list(nightly, delegation('del-4', 'nightly-checks', [approveGate()])))],
        [answerRoute('del-4', 'q-approve')]: [ok({})],
    })
    const { user } = renderCard()
    expect(await screen.findAllByRole('button', { name: 'Approve' })).toHaveLength(2)
    expect(screen.getByRole('group', { name: 'Delegation nightly-checks (/srv/repo · env env-b · target run-del-1)' })).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Delegation nightly-checks (/srv/repo · env env-b · target run-del-4)' })).toBeInTheDocument()

    await user.click(within(screen.getByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-4') })).getByRole('button', { name: 'Approve' }))
    await settled()
    expect(sent(answerRoute('del-4', 'q-approve'))).toEqual([{ selected_value: 'approve' }])
    expect(sent(answerRoute('del-1', 'q-approve'))).toEqual([])
})

it('asks before canceling: dismissing sends nothing, confirming sends one request and updates the row', async () => {
    const sealed = { ...pausedTask, phase: 'sealed', result: { status: 'canceled' } }
    serve({
        [LIST]: [ok(list(pausedTask)), ok(list(sealed))],
        [cancelRoute('del-2')]: [ok(sealed)],
    })
    const { user } = renderCard()
    await user.click(await screen.findByRole('button', { name: 'Cancel delegation paused-task' }))
    const dialog = screen.getByRole('alertdialog')
    expect(within(dialog).getByText('Cancel delegation paused-task?')).toBeInTheDocument()
    expect(within(dialog).getByText('paused-task is paused, so it will be sealed locally as canceled.')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Keep delegation' }))
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(sent(cancelRoute('del-2'))).toEqual([])

    await confirmCancel(user, 'paused-task')
    expect(await screen.findByText('flow · canceled')).toBeInTheDocument()
    expect(screen.getByText('Canceled delegation paused-task.')).toBeInTheDocument()
    expect(sent(cancelRoute('del-2'))).toHaveLength(1)
})

it('shows only the error when the first load for a filter fails, clearing the prior status', async () => {
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [fail(500, 'database is locked')],
        [cancelRoute('del-1')]: [ok({ ...nightly, ...requested })],
    })
    const { user } = renderCard()
    await confirmCancel(user, 'nightly-checks')
    expect(await screen.findByText('Cancellation requested for nightly-checks.')).toBeInTheDocument()
    await settled()

    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('No matching delegations.')).not.toBeInTheDocument()
    expect(screen.queryByText('Loading delegations…')).not.toBeInTheDocument()
    expect(screen.queryByText('Cancellation requested for nightly-checks.')).not.toBeInTheDocument()
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()

    // Choosing the selected filter again keeps the error rather than an endless "Loading".
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(screen.getByText('Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('Loading delegations…')).not.toBeInTheDocument()
})
