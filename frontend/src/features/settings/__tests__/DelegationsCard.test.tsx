import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { DelegationsCard, useDelegationAnswers } from '../DelegationsCard'

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
    useDelegationAnswers.setState({ drafts: {}, acknowledged: new Set() })
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
// A conversation request for one secret answer.
const tokenRequest = (questionId: string) => ({
    question_id: questionId,
    questions: [{ id: 'token', question: 'Deploy token?', question_type: 'FREEFORM', isSecret: true }],
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

it('keeps the exact form draft when the user switches filters and returns before a refused accept', async () => {
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

// Leaving Settings unmounts the card; coming back mounts a new one.
it('keeps a free-text answer in flight, then its draft and 409, when Settings is left and reopened during the reply', async () => {
    const row = delegation('del-1', 'nightly-checks', [freeText('q-note', 'Any release notes?')])
    const answer = later()
    serve({
        [LIST]: [ok(list(row))],
        [answerRoute('del-1', 'q-note')]: [answer.reply, ok({})],
    })
    const first = renderCard()
    const name = asked('nightly-checks', 'Any release notes?', 'del-1')
    const before = await screen.findByRole('group', { name })
    await first.user.type(within(before).getByRole('textbox', { name: 'Free-text answer' }), 'ship it')
    await first.user.click(within(before).getByRole('button', { name: 'Answer' }))
    first.unmount()

    const { user } = renderCard()
    const responder = await screen.findByRole('group', { name })
    const text = within(responder).getByRole('textbox', { name: 'Free-text answer' })
    expect(text).toHaveValue('ship it')
    expect(text).toBeDisabled()
    await answer.resolve({ detail: 'Question q-note is not open in delegation del-1.' }, 409)
    expect(within(responder).getByText('Question q-note is not open in delegation del-1.')).toBeInTheDocument()
    expect(text).toHaveValue('ship it')
    expect(text).toBeEnabled()

    await user.click(within(responder).getByRole('button', { name: 'Answer' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    expect(sent(answerRoute('del-1', 'q-note'))).toEqual([{ selected_value: 'ship it' }, { selected_value: 'ship it' }])
})

it('keeps the exact form draft beside a refused Submit, Decline or Cancel when Settings is left during the reply, and clears it once sent', async () => {
    const form = deployForm('form-away', 'Away details')
    const replies = [later(), later(), later()]
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [form])))],
        [answerRoute('del-f', 'form-away')]: [...replies.map((reply) => reply.reply), ok({})],
    })
    const name = asked('deploy-task', 'Away details', 'del-f')
    let view = renderCard()
    let responder = await screen.findByRole('group', { name })
    await view.user.type(within(responder).getByRole('textbox', { name: 'Region' }), 'eu-west')
    // Past f64's exact integers: only the typed text keeps every digit.
    await view.user.type(within(responder).getByRole('textbox', { name: 'Replicas' }), '12345678901234567890')
    for (const [index, action] of (['Submit', 'Decline', 'Cancel'] as const).entries()) {
        await view.user.click(within(responder).getByRole('button', { name: action }))
        view.unmount()
        view = renderCard()
        responder = await screen.findByRole('group', { name })
        expect(within(responder).getByRole('button', { name: 'Submitting...' })).toBeDisabled()
        await replies[index].resolve({ detail: `${action} refused.` }, 409)
        expect(within(responder).getByText(`${action} refused.`)).toBeInTheDocument()
        expect(within(responder).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')
        expect(within(responder).getByRole('textbox', { name: 'Replicas' })).toHaveValue('12345678901234567890')
        expect(within(responder).getByRole('textbox', { name: 'Replicas' })).toBeEnabled()
    }
    const [submit, decline, cancel] = sent(answerRoute('del-f', 'form-away'))
    expect(submit).toMatchObject({ form: { action: 'accept', content: { region: 'eu-west' } } })
    expect([decline, cancel]).toEqual([{ form: { action: 'decline' } }, { form: { action: 'cancel' } }])

    // Once a reply is accepted, the question asked again starts from an empty form.
    await view.user.click(within(responder).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()
    await settled()
    routes.set(LIST, [ok(list(delegation('del-f', 'deploy-task', []))), ok(list(delegation('del-f', 'deploy-task', [form])))])
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(6))
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' })).toHaveValue('')
})

it('keeps an unsent draft for a newer question when a refresh from before Settings was left returns late', async () => {
    const late = later()
    const newer = delegation('del-1', 'nightly-checks', [approveGate(), freeText('q-next', 'Anything else?')])
    serve({ [LIST]: [ok(list(nightly)), late.reply, ok(list(newer))] })
    const first = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    // A live refresh is in flight when Settings closes.
    await act(async () => { vi.advanceTimersByTime(4000) })
    first.unmount()

    const { user } = renderCard()
    const name = asked('nightly-checks', 'Anything else?', 'del-1')
    const next = await screen.findByRole('group', { name })
    await user.type(within(next).getByRole('textbox', { name: 'Free-text answer' }), 'later note')
    // The old card's snapshot predates the newer question.
    await late.resolve(list(nightly))
    expect(within(screen.getByRole('group', { name })).getByRole('textbox', { name: 'Free-text answer' })).toHaveValue('later note')
})

it('keeps an answered question hidden when Settings is reopened before the follower drops it, and sends it once', async () => {
    const answer = later()
    serve({
        // The follower's snapshot still lists the gate after the answer went through.
        [LIST]: [ok(list(nightly))],
        [answerRoute('del-1', 'q-approve')]: [answer.reply],
    })
    const first = renderCard()
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    await first.user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    first.unmount()

    const second = renderCard()
    expect(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' })).toBeDisabled()
    await answer.resolve({})
    await waitFor(() => expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument())
    second.unmount()

    renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await settled()
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
    expect(sent(answerRoute('del-1', 'q-approve'))).toEqual([{ selected_value: 'approve' }])
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

it('sends a form field named "__proto__" in the content map', async () => {
    const odd = {
        question_id: 'form-proto',
        form: {
            request_id: 'acp-proto',
            harness: 'acp:x',
            harness_label: 'X Agent',
            message: 'Odd field',
            // Parsed, so "__proto__" is the schema's own property name, as it is off the wire.
            schema: JSON.parse('{"type":"object","properties":{"__proto__":{"type":"string","title":"Prototype"}},"required":["__proto__"]}'),
        },
    }
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [odd])))],
        [answerRoute('del-f', 'form-proto')]: [ok({})],
    })
    const { user } = renderCard()
    const form = await screen.findByRole('group', { name: asked('deploy-task', 'Odd field', 'del-f') })
    await user.type(within(form).getByRole('textbox', { name: 'Prototype' }), 'kept')
    await user.click(within(form).getByRole('button', { name: 'Submit' }))
    await settled()
    expect(sent(answerRoute('del-f', 'form-proto')).map((body) => JSON.stringify(body)))
        .toEqual(['{"form":{"action":"accept","content":{"__proto__":"kept"}}}'])
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

it('asks again before sealing locally when a delegation pauses while its remote cancel is being confirmed', async () => {
    const unlinked = { ...nightly, paused: 'No link to this environment; link it again to resume.' }
    const sealed = { ...unlinked, phase: 'sealed', result: { status: 'canceled' } }
    serve({
        [LIST]: [ok(list(nightly)), ok(list(unlinked)), ok(list(sealed))],
        [cancelRoute('del-1')]: [ok(sealed)],
    })
    const { user } = renderCard()
    await user.click(await screen.findByRole('button', { name: 'Cancel delegation nightly-checks' }))
    const remote = screen.getByRole('alertdialog')
    expect(within(remote).getByText('Spark asks the linked environment to stop nightly-checks.')).toBeInTheDocument()

    // The live refresh finds the link gone while the dialog is open.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    await user.click(within(remote).getByRole('button', { name: 'Cancel delegation' }))
    expect(await screen.findByText('nightly-checks is paused, so it will be sealed locally as canceled.')).toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toEqual([])

    await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel delegation' }))
    expect(await screen.findByText('Canceled delegation nightly-checks.')).toBeInTheDocument()
    expect(screen.getByText('flow · canceled')).toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toHaveLength(1)
})

it('sends no second cancel when a refresh shows a remote cancel already pending, and still offers the local seal once paused', async () => {
    const canceling = { ...nightly, ...requested }
    const unlinked = { ...canceling, paused: 'No link to this environment; link it again to resume.' }
    const sealed = { ...unlinked, phase: 'sealed', result: { status: 'canceled' } }
    serve({
        [LIST]: [ok(list(nightly)), ok(list(canceling)), ok(list(unlinked)), ok(list(sealed))],
        [cancelRoute('del-1')]: [ok(sealed)],
    })
    const { user } = renderCard()
    await user.click(await screen.findByRole('button', { name: 'Cancel delegation nightly-checks' }))
    const remote = screen.getByRole('alertdialog')
    // Another session's cancel shows up while the dialog is open.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('flow · canceling')).toBeInTheDocument()
    await user.click(within(remote).getByRole('button', { name: 'Cancel delegation' }))
    expect(await screen.findByText('Cancellation was already requested for nightly-checks.')).toBeInTheDocument()
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toEqual([])

    await act(async () => { vi.advanceTimersByTime(4000) })
    await user.click(await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' }))
    const local = screen.getByRole('alertdialog')
    expect(within(local).getByText('nightly-checks is paused, so it will be sealed locally as canceled.')).toBeInTheDocument()
    await user.click(within(local).getByRole('button', { name: 'Cancel delegation' }))
    expect(await screen.findByText('Canceled delegation nightly-checks.')).toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toHaveLength(1)
})

it('keeps polling with the error shown when Open\'s first load fails after an empty Paused list, while All showed a delegation in flight', async () => {
    const retry = later()
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [ok(list())],
        [`${LIST}?open=true`]: [fail(500, 'database is locked'), retry.reply],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(`${LIST}?open=true`)).toHaveLength(2)
    expect(screen.getByText('Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('Loading delegations…')).not.toBeInTheDocument()

    await retry.resolve(list(nightly))
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    expect(screen.queryByText('Database is locked')).not.toBeInTheDocument()
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

it('shows a failed cancel on its row, only where the row is listed, until a refresh shows it canceling', async () => {
    serve({
        [LIST]: [ok(list(nightly, pausedTask)), ok(list(nightly, pausedTask)), ok(list({ ...nightly, ...requested }, pausedTask))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [cancelRoute('del-1')]: [fail(409, 'delegation is busy')],
    })
    const { user } = renderCard()
    const row = () => screen.findByRole('group', { name: /^Delegation nightly-checks/ })
    await confirmCancel(user, 'nightly-checks')
    expect(await within(await row()).findByText('Delegation is busy')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    expect(screen.queryByText('Delegation is busy')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'All' }))
    expect(await within(await row()).findByText('Delegation is busy')).toBeInTheDocument()

    // Another session's cancel went through: the refresh that shows it retires the error.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('flow · canceling')).toBeInTheDocument()
    expect(screen.queryByText('Delegation is busy')).not.toBeInTheDocument()
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

it('treats an unanswered sub-question with the ID "constructor" as missing', async () => {
    const request = {
        question_id: 'req-build',
        questions: [{ id: 'constructor', question: 'Which builder?', question_type: 'FREEFORM', options: [] }],
    }
    serve({
        [LIST]: [ok(list(delegation('del-c', 'chat-task', [request], { kind: 'conversation' })))],
        [answerRoute('del-c', 'req-build')]: [ok({})],
    })
    const { user } = renderCard()
    const responder = await screen.findByRole('group', { name: asked('chat-task', 'Which builder?', 'del-c') })
    const builder = within(responder).getByRole('textbox', { name: 'Which builder?' })
    expect(builder).toHaveValue('')
    await user.click(within(responder).getByRole('button', { name: 'Submit answers' }))
    expect(within(responder).getByText('Answer "Which builder?" before submitting.')).toBeInTheDocument()
    expect(sent(answerRoute('del-c', 'req-build'))).toEqual([])

    await user.type(builder, 'docker')
    await user.click(within(responder).getByRole('button', { name: 'Submit answers' }))
    await settled()
    expect(sent(answerRoute('del-c', 'req-build'))).toEqual([{ answers: { constructor: 'docker' } }])
})

it('releases unsent drafts, secrets included, once a delegation seals, keeping those still open to a retry', async () => {
    const first = delegation('del-1', 'nightly-checks', [tokenRequest('req-1')], { kind: 'conversation' })
    const second = delegation('del-2', 'paused-task', [tokenRequest('req-2')], { kind: 'conversation' })
    const unlinked = { ...second, paused: 'No link to this environment; link it again to resume.' }
    serve({
        // The sealed delegation's last snapshot still lists its request.
        [LIST]: [ok(list(first, second)), ok(list({ ...first, phase: 'sealed', result: { status: 'completed' } }, unlinked)), fail(500, 'database is locked')],
        [cancelRoute('del-2')]: [ok({ ...unlinked, phase: 'sealed', result: { status: 'canceled' } })],
    })
    const { user } = renderCard()
    const token = async (requestKey: string, id: string) => within(await screen.findByRole('group', { name: asked(requestKey, 'Deploy token?', id) })).getByLabelText('Deploy token?')
    await user.type(await token('nightly-checks', 'del-1'), 's3cret-1')
    await user.type(await token('paused-task', 'del-2'), 's3cret-2')
    expect(Object.keys(useDelegationAnswers.getState().drafts)).toEqual(['del-1:req-1', 'del-2:req-2'])

    // del-1 seals; del-2 only pauses, so its draft waits for the link to come back.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('conversation · completed')).toBeInTheDocument()
    expect(Object.keys(useDelegationAnswers.getState().drafts)).toEqual(['del-2:req-2'])
    expect(await token('paused-task', 'del-2')).toHaveValue('s3cret-2')

    // Sealing it locally releases its draft even though the reload after it fails.
    await confirmCancel(user, 'paused-task')
    expect(await screen.findByText('Canceled delegation paused-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(useDelegationAnswers.getState().drafts).toEqual({})
})

it('releases the drafts and acknowledgements of a delegation an Open refresh leaves out, and keeps those a Paused list leaves out', async () => {
    const first = delegation('del-1', 'nightly-checks', [approveGate(), tokenRequest('req-1')])
    const second = delegation('del-5', 'release-task', [tokenRequest('req-5')])
    serve({
        [LIST]: [ok(list(first, second))],
        // The reload after the answer still lists the gate; then another tab seals del-1.
        [`${LIST}?open=true`]: [ok(list(first, second)), ok(list(first, second)), ok(list(second))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
    })
    const { user } = renderCard()
    await user.click(await screen.findByRole('button', { name: 'Open' }))
    const token = async (requestKey: string, id: string) => within(await screen.findByRole('group', { name: asked(requestKey, 'Deploy token?', id) })).getByLabelText('Deploy token?')
    await user.type(await token('nightly-checks', 'del-1'), 's3cret-1')
    await user.type(await token('release-task', 'del-5'), 's3cret-5')
    await user.click(within(screen.getByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    await settled()
    expect([...useDelegationAnswers.getState().acknowledged]).toEqual(['del-1:q-approve'])

    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument())
    expect(Object.keys(useDelegationAnswers.getState().drafts)).toEqual(['del-5:req-5'])
    expect(useDelegationAnswers.getState().acknowledged.size).toBe(0)

    // Paused leaves del-5 out without saying it ended: its draft stays for when Open shows it again.
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    expect(Object.keys(useDelegationAnswers.getState().drafts)).toEqual(['del-5:req-5'])
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(await token('release-task', 'del-5')).toHaveValue('s3cret-5')
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
