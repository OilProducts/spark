import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { EnvironmentBootstrap, EnvironmentSwitcherBar } from '@/app/EnvironmentSwitcher'
import { DialogProvider } from '@/components/app/dialog-controller'
import { resetActiveEnvironmentForTests } from '@/lib/activeEnvironment'
import { EnvironmentLinksCard } from '../EnvironmentLinksCard'

const TOKENS = '/workspace/api/environment/tokens'
const LINKS = '/workspace/api/environments/links'
const descriptor = {
    environment_id: 'env-local-1',
    display_name: 'mac-studio',
    protocol_version: 1,
    ui_api_version: 1,
    spark_version: '0.1.0',
}
type Link = { environment_id: string; origin: string; label: string; created_at: string }
type Token = { token_id: string; label: string; created_at: string; revoked_at: string | null; is_current: boolean }
const link = (environment_id: string, label: string): Link =>
    ({ environment_id, origin: `https://${environment_id}.example:8000`, label, created_at: '2026-11-23T00:00:00Z' })
const token = (token_id: string, label: string, is_current = false): Token =>
    ({ token_id, label, created_at: '2026-11-23T00:00:00Z', revoked_at: null, is_current })
const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

/** A Spark backend behind the real environments client: mutations change what later listings return. */
function stubBackend(seed: { tokens?: Token[]; links?: Link[]; replies?: Record<string, unknown> } = {}) {
    const backend = {
        tokens: seed.tokens ?? [],
        links: seed.links ?? [],
        /** Per-URL overrides for GETs: a JSON body, or a function returning the response. */
        replies: seed.replies ?? {},
        /** Every non-GET request, as "METHOD url". */
        mutations: [] as string[],
        bodies: [] as unknown[],
    }
    vi.stubGlobal('fetch', vi.fn(async (url: string, init?: RequestInit) => {
        const method = init?.method ?? 'GET'
        if (method !== 'GET') {
            backend.mutations.push(`${method} ${url}`)
            backend.bodies.push(init?.body ? JSON.parse(String(init.body)) : null)
        }
        const reply = method === 'GET' ? backend.replies[url] : undefined
        if (typeof reply === 'function') return reply()
        if (reply !== undefined) return json(reply)
        const revoked = method === 'POST' && url.match(/^\/workspace\/api\/environment\/tokens\/([^/]+)\/revoke$/)
        if (revoked) {
            backend.tokens = backend.tokens.map((row) => row.token_id === revoked[1] ? { ...row, revoked_at: '2026-11-24T00:00:00Z' } : row)
            return json({ ok: true })
        }
        const unlinked = method === 'DELETE' && url.match(/^\/workspace\/api\/environments\/links\/([^/]+)$/)
        if (unlinked) {
            backend.links = backend.links.filter((row) => row.environment_id !== unlinked[1])
            return json({ ok: true })
        }
        if (method === 'POST' && url === LINKS) {
            const body = JSON.parse(String(init?.body)) as { expected_id: string; label: string }
            const created = link(body.expected_id, body.label)
            backend.links = [...backend.links, created]
            return json(created)
        }
        if (method === 'POST' && url === '/workspace/api/environment/pairing-codes') {
            return json({ environment_id: descriptor.environment_id, code: `code-${backend.mutations.length}`, expires_at: new Date(Date.now() + 10 * 60_000).toISOString() })
        }
        if (url === '/environment') return json(descriptor)
        if (url === TOKENS) return json({ tokens: backend.tokens })
        if (url === LINKS) return json({ links: backend.links })
        return json({ detail: `No route for ${method} ${url}` }, 404)
    }))
    return backend
}

type User = ReturnType<typeof userEvent.setup>
const card = (active = true) => <DialogProvider><EnvironmentLinksCard active={active} /></DialogProvider>
const stubClipboard = (clipboard: Partial<Clipboard> | undefined) =>
    vi.spyOn(navigator, 'clipboard', 'get').mockReturnValue(clipboard as Clipboard)
const statusText = () => screen.queryByRole('status')?.textContent ?? ''

afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
    vi.restoreAllMocks()
    resetActiveEnvironmentForTests()
})

it('lists the projects a linked peer relays as a JSON array and rejects any other shape', async () => {
    const user = userEvent.setup()
    stubBackend({
        links: [link('env-build', 'Build box'), link('env-old', 'Old box')],
        replies: {
            [`${LINKS}/env-build/projects`]: [{ project_path: '/srv/app' }, { project_path: '/srv/worker' }],
            [`${LINKS}/env-old/projects`]: { projects: [{ project_path: '/srv/legacy' }] },
        },
    })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Projects on Build box' }))
    expect(await screen.findByText('/srv/app')).toBeInTheDocument()
    expect(screen.getByText('/srv/worker')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Projects on Old box' }))
    expect(await screen.findByText(/Expected an array of projects/)).toBeInTheDocument()
    expect(screen.queryByText('/srv/legacy')).not.toBeInTheDocument()
})

it('keeps the token authenticating this connection from being revoked', async () => {
    stubBackend({ tokens: [token('tok-self', 'This laptop', true), token('tok-ci', 'CI runner')] })
    render(card())

    expect(await screen.findByRole('button', { name: 'Cannot revoke This laptop: it authenticates this connection' })).toBeDisabled()
    expect(screen.getByText('Current connection')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Revoke This laptop' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Revoke CI runner' })).toBeEnabled()
})

/** Each performs a mutation and returns a check that its result is on screen. */
const mutations = [
    ['revoke', async (user: User) => {
        await user.click(screen.getByRole('button', { name: 'Revoke CI runner' }))
        await user.click(await screen.findByRole('button', { name: 'Revoke token' }))
        return () => {
            expect(screen.getByText('Revoked')).toBeInTheDocument()
            expect(screen.queryByRole('button', { name: 'Revoke CI runner' })).not.toBeInTheDocument()
        }
    }],
    ['add', async (user: User) => {
        await user.type(screen.getByLabelText('Remote origin'), 'https://env-gpu.example:8000')
        await user.type(screen.getByLabelText('Remote environment ID'), 'env-gpu')
        await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
        await user.type(screen.getByLabelText('Link label (optional)'), 'GPU')
        await user.click(screen.getByRole('button', { name: 'Add link' }))
        return () => expect(screen.getByRole('button', { name: 'Remove link GPU' })).toBeInTheDocument()
    }],
    ['remove', async (user: User) => {
        await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
        await user.click(await screen.findByRole('button', { name: 'Remove link' }))
        return () => {
            expect(screen.queryByRole('button', { name: 'Remove link Build box' })).not.toBeInTheDocument()
            expect(screen.queryByRole('button', { name: 'Projects on Build box' })).not.toBeInTheDocument()
        }
    }],
] as const

it.each(mutations)('keeps the rows after a %s when a refresh started before it resolves last', async (_name, mutate) => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')] })
    render(card())
    await screen.findByRole('button', { name: 'Revoke CI runner' })

    // The next listings answer with the rows as they are before the mutation, but only once released.
    const releases: Array<() => void> = []
    for (const url of [TOKENS, LINKS]) {
        const stale = json({ tokens: backend.tokens, links: backend.links })
        backend.replies[url] = () => {
            delete backend.replies[url]
            return new Promise<Response>((resolve) => releases.push(() => resolve(stale)))
        }
    }
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    const postMutationRows = await mutate(user)
    await waitFor(postMutationRows)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())

    await act(async () => releases.forEach((release) => release()))
    postMutationRows()
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled()
})

it.each(mutations)('shows a successful %s even when the follow-up listing fails', async (_name, mutate) => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')] })
    render(card())
    await screen.findByRole('button', { name: 'Revoke CI runner' })

    backend.replies[TOKENS] = () => json({ detail: 'token listing failed' }, 500)
    backend.replies[LINKS] = () => json({ detail: 'link listing failed' }, 500)
    const postMutationRows = await mutate(user)
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load peer tokens: Token listing failed')
    expect(alert).toHaveTextContent('Could not load linked environments: Link listing failed')
    postMutationRows()
    expect(backend.mutations).toHaveLength(1)
})

it('confirms revoking a token and removing a link, naming each, and sends nothing on cancel', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')] })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Revoke CI runner' }))
    const revokeDialog = await screen.findByRole('alertdialog')
    expect(revokeDialog).toHaveTextContent('Revoke peer token?')
    expect(revokeDialog).toHaveTextContent('CI runner (tok-ci)')
    await user.click(within(revokeDialog).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(backend.mutations).toEqual([])
    expect(screen.queryByText('Revoked')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke CI runner' })).toHaveFocus())

    await user.click(screen.getByRole('button', { name: 'Revoke CI runner' }))
    await user.click(await screen.findByRole('button', { name: 'Revoke token' }))
    expect(await screen.findByText('Revoked')).toBeInTheDocument()
    expect(backend.mutations).toEqual([`POST ${TOKENS}/tok-ci/revoke`])

    backend.mutations.length = 0
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    const removeDialog = await screen.findByRole('alertdialog')
    expect(removeDialog).toHaveTextContent('Remove environment link?')
    expect(removeDialog).toHaveTextContent('Build box (https://env-build.example:8000)')
    expect(removeDialog).toHaveTextContent('open delegations pause')
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(backend.mutations).toEqual([])
    await waitFor(() => expect(screen.getByRole('button', { name: 'Remove link Build box' })).toHaveFocus())

    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    expect(await screen.findByText('No linked environments configured.')).toBeInTheDocument()
    expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`])
})

it('updates the environment switcher without a reload when links are added and removed', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    render(<DialogProvider><EnvironmentBootstrap /><EnvironmentSwitcherBar /><EnvironmentLinksCard /></DialogProvider>)
    const tabs = () => within(screen.getByRole('tablist', { name: 'Spark environments' })).getAllByRole('tab').map((tab) => tab.textContent)
    await waitFor(() => expect(tabs()).toEqual(['mac-studio(local)', 'Build box']))

    await user.type(await screen.findByLabelText('Remote origin'), '  https://env-gpu.example:8000 ')
    await user.type(screen.getByLabelText('Remote environment ID'), 'env-gpu')
    await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
    await user.type(screen.getByLabelText('Link label (optional)'), 'GPU')
    await user.click(screen.getByRole('button', { name: 'Add link' }))
    expect(await screen.findByRole('button', { name: 'Remove link GPU' })).toBeInTheDocument()
    expect(backend.bodies).toEqual([{ origin: 'https://env-gpu.example:8000', expected_id: 'env-gpu', code: 'code-xyz', label: 'GPU' }])
    await waitFor(() => expect(tabs()).toEqual(['mac-studio(local)', 'Build box', 'GPU']))

    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove link Build box' })).not.toBeInTheDocument())
    await waitFor(() => expect(tabs()).toEqual(['mac-studio(local)', 'GPU']))
})

it.each([
    ['an HTML page', () => new Response('<!doctype html><title>Spark</title>', { status: 200, headers: { 'Content-Type': 'text/html' } }), 'Could not load this environment: GET /environment: Expected JSON response body.'],
    ['a server error', () => json({ detail: 'database is locked' }, 500), 'Could not load this environment: Database is locked'],
])('keeps the token and link rows when the descriptor load returns %s', async (_name, reply, message) => {
    stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')], replies: { '/environment': reply } })
    render(card())

    expect(await screen.findByRole('alert')).toHaveTextContent(message)
    expect(screen.getByText('CI runner')).toBeInTheDocument()
    expect(screen.getByText('Build box')).toBeInTheDocument()
    expect(screen.queryByText('env-local-1')).not.toBeInTheDocument()
})

it('shows empty states only after a section loads, and errors instead when it fails', async () => {
    stubBackend()
    const view = render(card())
    expect(screen.queryByText('No peer tokens issued yet.')).not.toBeInTheDocument()
    expect(screen.queryByText('No linked environments configured.')).not.toBeInTheDocument()
    expect(await screen.findByText('No peer tokens issued yet.')).toBeInTheDocument()
    expect(screen.getByText('No linked environments configured.')).toBeInTheDocument()
    view.unmount()

    stubBackend({ replies: { [TOKENS]: () => json({ detail: 'tokens table missing' }, 500), [LINKS]: () => json({ detail: 'links table missing' }, 500) } })
    render(card())
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load peer tokens: Tokens table missing')
    expect(alert).toHaveTextContent('Could not load linked environments: Links table missing')
    expect(screen.getByText('env-local-1')).toBeInTheDocument()
    expect(screen.queryByText('No peer tokens issued yet.')).not.toBeInTheDocument()
    expect(screen.queryByText('No linked environments configured.')).not.toBeInTheDocument()
})

it('clears an earlier success status when Projects discovery fails', async () => {
    const user = userEvent.setup()
    stubBackend({ links: [link('env-build', 'Build box')], replies: { [`${LINKS}/env-build/projects`]: () => json({ detail: 'peer unreachable' }, 502) } })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Create pairing code' }))
    await waitFor(() => expect(statusText()).toBe('Created a single-use pairing code.'))
    await user.click(screen.getByRole('button', { name: 'Projects on Build box' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Peer unreachable')
    expect(statusText()).toBe('')
})

it('clears an earlier success status when a later refresh fails', async () => {
    const user = userEvent.setup()
    const backend = stubBackend()
    stubClipboard({ writeText: vi.fn().mockResolvedValue(undefined) })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Copy ID' }))
    await waitFor(() => expect(statusText()).toBe('Copied environment ID.'))

    backend.replies['/environment'] = () => json({ detail: 'server restarting' }, 503)
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not load this environment:')
    expect(statusText()).toBe('')
})

it.each([
    ['unavailable', () => undefined, (label: string) => `Clipboard is unavailable; could not copy ${label}.`],
    ['rejecting writes', () => ({ writeText: vi.fn().mockRejectedValue(new DOMException('Denied', 'NotAllowedError')) }), (label: string) => `Could not copy ${label}.`],
])('reports a failed copy when the clipboard is %s', async (_name, clipboard, message) => {
    const user = userEvent.setup()
    stubBackend()
    stubClipboard(clipboard())
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Copy ID' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(message('environment ID'))
    expect(statusText()).toBe('')

    await user.click(screen.getByRole('button', { name: 'Create pairing code' }))
    await waitFor(() => expect(statusText()).toBe('Created a single-use pairing code.'))
    await user.click(screen.getByRole('button', { name: 'Copy code' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(message('pairing code'))
    expect(statusText()).toBe('')
})

it('copies the environment ID and pairing code and announces it only once written', async () => {
    const user = userEvent.setup()
    const backend = stubBackend()
    const writeText = vi.fn().mockResolvedValue(undefined)
    stubClipboard({ writeText })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Copy ID' }))
    await waitFor(() => expect(statusText()).toBe('Copied environment ID.'))
    expect(writeText).toHaveBeenLastCalledWith('env-local-1')

    await user.type(screen.getByLabelText('Token label (optional)'), ' CI runner ')
    await user.click(screen.getByRole('button', { name: 'Create pairing code' }))
    expect(backend.bodies).toEqual([{ label: 'CI runner' }])
    const code = (await screen.findByTestId('pairing-code-value')).textContent
    await user.click(screen.getByRole('button', { name: 'Copy code' }))
    await waitFor(() => expect(statusText()).toBe('Copied pairing code.'))
    expect(writeText).toHaveBeenLastCalledWith(code)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
})

it('stops offering an expired pairing code: on time, at copy time, on returning to the tab, and on refresh', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    vi.setSystemTime(new Date('2026-11-24T00:00:00Z'))
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime })
    stubBackend()
    const writeText = vi.fn().mockResolvedValue(undefined)
    stubClipboard({ writeText })
    const view = render(card())
    await screen.findByText('env-local-1')

    const createCode = async () => {
        await user.click(screen.getByRole('button', { name: 'Create pairing code' }))
        await screen.findByTestId('pairing-code-value')
    }
    // setSystemTime moves the clock without firing the expiry timer, as when a sleeping laptop wakes.
    const skipPastExpiry = () => vi.setSystemTime(Date.now() + 11 * 60_000)
    const expectExpired = () => {
        expect(screen.queryByRole('button', { name: 'Copy code' })).not.toBeInTheDocument()
        expect(screen.queryByTestId('pairing-code-value')).not.toBeInTheDocument()
        expect(screen.getByText(/The pairing code expired at/)).toBeInTheDocument()
    }
    const leaveAndReturn = () => {
        view.rerender(card(false))
        view.rerender(card(true))
    }

    // A live code copies; its timer expires it, and neither returning to the tab nor refreshing brings it back.
    await createCode()
    await user.click(screen.getByRole('button', { name: 'Copy code' }))
    expect(writeText).toHaveBeenCalledTimes(1)
    act(() => vi.advanceTimersByTime(10 * 60_000))
    expectExpired()
    leaveAndReturn()
    expectExpired()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    expectExpired()

    // A clock set back after the timer was armed makes it fire early; it re-arms rather than leaving the code live.
    await createCode()
    vi.setSystemTime(Date.now() - 5 * 60_000)
    act(() => vi.advanceTimersByTime(10 * 60_000))
    expect(screen.getByRole('button', { name: 'Copy code' })).toBeInTheDocument()
    act(() => vi.advanceTimersByTime(5 * 60_000))
    expectExpired()

    // Copying re-checks the expiry first.
    await createCode()
    skipPastExpiry()
    await user.click(screen.getByRole('button', { name: 'Copy code' }))
    expectExpired()
    expect(writeText).toHaveBeenCalledTimes(1)

    // Returning to the System tab re-checks it.
    await createCode()
    skipPastExpiry()
    leaveAndReturn()
    expectExpired()

    // So does Refresh.
    await createCode()
    skipPastExpiry()
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    expectExpired()
    expect(writeText).toHaveBeenCalledTimes(1)
})
