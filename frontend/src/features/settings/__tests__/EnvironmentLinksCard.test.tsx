import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { EnvironmentBootstrap, EnvironmentSwitcherBar } from '@/app/EnvironmentSwitcher'
import { DialogProvider } from '@/components/app/dialog-controller'
import { getActiveEnvironmentId, resetActiveEnvironmentForTests, useActiveEnvironment } from '@/lib/activeEnvironment'
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
/** env-build as another tab relinks it, at a new origin. */
const relinked = { ...link('env-build', 'Build box'), origin: 'https://new-build.example:8000', created_at: '2026-11-24T00:05:00Z' }
const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } })

/** A Spark backend behind the real environments client: mutations change what later listings return. */
function stubBackend(seed: { tokens?: Token[]; links?: Link[]; replies?: Record<string, unknown> } = {}) {
    const backend = {
        tokens: seed.tokens ?? [],
        links: seed.links ?? [],
        /** Overrides keyed by URL for a GET, else by "METHOD url": a JSON body, or a function returning the response (or undefined to pass). */
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
        const reply = backend.replies[method === 'GET' ? url : `${method} ${url}`]
        const answer = typeof reply === 'function' ? await reply() : reply === undefined ? undefined : json(reply)
        if (answer) return answer
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

    // Listings fail once the mutation has been sent.
    backend.replies[TOKENS] = () => backend.mutations.length ? json({ detail: 'token listing failed' }, 500) : undefined
    backend.replies[LINKS] = () => backend.mutations.length ? json({ detail: 'link listing failed' }, 500) : undefined
    const postMutationRows = await mutate(user)
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Could not load peer tokens: Token listing failed')
    expect(alert).toHaveTextContent('Could not load linked environments: Link listing failed')
    postMutationRows()
    expect(screen.queryByText(/· revoked/)).not.toBeInTheDocument()
    expect(backend.mutations).toHaveLength(1)
})

it.each(mutations.flatMap(([name, mutate]) => (['succeeds', 'fails'] as const).map((listing) => [name, listing, mutate] as const)))(
    'updates a card reopened during a pending %s once it succeeds, when its follow-up listing %s', async (name, listing, mutate) => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')] })
    const request = { revoke: `POST ${TOKENS}/tok-ci/revoke`, add: `POST ${LINKS}`, remove: `DELETE ${LINKS}/env-build` }[name]
    let finishRequest = () => {}
    backend.replies[request] = () => new Promise<undefined>((resolve) => { finishRequest = () => resolve(undefined) })
    const settings = render(card())
    await screen.findByRole('button', { name: 'Revoke CI runner' })
    const postMutationRows = await mutate(user)
    await waitFor(() => expect(backend.mutations).toContain(request))

    // Leaving Settings unmounts the card; reopening System mounts one that lists the rows as they were before the request.
    settings.unmount()
    render(card())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    expect(screen.getByText('CI runner')).toBeInTheDocument()
    expect(screen.getByText('Build box')).toBeInTheDocument()

    if (listing === 'fails') {
        backend.replies[TOKENS] = () => json({ detail: 'token listing failed' }, 500)
        backend.replies[LINKS] = () => json({ detail: 'link listing failed' }, 500)
    }
    await act(async () => finishRequest())
    await waitFor(postMutationRows)
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    postMutationRows()
    if (listing === 'fails') {
        const alert = screen.getByRole('alert')
        expect(alert).toHaveTextContent('Could not load peer tokens: Token listing failed')
        expect(alert).toHaveTextContent('Could not load linked environments: Link listing failed')
    } else {
        expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    }
})

it('shows a failure that lands while Settings is closed in the next card to open', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    let failDelete = () => {}
    backend.replies[`DELETE ${LINKS}/env-build`] = () => new Promise<Response>((resolve) => {
        failDelete = () => resolve(json({ detail: 'database is locked' }, 500))
    })
    const settings = render(card())
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`]))

    settings.unmount()
    await act(async () => failDelete())
    render(card())
    expect(await screen.findByRole('alert')).toHaveTextContent('Database is locked')
    expect(screen.getByText('Build box')).toBeInTheDocument()
})

it('waits for a listing before saying no links remain after removing the last visible one', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    render(card())
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))

    // The listing after the DELETE is slow and finds a link added elsewhere meanwhile.
    let release!: () => void
    backend.replies[LINKS] = () => backend.mutations.length
        ? new Promise<Response>((resolve) => { release = () => resolve(json({ links: [link('env-gpu', 'GPU')] })) })
        : undefined
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Remove link Build box' })).not.toBeInTheDocument())
    expect(screen.queryByText('No linked environments configured.')).not.toBeInTheDocument()

    await act(async () => release())
    expect(await screen.findByRole('button', { name: 'Remove link GPU' })).toBeInTheDocument()
    expect(screen.queryByText('No linked environments configured.')).not.toBeInTheDocument()
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
    expect(await screen.findByText(/· revoked 2026-11-24T00:00:00Z/)).toBeInTheDocument()
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

it.each([
    ['refreshes', true],
    ['never refreshes', false],
])('sends no DELETE when another tab relinks the environment and this card %s while its removal is being confirmed', async (_name, refreshes) => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    render(card())
    await screen.findByRole('button', { name: 'Remove link Build box' })

    // Optionally, a slow Refresh is in flight while Remove waits for confirmation.
    let finishRefresh = () => {}
    if (refreshes) {
        backend.replies[LINKS] = () => {
            delete backend.replies[LINKS]
            return new Promise<Response>((resolve) => { finishRefresh = () => resolve(json({ links: backend.links })) })
        }
        await user.click(screen.getByRole('button', { name: 'Refresh' }))
    }
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    const staleDialog = await screen.findByRole('alertdialog')
    expect(staleDialog).toHaveTextContent('Build box (https://env-build.example:8000)')
    // Another tab relinks env-build at a new origin.
    backend.links = [relinked]
    await act(async () => finishRefresh())
    expect(screen.queryByText(/https:\/\/new-build\.example:8000/) !== null).toBe(refreshes)

    await user.click(within(staleDialog).getByRole('button', { name: 'Remove link' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it, so nothing was removed.')
    expect(backend.mutations).toEqual([])
    expect(backend.links).toHaveLength(1)
    expect(screen.getByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()

    // The current link can be reviewed and removed.
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    const currentDialog = await screen.findByRole('alertdialog')
    expect(currentDialog).toHaveTextContent('Build box (https://new-build.example:8000)')
    await user.click(within(currentDialog).getByRole('button', { name: 'Remove link' }))
    expect(await screen.findByText('No linked environments configured.')).toBeInTheDocument()
    expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`])
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
})

it.each([
    ['before', true],
    ['after', false],
])('keeps the relinked row and re-enables Refresh when a pending refresh finishes %s the removal check', async (_name, refreshFirst) => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    render(card())
    await screen.findByRole('button', { name: 'Remove link Build box' })

    // Link listings are held, each answering with the links the server had when it was asked.
    const held: Array<() => void> = []
    backend.replies[LINKS] = () => {
        const answer = json({ links: backend.links })
        return new Promise<Response>((resolve) => held.push(() => resolve(answer)))
    }
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    // Another tab relinks env-build at a new origin before this one confirms.
    backend.links = [relinked]
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(held).toHaveLength(2))
    delete backend.replies[LINKS]
    const [finishRefresh, finishCheck] = held
    await act(async () => (refreshFirst ? finishRefresh : finishCheck)())
    await act(async () => (refreshFirst ? finishCheck : finishRefresh)())

    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it, so nothing was removed.')
    expect(screen.getByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()
    expect(screen.queryByText(/https:\/\/env-build\.example:8000/)).not.toBeInTheDocument()
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    expect(backend.mutations).toEqual([])
})

/** App's workspace lifecycle (App.tsx): the card unmounts while a switch verifies and remounts for the new environment. */
function WorkspaceLikeApp() {
    const { activeEnvironmentId, selectedEnvironmentId, status } = useActiveEnvironment()
    const workspaceReady = selectedEnvironmentId === null || (status === 'ready' && activeEnvironmentId === selectedEnvironmentId)
    return (
        <DialogProvider>
            <EnvironmentBootstrap />
            <EnvironmentSwitcherBar />
            {workspaceReady ? <EnvironmentLinksCard key={activeEnvironmentId ?? 'local'} /> : null}
        </DialogProvider>
    )
}

it.each([
    ['completes', true],
    ['is still verifying', false],
])('removes a link only from the environment where it was confirmed when a switch %s as its check returns', async (_name, switchFirst) => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box'), link('env-gpu', 'GPU')] })
    // Switching to GPU waits on its descriptor, relayed through its proxy.
    const gpuDescriptor = `${LINKS}/env-gpu/proxy/environment`
    const gpu = { ...descriptor, environment_id: 'env-gpu', display_name: 'gpu-box' }
    let finishSwitch = () => {}
    backend.replies[gpuDescriptor] = () => {
        backend.replies[gpuDescriptor] = gpu
        return new Promise<Response>((resolve) => { finishSwitch = () => resolve(json(gpu)) })
    }
    backend.replies[`${LINKS}/env-gpu/proxy${TOKENS}`] = { tokens: [token('tok-gpu', 'GPU runner')] }
    backend.replies[`${LINKS}/env-gpu/proxy${LINKS}`] = { links: [] }
    render(<WorkspaceLikeApp />)
    const gpuTab = await screen.findByTestId('environment-tab-env-gpu')
    // The card names its environment, as in the notice below, once its descriptor loads.
    await screen.findByText('env-local-1')
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))

    // The removal's link check is held while the user switches to GPU, which unmounts this card.
    let finishCheck = () => {}
    backend.replies[LINKS] = () => {
        delete backend.replies[LINKS]
        const answer = json({ links: backend.links })
        return new Promise<Response>((resolve) => { finishCheck = () => resolve(answer) })
    }
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    await user.click(gpuTab)
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Refresh' })).not.toBeInTheDocument())
    if (switchFirst) {
        await act(async () => finishSwitch())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-gpu'))
        // On GPU's card, the user opens a new confirmation before the old check returns.
        await user.click(await screen.findByRole('button', { name: 'Revoke GPU runner' }))
        const dialog = await screen.findByRole('alertdialog')
        await act(async () => finishCheck())
        expect(await screen.findByText('You switched environments before the link to Build box on mac-studio was removed, so nothing was removed.')).toBeInTheDocument()
        expect(dialog).toHaveTextContent('Revoke peer token?')
        expect(backend.mutations).toEqual([])
        await user.click(within(dialog).getByRole('button', { name: 'Revoke token' }))
        await waitFor(() => expect(backend.mutations).toEqual([`POST ${LINKS}/env-gpu/proxy${TOKENS}/tok-gpu/revoke`]))
    } else {
        await act(async () => finishCheck())
        await waitFor(() => expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`]))
        await act(async () => finishSwitch())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-gpu'))
        expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`])
        expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    }
})

it('refreshes only cards for the environment a mutation changed once it succeeds after a switch', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box'), link('env-gpu', 'GPU')] })
    backend.replies[`${LINKS}/env-gpu/proxy/environment`] = { ...descriptor, environment_id: 'env-gpu', display_name: 'gpu-box' }
    backend.replies[`${LINKS}/env-gpu/proxy${TOKENS}`] = { tokens: [token('tok-gpu', 'GPU runner')] }
    backend.replies[`${LINKS}/env-gpu/proxy${LINKS}`] = { links: [] }
    let finishDelete = () => {}
    backend.replies[`DELETE ${LINKS}/env-build`] = () => new Promise<undefined>((resolve) => { finishDelete = () => resolve(undefined) })
    render(<WorkspaceLikeApp />)
    await screen.findByText('env-local-1')

    // The local removal is sent, then the user switches to GPU before it completes.
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(backend.mutations).toEqual([`DELETE ${LINKS}/env-build`]))
    await user.click(screen.getByTestId('environment-tab-env-gpu'))
    expect(await screen.findByRole('button', { name: 'Revoke GPU runner' })).toBeInTheDocument()
    const requests = vi.mocked(fetch).mock.calls.length

    await act(async () => finishDelete())
    await waitFor(() => expect(screen.queryByTestId('environment-tab-env-build')).not.toBeInTheDocument())
    // Only the switcher reloads this machine's links; GPU's card and the removed card send nothing.
    expect(vi.mocked(fetch).mock.calls.slice(requests).map(([url]) => url)).toEqual(['/environment', LINKS])
    expect(screen.getByRole('button', { name: 'Revoke GPU runner' })).toBeInTheDocument()
})

it('shows a replacement a removal check finds during a refresh, and still shows that refresh\'s other results', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box')] })
    render(card())
    await screen.findByRole('button', { name: 'Revoke CI runner' })

    // A slow Refresh finds the descriptor failing, CI runner revoked from another tab, and env-build as it was.
    const held: Array<() => void> = []
    const hold = (url: string, answer: Response) => {
        backend.replies[url] = () => {
            delete backend.replies[url]
            return new Promise<Response>((resolve) => held.push(() => resolve(answer)))
        }
    }
    hold('/environment', json({ detail: 'server restarting' }, 503))
    hold(TOKENS, json({ tokens: [{ ...token('tok-ci', 'CI runner'), revoked_at: '2026-11-24T00:00:00Z' }] }))
    hold(LINKS, json({ links: backend.links }))
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    await waitFor(() => expect(held).toHaveLength(3))

    // Another tab relinks env-build, and removing it here finds the replacement while the refresh is still in flight.
    backend.links = [relinked]
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it, so nothing was removed.')
    expect(screen.getByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled()

    await act(async () => held.forEach((release) => release()))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    const alert = screen.getByRole('alert')
    expect(alert).toHaveTextContent('The link to Build box changed since you chose to remove it')
    expect(alert).toHaveTextContent('Could not load this environment: Server restarting')
    expect(screen.getByText(/· revoked 2026-11-24T00:00:00Z/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Revoke CI runner' })).not.toBeInTheDocument()
    expect(screen.getByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()
    expect(screen.queryByText(/https:\/\/env-build\.example:8000/)).not.toBeInTheDocument()
    expect(backend.mutations).toEqual([])
})

it('keeps a replacement saved at the same origin in the same second under another label, and shows it for review', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    render(card())
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))

    // Another tab relinks env-build at the same origin, within the same second, as Build box 2.
    backend.links = [link('env-build', 'Build box 2')]
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it, so nothing was removed.')
    expect(backend.mutations).toEqual([])
    await waitFor(() => expect(screen.getByRole('button', { name: 'Remove link Build box 2' })).toHaveFocus())
})

it('leaves focus on a usable control in the card after revoking the last token that can be revoked', async () => {
    const user = userEvent.setup()
    stubBackend({ tokens: [token('tok-self', 'This laptop', true), token('tok-ci', 'CI runner')] })
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Revoke CI runner' }))
    await user.click(await screen.findByRole('button', { name: 'Revoke token' }))
    await waitFor(() => expect(statusText()).toBe('Revoked token CI runner.'))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toHaveFocus())
})

it('keeps focus on the control behind a failed revoke or removal, or in its section once that control is gone', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')], links: [link('env-build', 'Build box'), link('env-gpu', 'GPU')] })
    backend.replies[`POST ${TOKENS}/tok-ci/revoke`] = () => json({ detail: 'database is locked' }, 500)
    backend.replies[`DELETE ${LINKS}/env-gpu`] = () => json({ detail: 'database is locked' }, 500)
    render(card())

    await user.click(await screen.findByRole('button', { name: 'Revoke CI runner' }))
    await user.click(await screen.findByRole('button', { name: 'Revoke token' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Database is locked')
    await waitFor(() => expect(screen.getByRole('button', { name: 'Revoke CI runner' })).toHaveFocus())

    await user.click(screen.getByRole('button', { name: 'Remove link GPU' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(backend.mutations).toContain(`DELETE ${LINKS}/env-gpu`))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Remove link GPU' })).toHaveFocus())

    // Another tab removes Build box while its removal is confirmed here, so its row and Remove button go.
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    backend.links = backend.links.filter((row) => row.environment_id !== 'env-build')
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it')
    await waitFor(() => expect(screen.getByRole('button', { name: 'Switch to GPU' })).toHaveFocus())
    expect(backend.mutations).toEqual([`POST ${TOKENS}/tok-ci/revoke`, `DELETE ${LINKS}/env-gpu`])
})

it.each([
    ['succeeds', null],
    ['fails', 'Could not load peer tokens: Tokens table missing'],
])('keeps a copy failure visible when a refresh started before the copy %s', async (_name, listingError) => {
    const user = userEvent.setup()
    const backend = stubBackend({ tokens: [token('tok-ci', 'CI runner')] })
    stubClipboard({ writeText: vi.fn().mockRejectedValue(new DOMException('Denied', 'NotAllowedError')) })
    render(card())
    await screen.findByRole('button', { name: 'Revoke CI runner' })

    let finishRefresh = () => {}
    backend.replies[TOKENS] = () => {
        delete backend.replies[TOKENS]
        const answer = listingError ? json({ detail: 'tokens table missing' }, 500) : json({ tokens: backend.tokens })
        return new Promise<Response>((resolve) => { finishRefresh = () => resolve(answer) })
    }
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    await user.click(screen.getByRole('button', { name: 'Copy ID' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy environment ID.')

    await act(async () => finishRefresh())
    await waitFor(() => expect(screen.getByRole('button', { name: 'Refresh' })).toBeEnabled())
    expect(screen.getByRole('alert')).toHaveTextContent('Could not copy environment ID.')
    if (listingError) expect(screen.getByRole('alert')).toHaveTextContent(listingError)
})

it('clears an earlier listing alert once a copy succeeds', async () => {
    const user = userEvent.setup()
    stubBackend({ replies: { [TOKENS]: () => json({ detail: 'tokens table missing' }, 500) } })
    stubClipboard({ writeText: vi.fn().mockResolvedValue(undefined) })
    render(card())

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not load peer tokens: Tokens table missing')
    await user.click(screen.getByRole('button', { name: 'Copy ID' }))
    await waitFor(() => expect(statusText()).toBe('Copied environment ID.'))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
})

it.each([
    ['shown', false],
    ['still loading', true],
])('drops Projects results %s for a link a refresh shows was replaced', async (_name, inFlight) => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    const url = `${LINKS}/env-build/projects`
    let finishLookup = () => {}
    backend.replies[url] = () => inFlight
        ? new Promise<Response>((resolve) => { finishLookup = () => resolve(json([{ project_path: '/srv/old' }])) })
        : json([{ project_path: '/srv/old' }])
    render(card())
    await user.click(await screen.findByRole('button', { name: 'Projects on Build box' }))
    if (!inFlight) expect(await screen.findByText('/srv/old')).toBeInTheDocument()

    // Another tab relinks env-build, and a refresh here lists the replacement.
    backend.links = [relinked]
    backend.replies[url] = [{ project_path: '/srv/current' }]
    await user.click(screen.getByRole('button', { name: 'Refresh' }))
    expect(await screen.findByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()
    await act(async () => finishLookup())
    expect(screen.queryByText('/srv/old')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Projects on Build box' }))
    expect(await screen.findByText('/srv/current')).toBeInTheDocument()
    expect(screen.queryByText('/srv/old')).not.toBeInTheDocument()
})

it('keeps the rows of a listing that finished during a removal check that then finds the link changed', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    const view = render(card())
    await user.click(await screen.findByRole('button', { name: 'Remove link Build box' }))

    // Another tab relinks env-build before this one confirms; the removal's check is held.
    backend.links = [relinked]
    let finishCheck = () => {}
    backend.replies[LINKS] = () => {
        delete backend.replies[LINKS]
        const answer = json({ links: backend.links })
        return new Promise<Response>((resolve) => { finishCheck = () => resolve(answer) })
    }
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(backend.replies[LINKS]).toBeUndefined())

    // Meanwhile another tab links GPU, and returning to this tab lists it.
    backend.links = [...backend.links, link('env-gpu', 'GPU')]
    view.rerender(card(false))
    view.rerender(card(true))
    expect(await screen.findByRole('button', { name: 'Remove link GPU' })).toBeInTheDocument()

    await act(async () => finishCheck())
    expect(await screen.findByRole('alert')).toHaveTextContent('The link to Build box changed since you chose to remove it')
    expect(screen.getByRole('button', { name: 'Remove link GPU' })).toBeInTheDocument()
    expect(screen.getByText(/https:\/\/new-build\.example:8000/)).toBeInTheDocument()
    expect(backend.mutations).toEqual([])
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

/** Holds the first Projects lookup on env-build until released with a failure; later lookups list /srv/current. */
function holdFirstProjectsLookup(backend: ReturnType<typeof stubBackend>) {
    const url = `${LINKS}/env-build/projects`
    const held = { fail: () => {} }
    backend.replies[url] = () => {
        backend.replies[url] = [{ project_path: '/srv/current' }]
        return new Promise<Response>((resolve) => { held.fail = () => resolve(json({ detail: 'peer unreachable' }, 502)) })
    }
    return held
}

async function removeBuildBox(user: User) {
    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    await user.click(await screen.findByRole('button', { name: 'Remove link' }))
    await waitFor(() => expect(statusText()).toBe('Removed link to Build box.'))
}

it('ignores a Projects lookup that finishes after its link was removed and linked again', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    const obsolete = holdFirstProjectsLookup(backend)
    render(card())
    await user.click(await screen.findByRole('button', { name: 'Projects on Build box' }))
    await removeBuildBox(user)

    await user.type(screen.getByLabelText('Remote origin'), 'https://env-build.example:8000')
    await user.type(screen.getByLabelText('Remote environment ID'), 'env-build')
    await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
    await user.type(screen.getByLabelText('Link label (optional)'), 'Build box')
    await user.click(screen.getByRole('button', { name: 'Add link' }))
    await waitFor(() => expect(statusText()).toBe('Linked Build box.'))
    await user.click(await screen.findByRole('button', { name: 'Projects on Build box' }))
    expect(await screen.findByText('/srv/current')).toBeInTheDocument()

    await act(async () => obsolete.fail())
    expect(screen.getByText('/srv/current')).toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(statusText()).toBe('Linked Build box.')
})

it('ignores a Projects lookup that finishes after its link was removed', async () => {
    const user = userEvent.setup()
    const backend = stubBackend({ links: [link('env-build', 'Build box')] })
    const obsolete = holdFirstProjectsLookup(backend)
    render(card())
    await user.click(await screen.findByRole('button', { name: 'Projects on Build box' }))
    await removeBuildBox(user)

    await act(async () => obsolete.fail())
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(statusText()).toBe('Removed link to Build box.')
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
