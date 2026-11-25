import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { notifyEnvironmentLinksChanged, refreshLocalEnvironments, resetActiveEnvironmentForTests, selectEnvironment } from '@/lib/activeEnvironment'
import { DelegationsCard, useDelegationAnswers } from '../DelegationsCard'

// Requests go through the real environmentsApi client and parser to a stubbed fetch.
type Reply = () => Response | Promise<Response>
type User = ReturnType<typeof userEvent.setup>

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })
const ok = (body: unknown): Reply => () => json(body)
const fail = (status: number, detail: string): Reply => () => json({ detail }, status)
// Ends the replies a test left pending: the card counts list requests in flight across cards, so
// one still out would hold off the next test's live refresh.
let endPending: Array<() => void> = []
function later() {
    let settle!: (response: Response) => void
    const pending = new Promise<Response>((resolve) => { settle = resolve })
    endPending.push(() => settle(json({ detail: 'The test ended.' }, 503)))
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
// This machine's answers ('' is the local environment).
const localAnswers = () => useDelegationAnswers.getState()['']

beforeEach(() => {
    requests = []
    useDelegationAnswers.setState({}, true)
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

afterEach(async () => {
    await act(async () => {
        endPending.forEach((end) => end())
        await new Promise((resolve) => setTimeout(resolve, 0))
    })
    endPending = []
    vi.clearAllTimers()
    vi.useRealTimers()
    vi.unstubAllGlobals()
    resetActiveEnvironmentForTests()
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
// env-x is a linked environment; its requests go through its proxy.
const REMOTE = '/workspace/api/environments/links/env-x/proxy'
const descriptor = (id: string) => ok({ environment_id: id, display_name: id, protocol_version: 1, ui_api_version: 1 })
const ENVIRONMENTS = { 'GET /environment': [descriptor('env-local')], [`GET ${REMOTE}/environment`]: [descriptor('env-x')] }
const switchTo = (environmentId: string | null) => act(async () => { await selectEnvironment(environmentId) })
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

    // Once a reply is accepted, its draft is released though the reload after it fails, which is
    // reported on its own, and the question asked again starts from an empty form.
    routes.set(LIST, [fail(500, 'database is locked'), ok(list(delegation('del-f', 'deploy-task', []))), ok(list(delegation('del-f', 'deploy-task', [form])))])
    await view.user.click(within(responder).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    await settled()
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(6))
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' })).toHaveValue('')
})

it('releases an accepted form draft as soon as the reply is in, while the reload after it is still out', async () => {
    const form = deployForm('form-done', 'Done details')
    const reload = later()
    serve({
        // Open shows the form answered; All then asks it again.
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [form]))), reload.reply, ok(list(delegation('del-f', 'deploy-task', [form])))],
        [`${LIST}?open=true`]: [ok(list(delegation('del-f', 'deploy-task', [])))],
        [answerRoute('del-f', 'form-done')]: [ok({})],
    })
    const { user } = renderCard()
    const name = asked('deploy-task', 'Done details', 'del-f')
    const before = await screen.findByRole('group', { name })
    await user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '3')
    await user.click(within(before).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(await screen.findByText('flow · waiting')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'All' }))
    expect(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' })).toHaveValue('')
    await reload.resolve(list(delegation('del-f', 'deploy-task', [form])))
})

it('reports a failed reload after a sent form once Paused is selected, beside Paused\'s rows, with the draft released', async () => {
    const form = deployForm('form-done', 'Done details')
    const reload = later()
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [form]))), reload.reply],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-f', 'form-done')]: [ok({})],
    })
    const { user } = renderCard()
    const before = await screen.findByRole('group', { name: asked('deploy-task', 'Done details', 'del-f') })
    await user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '3')
    await user.click(within(before).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    await reload.resolve({ detail: 'database is locked' }, 500)
    expect(await screen.findByText('Sent answer for deploy-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.queryByText('Sent answer for deploy-task.')).not.toBeInTheDocument()
    expect(screen.getByText('paused-task')).toBeInTheDocument()
    expect(screen.queryByText('deploy-task')).not.toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})
    expect(localAnswers().acknowledged).toEqual(new Set(['del-f:form-done']))
    expect(sent(answerRoute('del-f', 'form-done'))).toHaveLength(1)
})

it('shows a reload that fails after Settings was reopened on the reopened card, with the sent form\'s draft released', async () => {
    const form = deployForm('form-done', 'Done details')
    const reload = later()
    serve({
        [LIST]: [ok(list(delegation('del-f', 'deploy-task', [form]))), reload.reply, ok(list(delegation('del-f', 'deploy-task', [form])))],
        [answerRoute('del-f', 'form-done')]: [ok({})],
    })
    const first = renderCard()
    const name = asked('deploy-task', 'Done details', 'del-f')
    const before = await screen.findByRole('group', { name })
    await first.user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await first.user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '3')
    await first.user.click(within(before).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})
    // Settings is left and reopened while the reload after the answer is still out.
    first.unmount()

    renderCard()
    expect(await screen.findByText('deploy-task')).toBeInTheDocument()
    await settled()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()
    await reload.resolve({ detail: 'database is locked' }, 500)
    expect(await screen.findByText('Sent answer for deploy-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.getByText('deploy-task')).toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})
    expect(localAnswers().acknowledged).toEqual(new Set(['del-f:form-done']))
    expect(sent(answerRoute('del-f', 'form-done'))).toHaveLength(1)
})

it('shows a failed reload after an answer that supersedes a reopened card\'s first load in place of "Loading", and keeps polling until a list loads', async () => {
    const answer = later()
    const first = later()
    serve({
        [LIST]: [ok(list(nightly)), first.reply, fail(500, 'database is locked'), ok(list(nightly))],
        [answerRoute('del-1', 'q-approve')]: [answer.reply],
    })
    const view = renderCard()
    await view.user.click(within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' }))
    view.unmount()

    renderCard()
    expect(await screen.findByText('Loading delegations…')).toBeInTheDocument()
    await answer.resolve({})
    expect(await screen.findByText('Sent answer for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    await first.resolve(list(nightly))
    expect(screen.queryByText('Loading delegations…')).not.toBeInTheDocument()
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    expect(sent(LIST)).toHaveLength(4)
})

it('keeps a Local reload that fails after the user opened a linked environment\'s Settings off that card', async () => {
    const reload = later()
    const remoteList = `GET ${REMOTE}/workspace/api/delegations`
    serve({
        ...ENVIRONMENTS,
        [LIST]: [ok(list(nightly)), reload.reply],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
        [remoteList]: [ok(list(delegation('del-9', 'remote-task', [])))],
    })
    const first = renderCard()
    await first.user.click(within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    first.unmount()
    await switchTo('env-x')
    renderCard()
    expect(await screen.findByText('remote-task')).toBeInTheDocument()
    await settled()

    await reload.resolve({ detail: 'database is locked' }, 500)
    expect(screen.queryByText(/Could not refresh delegations/)).not.toBeInTheDocument()
    expect(screen.getByText('remote-task')).toBeInTheDocument()
})

it('keeps other delegations usable after an accepted reply while the reload after it stalls and another filter loads', async () => {
    const reload = later()
    const other = delegation('del-3', 'release-task', [approveGate()])
    serve({
        [LIST]: [ok(list(nightly, other)), reload.reply],
        [`${LIST}?open=true`]: [ok(list(nightly, other))],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
    })
    const { user } = renderCard()
    await user.click(within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})

    await user.click(screen.getByRole('button', { name: 'Open' }))
    await settled()
    expect(within(screen.getByRole('group', { name: asked('release-task', 'Ship this build?', 'del-3') })).getByRole('button', { name: 'Approve' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Cancel delegation release-task' })).toBeEnabled()
    expect(screen.queryByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).not.toBeInTheDocument()
})

it('keeps other delegations usable after a cancel while the reload after it stalls and another filter loads', async () => {
    const reload = later()
    const other = delegation('del-3', 'release-task', [])
    serve({
        [LIST]: [ok(list(nightly, other)), reload.reply],
        [`${LIST}?open=true`]: [ok(list({ ...nightly, ...requested }, other))],
        [cancelRoute('del-1')]: [ok({ ...nightly, ...requested })],
    })
    const { user } = renderCard()
    await confirmCancel(user, 'nightly-checks')
    expect(await screen.findByText('Cancellation requested for nightly-checks.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Open' }))
    await settled()
    expect(screen.getByRole('button', { name: 'Cancel delegation release-task' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeDisabled()
})

it('shows a reload that failed while Settings was closed when Settings next opens in that environment, and not in another', async () => {
    const reload = later()
    const remoteList = `GET ${REMOTE}/workspace/api/delegations`
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    serve({
        ...ENVIRONMENTS,
        [LIST]: [ok(list(nightly)), reload.reply, ok(list(nightly))],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
        [remoteList]: [ok(list(delegation('del-9', 'remote-task', [])))],
    })
    const first = renderCard()
    await first.user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    first.unmount()
    await reload.resolve({ detail: 'database is locked' }, 500)

    await switchTo('env-x')
    const remote = renderCard()
    expect(await screen.findByText('remote-task')).toBeInTheDocument()
    await settled()
    expect(screen.queryByText(/Could not refresh delegations/)).not.toBeInTheDocument()
    remote.unmount()

    await switchTo(null)
    renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await settled()
    expect(screen.getByText('Sent answer for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})
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
    // The new card's live refresh waits for the old card's request.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(LIST)).toHaveLength(3)
    // The old card's snapshot predates the newer question.
    await late.resolve(list(nightly))
    expect(within(screen.getByRole('group', { name })).getByRole('textbox', { name: 'Free-text answer' })).toHaveValue('later note')
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(4))
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
    // The stale snapshot still lists the answered gate, so the row shows the follower's status.
    expect(screen.getByText('flow · waiting')).toBeInTheDocument()

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

it('keeps an empty Paused view polling so a delegation that loses its link shows up, without reading the full history', async () => {
    const unlinked = { ...nightly, paused: 'No link to this environment; link it again to resume.' }
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [ok(list())],
        // Paused's live refresh reads the Open list alone and keeps the paused ones.
        [`${LIST}?open=true`]: [ok(list(unlinked, delegation('del-3', 'release-task', [])))],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    expect(screen.getByText('nightly-checks')).toBeInTheDocument()
    expect(screen.queryByText('release-task')).not.toBeInTheDocument()
    expect(sent(LIST)).toHaveLength(1)
    expect(sent(`${LIST}?paused=true`)).toHaveLength(1)
    expect(sent(`${LIST}?open=true`)).toHaveLength(1)
})

it('sends one request per Paused live refresh, skips ticks while it is out, and keeps its failure shown until one succeeds', async () => {
    const poll = later()
    const retry = later()
    const unlinked = { ...nightly, paused: 'No link to this environment; link it again to resume.' }
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [ok(list())],
        [`${LIST}?open=true`]: [poll.reply, retry.reply],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()
    await settled()

    await act(async () => { vi.advanceTimersByTime(4000) })
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(`${LIST}?open=true`)).toHaveLength(1)
    await poll.resolve({ detail: 'database is locked' }, 500)
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(`${LIST}?open=true`)).toHaveLength(2)
    expect(screen.getByText('Database is locked')).toBeInTheDocument()
    await retry.resolve(list(unlinked))
    expect(await screen.findByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    expect(screen.queryByText('Database is locked')).not.toBeInTheDocument()
    expect(sent(LIST)).toHaveLength(1)
    expect(sent(`${LIST}?paused=true`)).toHaveLength(1)
})

it('stops polling Paused once the last active delegation seals while it stays selected', async () => {
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?paused=true`]: [ok(list())],
        [`${LIST}?open=true`]: [ok(list())],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()
    expect(vi.getTimerCount()).toBe(1)

    await act(async () => { vi.advanceTimersByTime(4000) })
    await settled()
    expect(sent(`${LIST}?open=true`)).toHaveLength(1)
    expect(sent(LIST)).toHaveLength(1)
    expect(vi.getTimerCount()).toBe(0)
    expect(screen.getByText('No matching delegations.')).toBeInTheDocument()
})

it('keeps live refreshes going in a healthy environment while a closed card\'s list request in another is stalled', async () => {
    const late = later()
    const remoteList = `GET ${REMOTE}/workspace/api/delegations`
    serve({
        ...ENVIRONMENTS,
        [LIST]: [ok(list(nightly)), late.reply],
        [remoteList]: [ok(list(delegation('del-9', 'remote-task', [])))],
    })
    const first = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    // A live refresh is stalled when the user switches environments and opens System settings there.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(LIST)).toHaveLength(2)
    first.unmount()
    await switchTo('env-x')
    renderCard()
    expect(await screen.findByText('remote-task')).toBeInTheDocument()
    await settled()

    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(remoteList)).toHaveLength(2))
    expect(sent(LIST)).toHaveLength(2)
})

it('stops polling an empty Paused view once Open shows every delegation seen in flight has sealed', async () => {
    serve({
        [LIST]: [ok(list(nightly))],
        [`${LIST}?open=true`]: [ok(list())],
        [`${LIST}?paused=true`]: [ok(list())],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    expect(vi.getTimerCount()).toBe(1)
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    await waitFor(() => expect(sent(`${LIST}?paused=true`)).toHaveLength(1))
    expect(await screen.findByText('No matching delegations.')).toBeInTheDocument()
    await settled()
    expect(vi.getTimerCount()).toBe(0)
})

it('reports a reply that fails after a refresh replaced its question, and keeps that question closed', async () => {
    const answer = later()
    serve({
        [LIST]: [ok(list(nightly)), ok(list(delegation('del-1', 'nightly-checks', [freeText('q-next', 'Anything else?')])))],
        [answerRoute('del-1', 'q-approve')]: [answer.reply],
    })
    const { user } = renderCard()
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    await user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    // The live refresh finds the gate closed and a new question asked while the reply is out.
    await act(async () => { vi.advanceTimersByTime(4000) })
    const next = await screen.findByRole('group', { name: asked('nightly-checks', 'Anything else?', 'del-1') })
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()

    await answer.resolve({ detail: 'Question q-approve is not open in delegation del-1.' }, 404)
    expect(screen.getByText('Could not answer nightly-checks: Question q-approve is not open in delegation del-1.')).toBeInTheDocument()
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
    expect(within(next).getByRole('textbox', { name: 'Free-text answer' })).toBeEnabled()
})

it('shows a reply refused while Paused hides its responder, there and once All shows its question closed', async () => {
    const answer = later()
    serve({
        [LIST]: [ok(list(nightly)), ok(list(delegation('del-1', 'nightly-checks', [])))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
        [answerRoute('del-1', 'q-approve')]: [answer.reply],
    })
    const { user } = renderCard()
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    await user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    await answer.resolve({ detail: 'Question q-approve is not open in delegation del-1.' }, 404)
    const report = 'Could not answer nightly-checks: Question q-approve is not open in delegation del-1.'
    expect(screen.getByText(report)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'All' }))
    expect(await screen.findByText('flow · waiting')).toBeInTheDocument()
    expect(screen.getByText(report)).toBeInTheDocument()
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
})

it('shows both of two replies, sent before and after Settings was reopened, refused after their questions closed', async () => {
    const approve = later()
    const note = later()
    const both = delegation('del-1', 'nightly-checks', [approveGate(), freeText('q-note', 'Any release notes?')])
    serve({
        [LIST]: [ok(list(both)), ok(list(both)), ok(list(delegation('del-1', 'nightly-checks', [])))],
        [answerRoute('del-1', 'q-approve')]: [approve.reply],
        [answerRoute('del-1', 'q-note')]: [note.reply],
    })
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    const notes = asked('nightly-checks', 'Any release notes?', 'del-1')
    const first = renderCard()
    await first.user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    first.unmount()

    const { user } = renderCard()
    const responder = await screen.findByRole('group', { name: notes })
    await user.type(within(responder).getByRole('textbox', { name: 'Free-text answer' }), 'ship it')
    await user.click(within(responder).getByRole('button', { name: 'Answer' }))
    // The live refresh finds both questions closed while the replies are out.
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(screen.queryByRole('group', { name: notes })).not.toBeInTheDocument())
    await approve.resolve({ detail: 'Question q-approve is not open.' }, 404)
    await note.resolve({ detail: 'Question q-note is not open.' }, 404)

    expect(screen.getByText('Could not answer nightly-checks: Question q-approve is not open.')).toBeInTheDocument()
    expect(screen.getByText('Could not answer nightly-checks: Question q-note is not open.')).toBeInTheDocument()
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: notes })).not.toBeInTheDocument()
})

it('reports a reply sent before Settings was reopened, and its failed reload, on the card shown now, with the form draft released', async () => {
    const answer = later()
    const form = deployForm('form-away', 'Away details')
    const row = delegation('del-f', 'deploy-task', [form])
    serve({
        [LIST]: [ok(list(row)), ok(list(row)), fail(500, 'database is locked'), ok(list(delegation('del-f', 'deploy-task', []))), ok(list(row))],
        [answerRoute('del-f', 'form-away')]: [answer.reply],
    })
    const name = asked('deploy-task', 'Away details', 'del-f')
    const first = renderCard()
    const before = await screen.findByRole('group', { name })
    await first.user.type(within(before).getByRole('textbox', { name: 'Region' }), 'eu-west')
    await first.user.type(within(before).getByRole('textbox', { name: 'Replicas' }), '3')
    await first.user.click(within(before).getByRole('button', { name: 'Submit' }))
    first.unmount()

    renderCard()
    expect(within(await screen.findByRole('group', { name })).getByRole('button', { name: 'Submitting...' })).toBeDisabled()
    await answer.resolve({})
    expect(await screen.findByText('Sent answer for deploy-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.queryByRole('group', { name })).not.toBeInTheDocument()

    // A refresh shows the form answered, the next asks it again: it starts empty.
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(4))
    await settled()
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' })).toHaveValue('')
})

it('keeps one environment\'s answered questions and failed draft while another environment\'s Settings is open', async () => {
    serve({
        ...ENVIRONMENTS,
        // Every local snapshot still lists both questions.
        [LIST]: [ok(list(delegation('del-1', 'nightly-checks', [approveGate(), freeText('q-note', 'Any release notes?')])))],
        [answerRoute('del-1', 'q-approve')]: [ok({})],
        [answerRoute('del-1', 'q-note')]: [fail(409, 'Question q-note is busy.')],
        [`GET ${REMOTE}/workspace/api/delegations`]: [ok(list(delegation('del-9', 'remote-task', [])))],
    })
    const gate = asked('nightly-checks', 'Ship this build?', 'del-1')
    const note = asked('nightly-checks', 'Any release notes?', 'del-1')
    const first = renderCard()
    await first.user.click(within(await screen.findByRole('group', { name: gate })).getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText('Sent answer for nightly-checks.')).toBeInTheDocument()
    await settled()
    await first.user.type(within(screen.getByRole('group', { name: note })).getByRole('textbox', { name: 'Free-text answer' }), 'ship it')
    await first.user.click(within(screen.getByRole('group', { name: note })).getByRole('button', { name: 'Answer' }))
    expect(await screen.findByText('Question q-note is busy.')).toBeInTheDocument()
    first.unmount()

    // The other environment's System settings list only its own delegations.
    await switchTo('env-x')
    const away = renderCard()
    expect(await screen.findByText('remote-task')).toBeInTheDocument()
    await settled()
    away.unmount()

    await switchTo(null)
    renderCard()
    const back = await screen.findByRole('group', { name: note })
    expect(within(back).getByText('Question q-note is busy.')).toBeInTheDocument()
    expect(within(back).getByRole('textbox', { name: 'Free-text answer' })).toHaveValue('ship it')
    expect(screen.queryByRole('group', { name: gate })).not.toBeInTheDocument()
    expect(screen.getByText('flow · waiting on answer')).toBeInTheDocument()
})

it('keeps each environment\'s form values apart when linked homes share delegation and question IDs', async () => {
    const shared = delegation('del-f', 'deploy-task', [deployForm('form-same', 'Same details')])
    serve({
        ...ENVIRONMENTS,
        [LIST]: [ok(list(shared))],
        [`GET ${REMOTE}/workspace/api/delegations`]: [ok(list(shared))],
        [`POST ${REMOTE}${answerRoute('del-f', 'form-same').slice('POST '.length)}`]: [ok({})],
    })
    const name = asked('deploy-task', 'Same details', 'del-f')
    const first = renderCard()
    await first.user.type(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' }), 'eu-west')
    first.unmount()

    await switchTo('env-x')
    const away = renderCard()
    const remote = await screen.findByRole('group', { name })
    expect(within(remote).getByRole('textbox', { name: 'Region' })).toHaveValue('')
    await away.user.type(within(remote).getByRole('textbox', { name: 'Region' }), 'us-east')
    await away.user.type(within(remote).getByRole('textbox', { name: 'Replicas' }), '2')
    await away.user.click(within(remote).getByRole('button', { name: 'Submit' }))
    expect(await screen.findByText('Sent answer for deploy-task.')).toBeInTheDocument()
    await settled()
    away.unmount()

    await switchTo(null)
    renderCard()
    expect(within(await screen.findByRole('group', { name })).getByRole('textbox', { name: 'Region' })).toHaveValue('eu-west')
    expect(sent(answerRoute('del-f', 'form-same'))).toEqual([])
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

it('disables answering as soon as a link is removed, and lets delegations to environments the reloaded links keep be answered, though the refresh it starts fails', async () => {
    const linkTo = (id: string) => ({ environment_id: id, origin: `http://${id}.test`, label: id, created_at: '2026-11-23T00:00:00Z' })
    const other = delegation('del-3', 'release-task', [approveGate()], { environment_id: 'env-c' })
    const reload = later()
    serve({
        ...ENVIRONMENTS,
        'GET /workspace/api/environments/links': [ok({ links: [linkTo('env-b'), linkTo('env-c')] }), ok({ links: [linkTo('env-c')] })],
        [LIST]: [ok(list(nightly, other)), reload.reply],
    })
    await act(async () => { await refreshLocalEnvironments() })
    renderCard()
    const toB = within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })
    const toC = within(screen.getByRole('group', { name: 'release-task: Ship this build? (/srv/repo · env env-c · target run-del-3)' })).getByRole('button', { name: 'Approve' })
    expect(toB).toBeEnabled()
    expect(toC).toBeEnabled()
    await settled()

    // EnvironmentLinksCard removed env-b's link and announced it, before EnvironmentBootstrap reloads this machine's links.
    act(() => notifyEnvironmentLinksChanged())
    expect(sent(LIST)).toHaveLength(2)
    expect(toB).toBeDisabled()
    expect(toC).toBeDisabled()
    expect(screen.getAllByText('Answering is unavailable while a link change is checked.')).toHaveLength(2)

    await reload.resolve({ detail: 'database is locked' }, 500)
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()
    expect(toB).toBeDisabled()
    expect(toC).toBeDisabled()

    await act(async () => { await refreshLocalEnvironments() })
    expect(toB).toBeDisabled()
    expect(screen.getByText('Answering is unavailable: the link to env-b is missing.')).toBeInTheDocument()
    expect(toC).toBeEnabled()
    expect(screen.queryByText('Answering is unavailable while a link change is checked.')).not.toBeInTheDocument()
})

it('shows a link removed while a delegation is canceling on its row, with answering still off, though the refresh fails', async () => {
    const link = { environment_id: 'env-b', origin: 'http://env-b.test', label: 'B', created_at: '2026-11-23T00:00:00Z' }
    serve({
        ...ENVIRONMENTS,
        'GET /workspace/api/environments/links': [ok({ links: [link] }), ok({ links: [] })],
        [LIST]: [ok(list({ ...nightly, ...requested })), fail(500, 'database is locked')],
    })
    await act(async () => { await refreshLocalEnvironments() })
    renderCard()
    const approve = within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })
    expect(screen.getByText('Answering is unavailable while cancellation is pending.')).toBeInTheDocument()
    await settled()

    await act(async () => {
        notifyEnvironmentLinksChanged()
        await refreshLocalEnvironments()
    })
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()
    expect(screen.getByText('Answering is unavailable: the link to env-b is missing.')).toBeInTheDocument()
    expect(screen.getByText('flow · canceling')).toBeInTheDocument()
    expect(approve).toBeDisabled()
})

it('keeps answering off on return to System after a link is removed while another Settings tab is shown, and reloads only once System is shown', async () => {
    const link = { environment_id: 'env-b', origin: 'http://env-b.test', label: 'B', created_at: '2026-11-23T00:00:00Z' }
    const before = later()
    serve({
        ...ENVIRONMENTS,
        'GET /workspace/api/environments/links': [ok({ links: [link] }), ok({ links: [] })],
        [LIST]: [ok(list(nightly)), before.reply, fail(500, 'database is locked'), ok(list({ ...nightly, paused: 'The link to this environment was removed.' }))],
    })
    await act(async () => { await refreshLocalEnvironments() })
    const { user, rerender } = renderCard()
    const approve = within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })
    expect(approve).toBeEnabled()
    await settled()
    // A list requested before the removal is still out.
    await user.click(screen.getByRole('button', { name: 'Refresh' }))

    // The removal completes while another Settings tab is shown; this machine's links haven't reloaded yet.
    rerender(<DialogProvider><DelegationsCard active={false} /></DialogProvider>)
    act(() => notifyEnvironmentLinksChanged())
    await before.resolve(list(nightly))
    expect(sent(LIST)).toHaveLength(2)

    rerender(<DialogProvider><DelegationsCard /></DialogProvider>)
    expect(await screen.findByText('Database is locked')).toBeInTheDocument()
    await settled()
    expect(sent(LIST)).toHaveLength(3)
    expect(approve).toBeDisabled()
    expect(screen.getByText('Answering is unavailable while a link change is checked.')).toBeInTheDocument()

    // Live refresh goes on and shows the delegation paused.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('The link to this environment was removed.')).toBeInTheDocument()
    expect(approve).toBeDisabled()
    await act(async () => { await refreshLocalEnvironments() })
    expect(screen.getByText('Answering is unavailable: the link to env-b is missing.')).toBeInTheDocument()
})

it('restores answering once a reread of this machine\'s links that outlasts live refreshes shows a resumed delegation\'s link', async () => {
    const link = { environment_id: 'env-b', origin: 'http://env-b.test', label: 'B', created_at: '2026-11-23T00:00:00Z' }
    const links = 'GET /workspace/api/environments/links'
    const slow = later()
    serve({
        ...ENVIRONMENTS,
        [links]: [ok({ links: [] }), slow.reply, later().reply],
        [LIST]: [ok(list({ ...nightly, paused: 'The link to this environment was removed.' })), ok(list(nightly))],
    })
    await act(async () => { await refreshLocalEnvironments() })
    renderCard()
    const approve = within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })
    await settled()

    // Linking it again resumes its delegations; the card rereads this machine's links, which take a while.
    act(() => notifyEnvironmentLinksChanged())
    expect(await screen.findByText('flow · waiting on answer')).toBeInTheDocument()
    await waitFor(() => expect(sent(links)).toHaveLength(2))
    for (const count of [3, 4]) {
        await act(async () => { vi.advanceTimersByTime(4000) })
        await waitFor(() => expect(sent(LIST)).toHaveLength(count))
        await settled()
    }
    expect(sent(links)).toHaveLength(2)
    expect(approve).toBeDisabled()

    await slow.resolve({ links: [link] })
    await waitFor(() => expect(approve).toBeEnabled())
    expect(screen.queryByText(/^Answering is unavailable/)).not.toBeInTheDocument()
})

it('keeps answering off while this machine\'s links omit a resumed delegation\'s environment, and rereads them on Refresh until they show it', async () => {
    const link = { environment_id: 'env-b', origin: 'http://env-b.test', label: 'B', created_at: '2026-11-23T00:00:00Z' }
    const links = 'GET /workspace/api/environments/links'
    serve({
        ...ENVIRONMENTS,
        [links]: [ok({ links: [] }), fail(500, 'database is locked'), fail(500, 'database is locked'), ok({ links: [link] })],
        [LIST]: [ok(list({ ...nightly, paused: 'The link to this environment was removed.' })), ok(list(nightly))],
    })
    await act(async () => { await refreshLocalEnvironments() })
    const { user } = renderCard()
    const approve = within(await screen.findByRole('group', { name: asked('nightly-checks', 'Ship this build?', 'del-1') })).getByRole('button', { name: 'Approve' })
    expect(approve).toBeDisabled()
    await settled()
    expect(sent(links)).toHaveLength(1)

    // Linking it again resumes its delegations, but this machine's links fail to reload, then again
    // when the card rereads them on seeing it resumed.
    await act(async () => {
        notifyEnvironmentLinksChanged()
        await refreshLocalEnvironments()
    })
    expect(await screen.findByText('flow · waiting on answer')).toBeInTheDocument()
    await waitFor(() => expect(sent(links)).toHaveLength(3))
    await settled()
    expect(approve).toBeDisabled()
    expect(screen.getByText('Answering is unavailable: the link to env-b is missing.')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(approve).toBeEnabled())
    expect(screen.queryByText(/^Answering is unavailable/)).not.toBeInTheDocument()
    expect(sent(links)).toHaveLength(4)
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

it('drops an Open response that resolves after Paused is selected, and polls only once it is in', async () => {
    const open = later()
    serve({
        [LIST]: [ok(list(nightly, pausedTask))],
        [`${LIST}?open=true`]: [open.reply, ok(list(nightly, pausedTask))],
        [`${LIST}?paused=true`]: [ok(list(pausedTask))],
    })
    const { user } = renderCard()
    expect(await screen.findByText('nightly-checks')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open' }))
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    await settled()
    // The superseded Open request is still out, so the live refresh skips its tick.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(sent(`${LIST}?open=true`)).toHaveLength(1)

    await open.resolve(list(nightly))
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
    expect(screen.getByText('paused-task')).toBeInTheDocument()
    // Paused's live refresh reads the Open list and keeps the paused ones.
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(`${LIST}?open=true`)).toHaveLength(2))
    await settled()
    expect(sent(`${LIST}?paused=true`)).toHaveLength(1)
    expect(sent(LIST)).toHaveLength(1)
    expect(screen.getByText('paused-task')).toBeInTheDocument()
    expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument()
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

it('shows a cancel that fails after Settings was left and reopened on the reopened row, and again on the next reopen', async () => {
    const cancel = later()
    serve({
        [LIST]: [ok(list(nightly))],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    const second = renderCard()
    const row = () => screen.findByRole('group', { name: /^Delegation nightly-checks/ })
    await settled()
    await cancel.resolve({ detail: 'linked environment is unreachable' }, 503)
    expect(await within(await row()).findByText('Linked environment is unreachable')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeEnabled()
    second.unmount()

    renderCard()
    expect(await within(await row()).findByText('Linked environment is unreachable')).toBeInTheDocument()
    await settled()
    expect(screen.getByText('Linked environment is unreachable')).toBeInTheDocument()
})

it('keeps a local seal that fails during a reopen on its row while the reopened list still shows it paused with its cancel pending', async () => {
    const unlinked = { ...nightly, ...requested, paused: 'No link to this environment; link it again to resume.' }
    const seal = later()
    const reopened = later()
    serve({
        [LIST]: [ok(list(unlinked)), reopened.reply, ok(list(unlinked))],
        [cancelRoute('del-1')]: [seal.reply],
    })
    const first = renderCard()
    await first.user.click(await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' }))
    await first.user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel delegation' }))
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    await seal.resolve({ detail: 'database is locked' }, 500)
    await reopened.resolve(list(unlinked))
    const row = await screen.findByRole('group', { name: /^Delegation nightly-checks/ })
    expect(within(row).getByText('Database is locked')).toBeInTheDocument()
    expect(within(row).getByRole('button', { name: 'Seal delegation nightly-checks as canceled' })).toBeEnabled()

    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(3))
    await settled()
    expect(within(row).getByText('Database is locked')).toBeInTheDocument()
})

it('shows a local seal that fails after Settings was reopened on the row the reopened list shows sealed', async () => {
    const unlinked = { ...nightly, ...requested, paused: 'No link to this environment; link it again to resume.' }
    const sealed = { ...unlinked, paused: null, phase: 'sealed', result: { status: 'canceled' }, updated_at: '2026-11-23T00:03:00Z' }
    const seal = later()
    serve({
        // The seal commits before its request fails, so the reopened list shows it sealed.
        [LIST]: [ok(list(unlinked)), ok(list(sealed))],
        [cancelRoute('del-1')]: [seal.reply],
    })
    const first = renderCard()
    await first.user.click(await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' }))
    await first.user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel delegation' }))
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    expect(await screen.findByText('flow · canceled')).toBeInTheDocument()
    await settled()
    await seal.resolve({ detail: 'database is locked' }, 500)
    const row = screen.getByRole('group', { name: /^Delegation nightly-checks/ })
    expect(within(row).getByText('Database is locked')).toBeInTheDocument()
    expect(within(row).getByText('flow · canceled')).toBeInTheDocument()
    expect(within(row).queryByRole('button', { name: /Cancel delegation|Seal delegation/ })).not.toBeInTheDocument()
})

// The reopened card's first list, held until the cancel sent before Settings was left has failed.
const cancelFailsBeforeFirstList = async (listed: unknown, ...polls: unknown[]) => {
    const cancel = later()
    const reopened = later()
    serve({
        [LIST]: [ok(list(nightly)), reopened.reply, ...polls.map((row) => ok(list(row)))],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    await cancel.resolve({ detail: 'linked environment is unreachable' }, 503)
    await reopened.resolve(list(listed))
    return screen.findByRole('group', { name: /^Delegation nightly-checks/ })
}
const canceledSeal = { ...nightly, ...requested, phase: 'sealed', result: { status: 'canceled' }, updated_at: '2026-11-23T00:03:00Z' }

it('shows a cancel that fails after Settings was reopened on the row its first list shows canceling, until a refresh shows it sealed', async () => {
    const canceling = { ...nightly, ...requested }
    const row = await cancelFailsBeforeFirstList(canceling, canceling, canceledSeal)
    expect(within(row).getByText('flow · canceling')).toBeInTheDocument()
    expect(within(row).getByText('Linked environment is unreachable')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(LIST)).toHaveLength(3))
    await settled()
    expect(within(row).getByText('Linked environment is unreachable')).toBeInTheDocument()

    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('flow · canceled')).toBeInTheDocument()
    expect(screen.queryByText('Linked environment is unreachable')).not.toBeInTheDocument()
})

it('shows a cancel that fails after Settings was reopened on the row its first list shows sealed', async () => {
    const row = await cancelFailsBeforeFirstList(canceledSeal)
    expect(within(row).getByText('flow · canceled')).toBeInTheDocument()
    expect(within(row).getByText('Linked environment is unreachable')).toBeInTheDocument()
})

it('leaves a row sealed by a retry after Settings was reopened clear of an earlier try\'s failure that arrives later', async () => {
    const unlinked = { ...nightly, ...requested, paused: 'No link to this environment; link it again to resume.' }
    const sealed = { ...unlinked, paused: null, phase: 'sealed', result: { status: 'canceled' }, updated_at: '2026-11-23T00:03:00Z' }
    const earlier = later()
    serve({
        [LIST]: [ok(list(unlinked)), ok(list(unlinked)), ok(list(sealed))],
        [cancelRoute('del-1')]: [earlier.reply, ok(sealed)],
    })
    const sealLocally = async (user: User) => {
        await user.click(await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' }))
        await user.click(within(screen.getByRole('alertdialog')).getByRole('button', { name: 'Cancel delegation' }))
    }
    const first = renderCard()
    await sealLocally(first.user)
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    const { user } = renderCard()
    await settled()
    await sealLocally(user)
    expect(await screen.findByText('Canceled delegation nightly-checks.')).toBeInTheDocument()
    await settled()
    await earlier.resolve({ detail: 'database is locked' }, 500)
    expect(screen.getByText('flow · canceled')).toBeInTheDocument()
    expect(screen.queryByText('Database is locked')).not.toBeInTheDocument()
    expect(sent(cancelRoute('del-1'))).toHaveLength(2)
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

it('shows a cancel that completes after Settings was reopened on the reopened row, even when the reload after it fails', async () => {
    const cancel = later()
    serve({
        [LIST]: [ok(list(nightly)), ok(list(nightly)), fail(500, 'database is locked')],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    const name = asked('nightly-checks', 'Ship this build?', 'del-1')
    expect(within(await screen.findByRole('group', { name })).getByRole('button', { name: 'Approve' })).toBeEnabled()
    await settled()
    await cancel.resolve({ ...nightly, ...requested })
    expect(await screen.findByText('Cancellation requested for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.getByText('flow · canceling')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancel delegation nightly-checks' })).toBeDisabled()
    expect(within(screen.getByRole('group', { name })).getByRole('button', { name: 'Approve' })).toBeDisabled()
    expect(sent(LIST)).toHaveLength(3)
    expect(sent(cancelRoute('del-1'))).toHaveLength(1)
})

it('keeps a row the reopened list shows sealed when an older cancel response arrives, even when the reload after it fails', async () => {
    const cancel = later()
    serve({
        [LIST]: [ok(list(nightly)), ok(list({ ...nightly, phase: 'sealed', result: { status: 'canceled' } })), fail(500, 'database is locked')],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    expect(await screen.findByText('flow · canceled')).toBeInTheDocument()
    await settled()
    await cancel.resolve({ ...nightly, ...requested })
    expect(await screen.findByText('Cancellation requested for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.getByText('flow · canceled')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Cancel delegation nightly-checks' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Approve' })).not.toBeInTheDocument()
    expect(sent(LIST)).toHaveLength(3)
})

it.each([
    ['a later second', '2026-11-23T00:02:00Z'],
    ['the same second', '2026-11-23T00:01:00Z'],
])('keeps a paused row from %s and its local seal on a reopened card when an unpaused cancel response arrives and the reload after it fails', async (_when, pausedAt) => {
    const cancel = later()
    const unlinked = { ...nightly, ...requested, paused: 'No link to this environment; link it again to resume.', updated_at: pausedAt }
    serve({
        [LIST]: [ok(list(nightly)), ok(list(unlinked)), fail(500, 'database is locked')],
        [cancelRoute('del-1')]: [cancel.reply],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    first.unmount()

    renderCard()
    expect(await screen.findByRole('button', { name: 'Seal delegation nightly-checks as canceled' })).toBeInTheDocument()
    await settled()
    await cancel.resolve({ ...nightly, ...requested, updated_at: '2026-11-23T00:01:00Z' })
    expect(await screen.findByText('Cancellation requested for nightly-checks. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.getByText('No link to this environment; link it again to resume.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Seal delegation nightly-checks as canceled' })).toBeEnabled()
})

it('leaves a linked environment\'s list requests and live refreshes alone when a cancel sent from Local completes there', async () => {
    const cancel = later()
    const remoteList = `GET ${REMOTE}/workspace/api/delegations`
    serve({
        ...ENVIRONMENTS,
        [LIST]: [ok(list(nightly))],
        [cancelRoute('del-1')]: [cancel.reply],
        [remoteList]: [ok(list(delegation('del-9', 'remote-task', [])))],
    })
    const first = renderCard()
    await confirmCancel(first.user, 'nightly-checks')
    await waitFor(() => expect(sent(cancelRoute('del-1'))).toHaveLength(1))
    // The user switches to the linked environment and opens its System settings while the cancel is out.
    first.unmount()
    await switchTo('env-x')
    renderCard()
    expect(await screen.findByText('remote-task')).toBeInTheDocument()
    await settled()

    await cancel.resolve({ ...nightly, ...requested })
    expect(sent(remoteList)).toHaveLength(1)
    expect(sent(LIST)).toHaveLength(1)
    expect(screen.queryByText('Cancellation requested for nightly-checks.')).not.toBeInTheDocument()
    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(sent(remoteList)).toHaveLength(2))
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
        // Sealing clears the pause, in the same second the list showed it paused.
        [cancelRoute('del-2')]: [ok({ ...unlinked, paused: null, phase: 'sealed', result: { status: 'canceled' } })],
    })
    const { user } = renderCard()
    const token = async (requestKey: string, id: string) => within(await screen.findByRole('group', { name: asked(requestKey, 'Deploy token?', id) })).getByLabelText('Deploy token?')
    await user.type(await token('nightly-checks', 'del-1'), 's3cret-1')
    await user.type(await token('paused-task', 'del-2'), 's3cret-2')
    expect(Object.keys(localAnswers().drafts)).toEqual(['del-1:req-1', 'del-2:req-2'])

    // del-1 seals; del-2 only pauses, so its draft waits for the link to come back.
    await act(async () => { vi.advanceTimersByTime(4000) })
    expect(await screen.findByText('conversation · completed')).toBeInTheDocument()
    expect(Object.keys(localAnswers().drafts)).toEqual(['del-2:req-2'])
    expect(await token('paused-task', 'del-2')).toHaveValue('s3cret-2')

    // Sealing it locally releases its draft even though the reload after it fails.
    await confirmCancel(user, 'paused-task')
    expect(await screen.findByText('Canceled delegation paused-task. Could not refresh delegations: Database is locked')).toBeInTheDocument()
    expect(screen.getByText('conversation · canceled')).toBeInTheDocument()
    expect(screen.queryByText('No link to this environment; link it again to resume.')).not.toBeInTheDocument()
    expect(localAnswers().drafts).toEqual({})
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
    expect([...localAnswers().acknowledged]).toEqual(['del-1:q-approve'])

    await act(async () => { vi.advanceTimersByTime(4000) })
    await waitFor(() => expect(screen.queryByText('nightly-checks')).not.toBeInTheDocument())
    expect(Object.keys(localAnswers().drafts)).toEqual(['del-5:req-5'])
    expect(localAnswers().acknowledged.size).toBe(0)

    // Paused leaves del-5 out without saying it ended: its draft stays for when Open shows it again.
    await user.click(screen.getByRole('button', { name: 'Paused' }))
    expect(await screen.findByText('paused-task')).toBeInTheDocument()
    expect(Object.keys(localAnswers().drafts)).toEqual(['del-5:req-5'])
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
