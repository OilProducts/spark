import { act, render, renderHook, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '@/App'
import { DialogProvider } from '@/components/app/dialog-controller'
import { useStartPipeline } from '@/features/launch'
import { useRunJournalStore } from '@/features/runs/state/runJournalStore'
import {
    EXPECTED_PROTOCOL_VERSION,
    EXPECTED_UI_API_VERSION,
    getActiveEnvironmentId,
    getActiveEnvironmentState,
    resetActiveEnvironmentForTests,
} from '@/lib/activeEnvironment'
import { workspaceLiveEventsUrl, workspaceUrl } from '@/lib/api/apiClient'
import { attractorUrl } from '@/lib/api/attractorApi'
import { DEFAULT_ROUTE_STATE, ROUTE_STATE_STORAGE_KEY, saveRouteState } from '@/state/store-helpers'
import { useStore } from '@/store'
import { deferred } from '@/test/environments'

class MockEventSource {
    static readonly instances: MockEventSource[] = []
    readonly url: string
    /** The environment active when it opened. */
    readonly environmentId = getActiveEnvironmentId()
    closed = false
    onopen: ((event: Event) => void) | null = null
    onmessage: ((event: MessageEvent<string>) => void) | null = null
    onerror: ((event: Event) => void) | null = null

    constructor(url: string | URL) {
        this.url = String(url)
        MockEventSource.instances.push(this)
    }

    close() {
        this.closed = true
    }
}

const resolveUrl = (input: RequestInfo | URL): string => {
    if (typeof input === 'string') return input
    if (input instanceof URL) return input.toString()
    return input.url
}

const json = (payload: unknown, status = 200) =>
    new Response(JSON.stringify(payload), {
        status,
        headers: { 'Content-Type': 'application/json' },
    })

const proxy = (environmentId: string) => `/workspace/api/environments/links/${environmentId}/proxy`
const WIN = proxy('env-win')
const WIN_AUTH = `${WIN}/workspace/api/environment`
const WIN_PUBLIC = `${WIN}/environment`
const GPU_AUTH = `${proxy('env-gpu')}/workspace/api/environment`
const LINKS = '/workspace/api/environments/links'
/** The only calls this machine's switcher makes while a linked environment is active. */
const LOCAL_BOOTSTRAP = ['/environment', LINKS]

const VERSIONS = { protocol_version: EXPECTED_PROTOCOL_VERSION, ui_api_version: EXPECTED_UI_API_VERSION }
const LOCAL_ENV = { environment_id: 'env-local', display_name: 'mac-studio', ...VERSIONS, spark_version: '0.3.0' }
const descriptor = (environmentId: string, overrides: Record<string, unknown> = {}) =>
    ({ environment_id: environmentId, display_name: `${environmentId}-box`, ...VERSIONS, spark_version: '0.3.0', ...overrides })
// The authenticated descriptor adds the build; the public one has none.
const WIN_ENV = descriptor('env-win', { build_id: '0.3.0+00ff' })

const LOCAL_PROJECT = {
    project_id: 'local-project',
    project_path: '/local/project',
    display_name: 'project',
    created_at: '2026-11-24T00:00:00Z',
    last_opened_at: '2026-11-24T00:00:00Z',
    last_accessed_at: null,
    is_favorite: false,
    active_conversation_id: null,
}

const link = (environment_id: string, label: string) =>
    ({ environment_id, origin: `http://${environment_id}:8000`, label, created_at: '2026-11-24T00:00:00Z' })
const WIN_LINK = link('env-win', 'Windows Workstation')
const GPU_LINK = link('env-gpu', 'GPU')

const CONNECTIONS = {
    connections: {
        revision: 'rev-1',
        stored: { server_host: '127.0.0.1', server_port: 8000, client_api_base_url: null },
        effective: { server_host: '127.0.0.1', server_port: 8000, client_api_base_url: 'http://127.0.0.1:8000' },
    },
}

type Route = (init?: RequestInit) => Response | Promise<Response>

/**
 * This machine and its linked environments behind their proxies. `routes` answers a URL ("METHOD url" for others than
 * GET) in place of the defaults; every request is recorded with the environment active when it was sent.
 */
function stubBackend(routes: Record<string, Route> = {}, links = [WIN_LINK]) {
    const requests: { url: string; method: string; environmentId: string | null }[] = []
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = resolveUrl(input)
        const method = init?.method ?? 'GET'
        requests.push({ url, method, environmentId: getActiveEnvironmentId() })
        const route = routes[method === 'GET' ? url : `${method} ${url}`]
        if (route) return route(init)
        const remote = /^\/workspace\/api\/environments\/links\/([^/]+)\/proxy(\/.*)$/.exec(url)
        const path = remote ? remote[2] : url
        if (url === '/environment') return json(LOCAL_ENV)
        if (url === LINKS) return json({ links })
        if (remote && path === '/workspace/api/environment') return json(remote[1] === 'env-win' ? WIN_ENV : descriptor(remote[1]))
        if (remote && path === '/environment') return json(descriptor(remote[1]))
        if (path.startsWith('/workspace/api/definitions?')) return json({ items: [] })
        if (path === '/attractor/runs') return json({ runs: [] })
        if (path === '/workspace/api/projects') return json(remote ? [] : [LOCAL_PROJECT])
        if (path === '/workspace/api/projects/metadata' || path.startsWith('/workspace/api/projects/metadata?')) return json({ branch: 'main' })
        if (path === '/workspace/api/settings') return json(CONNECTIONS)
        if (path === '/workspace/api/triggers' || path === '/workspace/api/trigger-issues') return json([])
        if (path === '/workspace/api/attention') return json({ items: [] })
        return json({})
    }))
    return {
        requests,
        /** Requests sent to `environmentId`'s proxy. */
        sentTo: (environmentId: string) => requests.filter((request) => request.url.startsWith(`${proxy(environmentId)}/`)).map((request) => request.url),
        /** Unprefixed requests, other than this machine's switcher bootstrap, sent while a linked environment was active. */
        leakedLocal: () => requests.filter((request) => request.environmentId !== null
            && !request.url.startsWith(`${proxy(request.environmentId)}/`) && !LOCAL_BOOTSTRAP.includes(request.url)),
    }
}

const tab = (environmentId: string) => screen.getByTestId(`environment-tab-${environmentId}`)
const tabs = () => within(screen.getByRole('tablist', { name: 'Spark environments' })).getAllByRole('tab').map((entry) => entry.textContent)
const workspaceMounted = () => screen.getByTestId('app-shell')

async function openSystemSettings() {
    act(() => {
        useStore.getState().setSettingsCategory('system')
        useStore.getState().setViewMode('settings')
    })
    expect(await screen.findByTestId('settings-panel')).toBeVisible()
}

describe('Same-window multi-environment switching', () => {
    beforeEach(() => {
        resetActiveEnvironmentForTests()
        MockEventSource.instances.length = 0
        window.history.replaceState(null, '', window.location.pathname + window.location.search)
        saveRouteState({ ...DEFAULT_ROUTE_STATE, activeProjectPath: '/local/project' })
        useStore.setState({
            viewMode: 'runs',
            activeProjectPath: '/local/project',
            projectRegistry: {
                '/local/project': {
                    directoryPath: '/local/project',
                    isFavorite: false,
                    lastAccessedAt: null,
                },
            },
        })
        vi.stubGlobal('EventSource', MockEventSource as unknown as typeof EventSource)
    })

    afterEach(() => {
        resetActiveEnvironmentForTests()
        MockEventSource.instances.length = 0
        delete window.__TAURI__
        vi.restoreAllMocks()
        vi.unstubAllGlobals()
    })

    it('hides the switcher when no linked environments exist', async () => {
        stubBackend({}, [])
        render(<App />)
        await waitFor(() => {
            expect(screen.getByTestId('app-shell')).toBeVisible()
        })
        expect(screen.queryByTestId('environment-switcher')).not.toBeInTheDocument()
    })

    it('keeps the workspace mounted while it checks a compatible remote, then sends every request through its proxy, hides Desktop Server and switches back', async () => {
        const user = userEvent.setup()
        const invokedTauriCommands: string[] = []
        window.__TAURI__ = {
            core: {
                invoke: vi.fn(async (command: string) => {
                    invokedTauriCommands.push(command)
                    if (command === 'desktop_client_identity') return 'desktop-test'
                    return {
                        remote_access_enabled: false,
                        bind_host: '127.0.0.1',
                        server_url: 'http://127.0.0.1:8000',
                        requires_restart: false,
                        revision: 'rev-1',
                        remote_access_warning: 'warning',
                    }
                }) as NonNullable<NonNullable<Window['__TAURI__']>['core']>['invoke'],
            },
        }
        const check = deferred<void>()
        const backend = stubBackend({
            [WIN_AUTH]: async () => {
                await check.promise
                return json(WIN_ENV)
            },
        })

        render(<App />)

        const remoteTab = await screen.findByTestId('environment-tab-env-win')
        expect(tab('local')).toHaveAttribute('aria-selected', 'true')
        expect(workspaceUrl('/projects')).toBe('/workspace/api/projects')
        expect(attractorUrl('/runs')).toBe('/attractor/runs')
        const localShell = workspaceMounted()

        await user.click(remoteTab)
        // Checking shows in the switcher bar; this machine's workspace stays mounted and visible meanwhile.
        expect(await screen.findByTestId('environment-checking')).toHaveTextContent('Connecting to Windows Workstation…')
        expect(workspaceMounted()).toBe(localShell)
        expect(localShell).toBeVisible()
        expect(getActiveEnvironmentId()).toBeNull()

        await act(async () => check.resolve())
        await waitFor(() => {
            expect(remoteTab).toHaveAttribute('aria-selected', 'true')
            expect(screen.getByTestId('remote-environment-indicator')).toHaveTextContent(/^Remote · http:\/\/env-win:8000\s*· UI API v2$/)
        })
        // The authenticated descriptor's build shows in the indicator's tooltip.
        expect(screen.getByTestId('remote-environment-indicator')).toHaveAttribute('title', expect.stringContaining('build 0.3.0+00ff'))
        expect(workspaceMounted()).not.toBe(localShell)

        expect(workspaceUrl('/projects')).toBe('/workspace/api/environments/links/env-win/proxy/workspace/api/projects')
        expect(attractorUrl('/runs')).toBe('/workspace/api/environments/links/env-win/proxy/attractor/runs')
        expect(workspaceLiveEventsUrl(new URLSearchParams({ include_runs_overview: 'true' }))).toBe(
            '/workspace/api/environments/links/env-win/proxy/workspace/api/live/events?include_runs_overview=true',
        )

        // Store was reset for remote environment while localStorage kept the local project selection.
        expect(useStore.getState().activeProjectPath).toBeNull()
        const savedLocalRoute = JSON.parse(window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY) ?? '{}')
        expect(savedLocalRoute.activeProjectPath).toBe('/local/project')

        // Live EventSource reconnected through the proxy and API requests went through the proxy.
        await waitFor(() => {
            expect(MockEventSource.instances.some((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`))).toBe(true)
            expect(backend.sentTo('env-win').some((url) => url.startsWith(`${WIN}/attractor/runs`))).toBe(true)
        })

        // Local-only Tauri Desktop Server card is hidden while viewing a remote environment.
        await openSystemSettings()
        expect(screen.queryByRole('heading', { name: 'Desktop server' })).not.toBeInTheDocument()
        expect(invokedTauriCommands).not.toContain('desktop_server_settings')
        await waitFor(() => expect(backend.sentTo('env-win')).toContain(`${WIN}/workspace/api/settings`))

        // Nothing but this machine's switcher bootstrap left unprefixed while the remote was active.
        expect(backend.leakedLocal()).toEqual([])
        expect(MockEventSource.instances.filter((source) => source.environmentId === 'env-win' && !source.url.startsWith(`${WIN}/`))).toEqual([])

        // Switching back to local restores unprefixed APIs and local route state.
        await user.click(tab('local'))
        await waitFor(() => {
            expect(tab('local')).toHaveAttribute('aria-selected', 'true')
        })
        expect(workspaceUrl('/projects')).toBe('/workspace/api/projects')
        expect(attractorUrl('/runs')).toBe('/attractor/runs')
        expect(useStore.getState().activeProjectPath).toBe('/local/project')
    })

    it('commits only the newest switch when an earlier check answers after it', async () => {
        const user = userEvent.setup()
        const win = deferred<void>()
        const gpu = deferred<void>()
        const routes: Record<string, Route> = {
            [WIN_AUTH]: () => win.promise.then(() => json(WIN_ENV)),
            [GPU_AUTH]: () => gpu.promise.then(() => json(descriptor('env-gpu'))),
        }
        const backend = stubBackend(routes, [WIN_LINK, GPU_LINK])
        render(<App />)

        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await user.click(tab('env-gpu'))
        await act(async () => gpu.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-gpu'))
        await act(async () => win.resolve())
        expect(getActiveEnvironmentId()).toBe('env-gpu')
        expect(tab('env-gpu')).toHaveAttribute('aria-selected', 'true')
        expect(backend.sentTo('env-win')).toEqual([WIN_AUTH])

        // Returning to this machine while a check is pending drops it too.
        const again = deferred<void>()
        routes[WIN_AUTH] = () => again.promise.then(() => json(WIN_ENV))
        await user.click(tab('env-win'))
        await user.click(tab('local'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBeNull())
        await act(async () => again.resolve())
        expect(getActiveEnvironmentId()).toBeNull()
        expect(tab('local')).toHaveAttribute('aria-selected', 'true')
        expect(getActiveEnvironmentState().status).toBe('ready')
    })

    const remoteAnswers = (auth: Response | null, publicDescriptor: unknown = descriptor('env-win')) => ({
        [WIN_AUTH]: () => auth ?? json({ detail: 'Not Found' }, 404),
        [WIN_PUBLIC]: () => json(publicDescriptor),
    })
    it.each([
        ['a missing ui_api_version', remoteAnswers(json(descriptor('env-win', { ui_api_version: undefined }))), 'Version mismatch with', /unversioned UI API · protocol v1/],
        ['an invalid ui_api_version', remoteAnswers(json(descriptor('env-win', { ui_api_version: '2' }))), 'Version mismatch with', /unversioned UI API/],
        ['a mismatched ui_api_version', remoteAnswers(json(descriptor('env-win', { ui_api_version: 3 }))), 'Version mismatch with', /Windows Workstation runs UI API v3/],
        ['a missing protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: undefined }))), 'Version mismatch with', /UI API v2 · unversioned protocol/],
        ['an invalid protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: 'v1' }))), 'Version mismatch with', /unversioned protocol/],
        ['a mismatched protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: 2 }))), 'Version mismatch with', /UI API v2 · protocol v2/],
        ['an older peer without the authenticated descriptor', remoteAnswers(null, descriptor('env-win', { ui_api_version: undefined, protocol_version: 1 })), 'Version mismatch with', /Windows Workstation runs unversioned UI API/],
        ['an older peer the proxy refuses for its missing identity', remoteAnswers(json({ detail: 'http://env-win:8000 answered as environment None, not the linked "env-win"; link it again if it moved.' }, 502), descriptor('env-win', { ui_api_version: 1 })), 'Version mismatch with', /UI API v1/],
        ['a peer whose only compatible descriptor is the public one', remoteAnswers(null), 'Couldn’t connect to', /Not Found/],
        ['a wrong environment_id', remoteAnswers(json(descriptor('env-other'))), 'Couldn’t connect to', /answered as environment env-other, not the linked env-win\. Remove the link in Settings and pair again/],
        ['a revoked bearer', remoteAnswers(json({ detail: 'Unauthorized' }, 401)), 'Couldn’t connect to', /refused this Spark’s credential \(HTTP 401\).*pair again with a new pairing code/],
    ])('refuses %s, never mounting the remote and keeping this machine\'s workspace', async (_name, routes, heading, detail) => {
        const user = userEvent.setup()
        const backend = stubBackend(routes)
        render(<App />)
        const localShell = workspaceMounted()
        await user.click(await screen.findByTestId('environment-tab-env-win'))

        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent(`${heading} Windows Workstation`)
        expect(gate).toHaveTextContent(detail)
        // Hidden, not unmounted, and never sent a workspace request.
        expect(workspaceMounted()).toBe(localShell)
        expect(localShell).not.toBeVisible()
        expect(getActiveEnvironmentId()).toBeNull()
        expect(backend.sentTo('env-win').filter((url) => url !== WIN_AUTH && url !== WIN_PUBLIC)).toEqual([])

        await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
        expect(workspaceMounted()).toBe(localShell)
        expect(localShell).toBeVisible()
        expect(tab('local')).toHaveAttribute('aria-selected', 'true')
    })

    it('refuses a stale UI bundle even when this machine and the remote were upgraded together', async () => {
        const user = userEvent.setup()
        stubBackend({
            '/environment': () => json({ ...LOCAL_ENV, ui_api_version: 3 }),
            [WIN_AUTH]: () => json(descriptor('env-win', { ui_api_version: 3 })),
        })
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))

        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        expect(gate).toHaveTextContent('This page was loaded for UI API v2 · protocol v1, but this Spark now runs UI API v3')
        expect(gate).toHaveTextContent(/Reload the page/)
        expect(getActiveEnvironmentId()).toBeNull()
    })

    it('shows an error screen when remote is unreachable and recovers on Retry', async () => {
        const user = userEvent.setup()
        let remoteOnline = false
        stubBackend({ [WIN_AUTH]: () => remoteOnline ? json(WIN_ENV) : json({ detail: 'connection refused' }, 503) })

        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))

        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to Windows Workstation')
        expect(gate).toHaveTextContent(/connection refused/i)

        remoteOnline = true
        await user.click(screen.getByRole('button', { name: 'Retry' }))

        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(workspaceMounted()).toBeVisible()
        expect(screen.getByTestId('remote-environment-indicator')).toBeVisible()
    })

    it.each([
        ['upgraded past this bundle', () => json(descriptor('env-win', { ui_api_version: 3 })), 'Version mismatch with Windows Workstation'],
        ['re-paired as another environment', () => json(descriptor('env-other')), 'answered as environment env-other'],
        ['revoking this Spark\'s credential', () => json({ detail: 'Unauthorized' }, 401), 'refused this Spark’s credential'],
    ])('leaves ready for the gate when the remote\'s live connection comes back %s', async (_name, answer, message) => {
        const user = userEvent.setup()
        const routes: Record<string, Route> = {}
        stubBackend(routes)
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        const remoteShell = workspaceMounted()
        const live = () => MockEventSource.instances.filter((source) => !source.closed && source.url.startsWith(`${WIN}/workspace/api/live/events`)).at(-1)
        await waitFor(() => expect(live()).toBeDefined())
        // An open after no error checks nothing again.
        const opened = live()!
        act(() => opened.onopen?.(new Event('open')))
        expect(getActiveEnvironmentState().status).toBe('ready')

        routes[WIN_AUTH] = answer
        act(() => opened.onerror?.(new Event('error')))
        await waitFor(() => expect(live()).toBeDefined(), { timeout: 2000 })
        expect(live()).not.toBe(opened)
        act(() => live()!.onopen?.(new Event('open')))

        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent(message)
        expect(getActiveEnvironmentState().status).not.toBe('ready')
        expect(getActiveEnvironmentId()).toBe('env-win')
        expect(workspaceMounted()).toBe(remoteShell)
        expect(remoteShell).not.toBeVisible()

        // Retry checks it again and shows the same workspace once it passes.
        routes[WIN_AUTH] = () => json(WIN_ENV)
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        expect(workspaceMounted()).toBe(remoteShell)
    })

    describe.each([
        ['the tab bar', (user: ReturnType<typeof userEvent.setup>) => user.click(tab('env-win'))],
        ['Settings', (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))],
    ])('switching from %s with unsaved settings', (_source, requestSwitch) => {
        const draftField = () => screen.getByLabelText('Command-line server URL')
        async function renderWithDraft(routes: Record<string, Route> = {}) {
            const user = userEvent.setup()
            const backend = stubBackend(routes)
            render(<App />)
            await screen.findByTestId('environment-tab-env-win')
            await openSystemSettings()
            await screen.findByRole('button', { name: 'Switch to Windows Workstation' })
            await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')
            return { user, backend, shell: workspaceMounted() }
        }
        const resets = () => vi.mocked(useRunJournalStore.setState).mock.calls.filter(([update]) => JSON.stringify(update) === '{"byRunId":{}}').length

        it('keeps this machine\'s workspace and its draft when the guard is cancelled', async () => {
            const { user, backend, shell } = await renderWithDraft()
            await requestSwitch(user)
            const dialog = await screen.findByRole('alertdialog')
            expect(dialog).toHaveTextContent('Discard unsaved settings?')
            await user.click(within(dialog).getByRole('button', { name: 'Keep editing' }))

            expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' })
            expect(tab('local')).toHaveAttribute('aria-selected', 'true')
            expect(workspaceMounted()).toBe(shell)
            expect(shell).toBeVisible()
            expect(draftField()).toHaveValue('http://draft.example')
            expect(backend.sentTo('env-win')).toEqual([WIN_AUTH])
        })

        it('keeps this machine\'s workspace and its draft when the check fails', async () => {
            const { user, shell } = await renderWithDraft({ [WIN_AUTH]: () => json({ detail: 'connection refused' }, 503) })
            await requestSwitch(user)
            await user.click(await screen.findByRole('button', { name: 'Return to this machine' }))

            expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
            expect(workspaceMounted()).toBe(shell)
            expect(shell).toBeVisible()
            expect(draftField()).toHaveValue('http://draft.example')
        })

        it('asks about an edit made during the check before unmounting, and commits once when approved', async () => {
            const user = userEvent.setup()
            const check = deferred<void>()
            stubBackend({ [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)) })
            vi.spyOn(useRunJournalStore, 'setState')
            render(<App />)
            await screen.findByTestId('environment-tab-env-win')
            await openSystemSettings()
            await screen.findByRole('button', { name: 'Switch to Windows Workstation' })
            await screen.findByLabelText('Command-line server URL')
            const shell = workspaceMounted()

            // The check starts with nothing unsaved; the workspace stays usable while it runs.
            await requestSwitch(user)
            expect(await screen.findByTestId('environment-checking')).toBeInTheDocument()
            await user.type(draftField(), 'http://draft.example')
            await act(async () => check.resolve())

            const dialog = await screen.findByRole('alertdialog')
            expect(dialog).toHaveTextContent('Discard unsaved settings?')
            expect(workspaceMounted()).toBe(shell)
            expect(getActiveEnvironmentId()).toBeNull()
            expect(resets()).toBe(0)

            await user.click(within(dialog).getByRole('button', { name: 'Discard and leave' }))
            await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
            expect(tab('env-win')).toHaveAttribute('aria-selected', 'true')
            expect(workspaceMounted()).not.toBe(shell)
            expect(resets()).toBe(1)
        })
    })

    it('updates the switcher tabs as soon as Settings adds or removes a link, before its links reload', async () => {
        const user = userEvent.setup()
        let links = [WIN_LINK]
        let holdListings = false
        const held: Array<() => void> = []
        stubBackend({
            [LINKS]: () => holdListings
                ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) })
                : json({ links }),
            [`POST ${LINKS}`]: (init) => {
                const body = JSON.parse(String(init?.body)) as { expected_id: string; label: string }
                const created = link(body.expected_id, body.label)
                links = [...links, created]
                holdListings = true
                return json(created)
            },
            [`DELETE ${LINKS}/env-win`]: () => {
                links = links.filter((row) => row.environment_id !== 'env-win')
                holdListings = true
                return json({ environment_id: 'env-win', removed: true })
            },
        })
        const release = () => {
            holdListings = false
            held.splice(0).forEach((resolve) => resolve())
        }
        render(<App />)
        await waitFor(() => expect(tabs()).toEqual(['mac-studio(local)', 'Windows Workstation']))
        await openSystemSettings()

        await user.type(await screen.findByLabelText('Remote origin'), 'http://env-gpu:8000')
        await user.type(screen.getByLabelText('Remote environment ID'), 'env-gpu')
        await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
        await user.type(screen.getByLabelText('Link label (optional)'), 'GPU')
        await user.click(screen.getByRole('button', { name: 'Add link' }))
        await waitFor(() => expect(held.length).toBeGreaterThan(0))
        expect(tabs()).toEqual(['mac-studio(local)', 'Windows Workstation', 'GPU'])
        await act(async () => release())

        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(held.length).toBeGreaterThan(0))
        expect(tabs()).toEqual(['mac-studio(local)', 'GPU'])
        await act(async () => release())
        expect(tabs()).toEqual(['mac-studio(local)', 'GPU'])
    })

    it.each(['launch', 'continuation'])('lets a %s answered during another environment\'s check land in its still-open workspace, and drops one answered after the switch', async (kind) => {
        const user = userEvent.setup()
        const checked = deferred<void>()
        const acks: Array<ReturnType<typeof deferred<Response>>> = []
        const requested: string[] = []
        const backend = stubBackend({ [WIN_AUTH]: () => checked.promise.then(() => json(WIN_ENV)) })
        const backendFetch = vi.mocked(fetch).getMockImplementation()!
        vi.mocked(fetch).mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
            const url = resolveUrl(input)
            if (init?.method === 'POST' && /\/attractor\/pipelines(\/[^/]+\/continue)?$/.test(url)) {
                requested.push(url)
                acks.push(deferred<Response>())
                return acks.at(-1)!.promise
            }
            return backendFetch(input, init)
        })
        render(<App />)
        act(() => useStore.getState().setViewMode('home'))
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        const { result } = renderHook(() => useStartPipeline(), { wrapper: DialogProvider })
        const form = { projectPath: '/local/project', workingDirectory: '/local/project', flowSource: 'flow', model: null }
        const start = () => kind === 'launch'
            ? result.current.startFlow(form)
            : result.current.continueFromRun('source', form, { startNodeId: 'review', flowSourceMode: 'snapshot' })
        const launches = [start(), start()]
        await waitFor(() => expect(acks).toHaveLength(2))

        // Checking the remote leaves this machine's workspace open, so a start answered meanwhile shows there.
        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')
        await act(async () => {
            acks[0].resolve(json({ status: 'started', pipeline_id: 'first-ack' }, 202))
            await expect(launches[0]).resolves.toMatchObject({ status: 'launched', runId: 'first-ack' })
        })
        expect(useStore.getState().runsListSession.selectedRunId).toBe('first-ack')

        // Once the switch commits, the other answers into nothing.
        await act(async () => checked.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        await act(async () => {
            acks[1].resolve(json({ status: 'started', pipeline_id: 'second-ack' }, 202))
            await expect(launches[1]).resolves.toEqual({ status: 'cancelled' })
        })
        expect(useStore.getState().runsListSession.selectedRunId).not.toBe('second-ack')
        // Both went to this machine, where they began.
        expect(requested.every((url) => url.startsWith('/attractor/pipelines'))).toBe(true)
        expect(backend.leakedLocal()).toEqual([])
    })
})
