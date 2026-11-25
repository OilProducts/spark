import { act, render, renderHook, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '@/App'
import { DialogProvider } from '@/components/app/dialog-controller'
import { useStartPipeline } from '@/features/launch'
import { RUNS_TRANSPORT_LOST_EVENT, requestRunsTransportReconnect } from '@/features/runs/services/runsTransportReconnect'
import { useRunJournalStore } from '@/features/runs/state/runJournalStore'
import {
    ENVIRONMENT_LINKS_CHANGED_EVENT,
    EXPECTED_PROTOCOL_VERSION,
    EXPECTED_UI_API_VERSION,
    getActiveEnvironmentId,
    getActiveEnvironmentState,
    resetActiveEnvironmentForTests,
} from '@/lib/activeEnvironment'
import { workspaceLiveEventsUrl, workspaceUrl } from '@/lib/api/apiClient'
import { attractorUrl } from '@/lib/api/attractorApi'
import { DEFAULT_ROUTE_STATE, ROUTE_STATE_STORAGE_KEY, saveRouteState, saveSessionViewMode } from '@/state/store-helpers'
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
/** This Spark's proxy refusing an answer at env-win's origin that names `answeredAs`, or none, as crates/spark-http/src/delegations.rs does. */
const proxyRefusal = (answeredAs: string | null) => json({
    detail: `http://env-win:8000 answered as environment ${answeredAs === null ? 'None' : `Some("${answeredAs}")`}, not the linked "env-win"; link it again if it moved.`,
    answered_as: answeredAs,
}, 502)
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
/** env-win linked anew, as at another origin: another row for the same environment ID. */
const WIN_RELINKED = { ...WIN_LINK, origin: 'http://env-win-new:8000', created_at: '2026-11-25T00:00:00Z' }

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
            && !request.url.startsWith(`${proxy(request.environmentId)}/`)
            && !(request.method === 'GET' && LOCAL_BOOTSTRAP.includes(request.url))),
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
        ['a mismatched ui_api_version while this machine gives no descriptor', { ...remoteAnswers(json(descriptor('env-win', { ui_api_version: 3 }))), '/environment': () => json({ detail: 'unavailable' }, 503) }, 'Version mismatch with', /Windows Workstation runs UI API v3 · protocol v1.*, while this page was loaded for UI API v2 · protocol v1/],
        ['a missing protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: undefined }))), 'Version mismatch with', /UI API v2 · unversioned protocol/],
        ['an invalid protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: 'v1' }))), 'Version mismatch with', /unversioned protocol/],
        ['a mismatched protocol_version', remoteAnswers(json(descriptor('env-win', { protocol_version: 2 }))), 'Version mismatch with', /UI API v2 · protocol v2/],
        ['an older peer without the authenticated descriptor', remoteAnswers(null, descriptor('env-win', { ui_api_version: undefined, protocol_version: 1 })), 'Version mismatch with', /Windows Workstation runs unversioned UI API/],
        ['an older peer the proxy refuses for its missing identity', remoteAnswers(proxyRefusal(null), descriptor('env-win', { ui_api_version: 1 })), 'Version mismatch with', /UI API v1/],
        ['another environment the proxy refuses at the link\'s origin', { [WIN_AUTH]: () => proxyRefusal('env-other'), [WIN_PUBLIC]: () => proxyRefusal('env-other') }, 'Couldn’t connect to', /answered as environment env-other, not the linked env-win\. Remove the link in Settings and pair again/],
        ['a peer whose only compatible descriptor is the public one', remoteAnswers(null), 'Couldn’t connect to', /Not Found/],
        ['a wrong environment_id', remoteAnswers(json(descriptor('env-other'))), 'Couldn’t connect to', /answered as environment env-other, not the linked env-win\. Remove the link in Settings and pair again/],
        ['a revoked bearer', remoteAnswers(json({ detail: 'Unauthorized' }, 401)), 'Couldn’t connect to', /refused this Spark’s credential \(HTTP 401\).*pair again with a new pairing code/],
        ['a revoked bearer while this machine gives no descriptor', { ...remoteAnswers(json({ detail: 'Unauthorized' }, 401)), '/environment': () => json({ detail: 'unavailable' }, 503) }, 'Couldn’t connect to', /refused this Spark’s credential \(HTTP 401\)/],
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

    it.each([
        ['before this page loaded', 3, 3],
        ['after this page loaded, against a remote still on this bundle\'s versions', 2, 2],
    ])('refuses a stale UI bundle when this machine was upgraded %s', async (_when, bootstrapUi, remoteUi) => {
        const user = userEvent.setup()
        const routes: Record<string, Route> = {
            '/environment': () => json({ ...LOCAL_ENV, ui_api_version: bootstrapUi }),
            [WIN_AUTH]: () => json(descriptor('env-win', { ui_api_version: remoteUi })),
        }
        stubBackend(routes)
        render(<App />)
        await waitFor(() => expect(getActiveEnvironmentState().localDescriptor?.ui_api_version).toBe(bootstrapUi))
        routes['/environment'] = () => json({ ...LOCAL_ENV, ui_api_version: 3 })
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

    const upgradedLocal = () => json({ ...LOCAL_ENV, ui_api_version: 3 })
    it.each([
        // Restarted as an upgrade: down for one reconnect, which keeps the workspace, then open again.
        ['comes back upgraded past this bundle', WIN_AUTH, () => json(descriptor('env-win', { ui_api_version: 3 })), true, 'Version mismatch with Windows Workstation'],
        ['comes back re-paired as another environment', WIN_AUTH, () => json(descriptor('env-other')), true, 'answered as environment env-other'],
        ['comes back after this Spark was upgraded under the page', '/environment', upgradedLocal, true, 'Reload the page'],
        // A revoked credential refuses the reconnect's handshake itself, which never opens.
        ['is refused for a revoked credential', WIN_AUTH, () => json({ detail: 'Unauthorized' }, 401), false, 'refused this Spark’s credential'],
    ])('leaves ready for the gate when the remote\'s live connection %s', async (_name, route, answer, reopens, message) => {
        const user = userEvent.setup()
        const routes: Record<string, Route> = {}
        const backend = stubBackend(routes)
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        const remoteShell = workspaceMounted()
        const live = () => MockEventSource.instances.filter((source) => !source.closed && source.url.startsWith(`${WIN}/workspace/api/live/events`)).at(-1)
        const checks = () => backend.requests.filter((request) => request.url === WIN_AUTH).length
        await waitFor(() => expect(live()).toBeDefined())
        // An open after no error checks nothing again.
        let source = live()!
        const before = checks()
        act(() => source.onopen?.(new Event('open')))
        expect(checks()).toBe(before)

        if (reopens) {
            // No answer while it restarts leaves the reconnects to retry, the workspace still shown.
            routes[WIN_AUTH] = () => json({ detail: 'connection refused' }, 503)
            act(() => source.onerror?.(new Event('error')))
            await waitFor(() => expect(checks()).toBe(before + 1))
            await act(async () => {})
            expect(getActiveEnvironmentState().status).toBe('ready')
            expect(remoteShell).toBeVisible()
            await waitFor(() => expect(live()).toBeDefined(), { timeout: 2000 })
            expect(live()).not.toBe(source)
            source = live()!
            delete routes[WIN_AUTH]
            routes[route] = answer
            act(() => source.onopen?.(new Event('open')))
        } else {
            routes[route] = answer
            act(() => source.onerror?.(new Event('error')))
        }

        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent(message)
        expect(getActiveEnvironmentState().status).not.toBe('ready')
        expect(getActiveEnvironmentId()).toBe('env-win')
        expect(workspaceMounted()).toBe(remoteShell)
        expect(remoteShell).not.toBeVisible()

        // Retry checks it again and shows the same workspace once it passes.
        delete routes[route]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        expect(workspaceMounted()).toBe(remoteShell)
    })

    const winLive = () => MockEventSource.instances.filter((source) => !source.closed && source.url.startsWith(`${WIN}/workspace/api/live/events`)).at(-1)
    async function mountWindows(routes: Record<string, Route> = {}, links?: typeof WIN_LINK[]) {
        const user = userEvent.setup()
        const backend = stubBackend(routes, links)
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        await waitFor(() => expect(winLive()).toBeDefined())
        return { user, backend, remoteShell: workspaceMounted(), checks: () => backend.requests.filter((request) => request.url === WIN_AUTH).length }
    }

    it.each([
        ['refuse a revoked credential, though this machine gives no descriptor', { [WIN_AUTH]: () => json({ detail: 'Unauthorized' }, 401), '/environment': () => json({ detail: 'unavailable' }, 503) }, 'refused this Spark’s credential'],
        ['speak a version this bundle doesn\'t', { [WIN_AUTH]: () => json(descriptor('env-win', { ui_api_version: 3 })) }, 'Version mismatch with Windows Workstation'],
        ['answer as another environment', { [WIN_AUTH]: () => json(descriptor('env-other')) }, 'answered as environment env-other'],
        ['are refused by the proxy for another environment answering', { [WIN_AUTH]: () => proxyRefusal('env-other'), [WIN_PUBLIC]: () => proxyRefusal('env-other') }, 'answered as environment env-other'],
        ['speak a version this bundle doesn\'t, though this machine gives no descriptor', { [WIN_AUTH]: () => json(descriptor('env-win', { ui_api_version: 3 })), '/environment': () => json({ detail: 'unavailable' }, 503) }, 'Version mismatch with Windows Workstation'],
        ['answer with no identity, while the public descriptor names another environment', { [WIN_AUTH]: () => proxyRefusal(null), [WIN_PUBLIC]: () => json(descriptor('env-other')) }, 'answered as environment env-other'],
        ['answer with no identity, while the proxy refuses the public descriptor for another environment', { [WIN_AUTH]: () => proxyRefusal(null), [WIN_PUBLIC]: () => proxyRefusal('env-other') }, 'answered as environment env-other'],
    ])('stops reconnecting a remote whose live handshakes never open and whose checks %s, until Retry passes a check', async (_finding, failures: Record<string, Route>, message) => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        // An outage first: its check gets no answer, and the connection is tried again with the workspace still shown.
        routes[WIN_AUTH] = () => json({ detail: 'connection refused' }, 503)
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        expect(remoteShell).toBeVisible()

        // Then every handshake fails; the first failure's check answers late, so the connection is tried again meanwhile.
        const late = deferred<Response>()
        Object.assign(routes, failures, { [WIN_AUTH]: () => late.promise })
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        act(() => winLive()!.onerror?.(new Event('error')))
        routes[WIN_AUTH] = failures[WIN_AUTH]
        await act(async () => late.resolve(await failures[WIN_AUTH]()))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent(message)
        expect(getActiveEnvironmentState().status).not.toBe('ready')

        // Nothing is tried or checked again while it stays so.
        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        for (const url of Object.keys(failures)) delete routes[url]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it.each([
        ['refuses a revoked credential', () => json({ detail: 'Unauthorized' }, 401), 'refused this Spark’s credential'],
        ['speaks a version this bundle doesn\'t', () => json(descriptor('env-win', { ui_api_version: 3 })), 'Version mismatch with Windows Workstation'],
        ['is refused by the proxy for another environment answering', () => proxyRefusal('env-other'), 'answered as environment env-other'],
    ])('checks a remote a gateway outage hid again as its handshakes keep failing, and stops reconnecting once it %s, until Retry passes', async (_finding, answer, message) => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        routes[WIN_AUTH] = () => proxyRefusal(null)
        routes[WIN_PUBLIC] = () => json({ detail: 'Bad gateway' }, 502)
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Couldn’t connect to Windows Workstation')
        expect(getActiveEnvironmentState().activeCheck.status).toBe('unreachable')

        // No handshake opens again before the next check finds a final failure.
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        routes[WIN_AUTH] = answer
        routes[WIN_PUBLIC] = answer
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent(message)
        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it.each([
        ['answers 503', () => json({ detail: 'connection refused' }, 503)],
        ['gives no answer', () => Promise.reject(new TypeError('Failed to fetch'))],
    ])('shows this Spark upgraded under the page, with reload guidance, and stops reconnecting, though the mounted remote %s, and keeps them once another tab links it anew', async (_peer, answer: Route) => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        routes['/environment'] = upgradedLocal
        routes[WIN_AUTH] = answer
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        expect(gate).toHaveTextContent('this Spark now runs UI API v3')
        expect(gate).toHaveTextContent('Reload the page')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible', activeCheck: { status: 'incompatible' } })
        expect(remoteShell).not.toBeVisible()

        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        // Another tab pairs it again while this page is stale: only a reload helps still.
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED]))
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 600)))
        expect(gate).toHaveTextContent('Reload the page')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible', activeCheck: { status: 'incompatible' } })
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
    })

    it('shows this Spark upgraded under the page, with reload guidance and no Retry, when the listing links the mounted remote anew while a reconnected connection\'s check, which the remote answers 503, still runs', async () => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // This Spark was upgraded under the page while Windows restarts: the reopened connection's check waits on its 503.
        const peer = deferred<void>()
        routes['/environment'] = upgradedLocal
        routes[WIN_AUTH] = () => peer.promise.then(() => json({ detail: 'connection refused' }, 503))
        const before = checks()
        const reopened = winLive()!
        act(() => reopened.onopen?.(new Event('open')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Another tab pairs Windows again; this tab's listing shows it while that check runs.
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Reload the page')

        await act(async () => peer.resolve())
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible', activeCheck: { status: 'incompatible' } })
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        expect(gate).toHaveTextContent('this Spark now runs UI API v3')
        expect(gate).toHaveTextContent('Reload the page')
        expect(remoteShell).not.toBeVisible()
        expect(reopened.closed).toBe(true)
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
    })

    it.each([
        ['a version mismatch only Windows has, then the unlink', 'peer-then-unlink', 'This Spark no longer links env-win', 'refused'],
        ['the unlink, while a check that finds a version mismatch only Windows has still runs', 'unlink-during-peer-check', 'This Spark no longer links env-win', 'refused'],
        ['this Spark\'s own version mismatch, then the unlink', 'local-then-unlink', 'Reload the page', 'incompatible'],
        ['the unlink, then this Spark\'s own version mismatch', 'unlink-then-local', 'Reload the page', 'incompatible'],
    ])('shows the guidance that still applies to a mounted remote after %s, its live connection closed', async (_order, order, message, status) => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        const gate = () => screen.getByTestId('remote-environment-gate')
        const peerMismatch = () => json(descriptor('env-win', { ui_api_version: 3 }))
        const unlink = async () => {
            links = []
            act(() => { window.dispatchEvent(new Event('focus')) })
            await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([]))
        }
        if (order === 'peer-then-unlink') {
            routes[WIN_AUTH] = peerMismatch
            act(() => winLive()!.onerror?.(new Event('error')))
            await waitFor(() => expect(gate()).toHaveTextContent('Update Spark so both environments match'))
            await unlink()
        } else if (order === 'unlink-during-peer-check') {
            const late = deferred<void>()
            routes[WIN_AUTH] = () => late.promise.then(peerMismatch)
            const before = checks()
            act(() => winLive()!.onerror?.(new Event('error')))
            await waitFor(() => expect(checks()).toBe(before + 1))
            await unlink()
            await act(async () => late.resolve())
        } else if (order === 'local-then-unlink') {
            routes['/environment'] = upgradedLocal
            routes[WIN_AUTH] = () => json({ detail: 'connection refused' }, 503)
            act(() => winLive()!.onerror?.(new Event('error')))
            await waitFor(() => expect(gate()).toHaveTextContent('Reload the page'))
            await unlink()
        } else {
            await unlink()
            await waitFor(() => expect(gate()).toHaveTextContent('This Spark no longer links'))
            routes['/environment'] = upgradedLocal
            act(() => { window.dispatchEvent(new Event('focus')) })
        }
        await waitFor(() => expect(gate()).toHaveTextContent(message))
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(gate()).toHaveTextContent(message)
        expect(gate()).not.toHaveTextContent('Update Spark so both environments match')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status, activeCheck: { status } })
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
    })

    it('says runs may be out of date once reconnected live connections keep failing their check with no answer, though none errors again', async () => {
        const routes: Record<string, Route> = {}
        const { remoteShell } = await mountWindows(routes)
        const lost = vi.fn()
        window.addEventListener(RUNS_TRANSPORT_LOST_EVENT, lost)
        routes[WIN_AUTH] = () => json({ detail: 'connection refused' }, 503)
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        expect(lost).not.toHaveBeenCalled()

        // Each connection that opens again fails its check, and closes before it delivers anything.
        for (const _attempt of [1, 2]) {
            const reopened = winLive()!
            act(() => reopened.onopen?.(new Event('open')))
            await waitFor(() => expect(reopened.closed).toBe(true))
            await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        }
        expect(lost).toHaveBeenCalledTimes(1)
        expect(remoteShell).toBeVisible()
        window.removeEventListener(RUNS_TRANSPORT_LOST_EVENT, lost)
    })

    it.each([
        ['its link is gone from this machine\'s listing', 'listing'],
        ['this Spark\'s proxy answers 404 for both its descriptors', 'proxy'],
    ])('leaves ready and stops reconnecting a mounted remote once %s, until Retry passes after linking it again', async (_unlinked, how) => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        if (how === 'listing') {
            links = []
            act(() => { window.dispatchEvent(new Event('focus')) })
        } else {
            const noLink = () => json({ detail: 'No link to environment "env-win".' }, 404)
            routes[WIN_AUTH] = noLink
            routes[WIN_PUBLIC] = noLink
            act(() => winLive()!.onerror?.(new Event('error')))
        }
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to')
        expect(gate).toHaveTextContent('This Spark no longer links')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(remoteShell).not.toBeVisible()

        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        // Linked again: Retry passes and the connection opens again.
        links = [WIN_LINK]
        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([WIN_LINK]))
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('keeps an unlinked remote behind its gate, its live connection closed, when another tab removes the link again while Retry checks it, and recovers on a Retry once linked again', async () => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        links = []
        act(() => { window.dispatchEvent(new Event('focus')) })
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('This Spark no longer links')

        // Linked again, then Retry: its check answers only later.
        links = [WIN_LINK]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([WIN_LINK]))
        const late = deferred<void>()
        routes[WIN_AUTH] = () => late.promise.then(() => json(WIN_ENV))
        const before = checks()
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Another tab removes the link while that check runs: its late success restores nothing.
        links = []
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([]))
        const tried = connects()
        await act(async () => late.resolve())
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 600)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(gate).toHaveTextContent('This Spark no longer links')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Linked again: Retry of the current link passes.
        links = [WIN_LINK]
        delete routes[WIN_AUTH]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([WIN_LINK]))
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('keeps a remote behind the unlinked gate, its live connection closed, once a reconnect\'s check finds both its descriptors gone, though a Retry sent before answers late, until Retry passes after linking it again', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        Object.assign(routes, gatewayDown)
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to Windows Workstation')
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // Retry during the outage: its check, sent while the link still exists, answers only later.
        const late = deferred<void>()
        routes[WIN_AUTH] = () => late.promise.then(() => json(WIN_ENV))
        const before = checks()
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Another tab removes the link, which this tab's listing doesn't show yet; the next failed connection's check
        // finds both descriptors missing at this Spark's proxy.
        const noLink = () => json({ detail: 'No link to environment "env-win".' }, 404)
        routes[WIN_AUTH] = noLink
        routes[WIN_PUBLIC] = noLink
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(gate).toHaveTextContent('This Spark no longer links'))

        await act(async () => late.resolve())
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(gate).toHaveTextContent('This Spark no longer links')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Linked again: a new Retry passes.
        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('keeps a remote behind the unlinked gate, its live connection closed, once a reconnect\'s check finds both its descriptors gone, though the check of a connection opened meanwhile passes after it is linked again, until Retry passes', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        // Another tab removes the link, which this tab's listing doesn't show yet: the failed connection's check finds
        // both descriptors missing at this Spark's proxy, and answers late.
        const noLink = () => json({ detail: 'No link to environment "env-win".' }, 404)
        const gone = deferred<void>()
        routes[WIN_AUTH] = () => gone.promise.then(noLink)
        routes[WIN_PUBLIC] = noLink
        const before = checks()
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Linked again meanwhile: the connection opens again, and its check, which passes, answers after that one.
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        const passes = deferred<void>()
        routes[WIN_AUTH] = () => passes.promise.then(() => json(WIN_ENV))
        act(() => winLive()!.onopen?.(new Event('open')))
        await waitFor(() => expect(checks()).toBe(before + 2))

        await act(async () => gone.resolve())
        const gate = await screen.findByTestId('remote-environment-gate')
        await waitFor(() => expect(gate).toHaveTextContent('This Spark no longer links'))
        const tried = connects()
        await act(async () => passes.resolve())
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(gate).toHaveTextContent('This Spark no longer links')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Only Retry lifts the gate.
        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('shows this Spark upgraded under the page, with reload guidance and its live connection closed, though a reconnect\'s check then finds both descriptors of the remote gone', async () => {
        const routes: Record<string, Route> = {}
        const { remoteShell } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        // This Spark, upgraded under the page, reports it to the switcher's next refresh.
        routes['/environment'] = upgradedLocal
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().localDescriptor?.ui_api_version).toBe(3))

        // Another tab removed the link, which this tab's listing doesn't show yet: the failed connection's check finds
        // both descriptors missing at this Spark's proxy.
        const noLink = () => json({ detail: 'No link to environment "env-win".' }, 404)
        routes[WIN_AUTH] = noLink
        routes[WIN_PUBLIC] = noLink
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        await waitFor(() => expect(gate).toHaveTextContent('Reload the page'))
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible', activeCheck: { status: 'incompatible' } })
        expect(gate).toHaveTextContent('this Spark now runs UI API v3')
        expect(gate).not.toHaveTextContent('This Spark no longer links')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
    })

    it('shows this Spark upgraded under the page, with reload guidance and its live connection closed, when a listing that gets no answer from this Spark finds the remote unlinked while a reconnect\'s check, which got this Spark\'s upgraded descriptor, waits on the remote\'s 503', async () => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { backend, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        const localAnswers = () => backend.requests.filter((request) => request.url === '/environment').length

        // The failed connection's check gets this Spark's upgraded descriptor; Windows answers 503 only later.
        const peer = deferred<void>()
        routes['/environment'] = upgradedLocal
        routes[WIN_AUTH] = () => peer.promise.then(() => json({ detail: 'connection refused' }, 503))
        const before = checks()
        const asked = localAnswers()
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))
        await waitFor(() => expect(localAnswers()).toBe(asked + 1))
        await act(() => new Promise((resolve) => setTimeout(resolve, 20)))

        // Another tab removes the link: this tab's next listing shows it, though this Spark doesn't answer for its version.
        routes['/environment'] = () => json({ detail: 'unavailable' }, 503)
        links = []
        act(() => { window.dispatchEvent(new Event('focus')) })
        const gate = await screen.findByTestId('remote-environment-gate')
        await waitFor(() => expect(gate).toHaveTextContent('Reload the page'))

        await act(async () => peer.resolve())
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible', activeCheck: { status: 'incompatible' } })
        expect(gate).toHaveTextContent('this Spark now runs UI API v3')
        expect(gate).not.toHaveTextContent('This Spark no longer links')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
    })

    it('keeps a remote behind its gate, its live connection closed, when the listing replaces its link while Retry checks the old one, until Retry passes for the current link', async () => {
        let links = [WIN_LINK]
        const routes: Record<string, Route> = { [LINKS]: () => json({ links }) }
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        Object.assign(routes, gatewayDown)
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to Windows Workstation')

        // Retry: the old link's check passes, but answers only once the listing shows the link replaced.
        const late = deferred<void>()
        routes[WIN_AUTH] = () => late.promise.then(() => json(WIN_ENV))
        const before = checks()
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(checks()).toBe(before + 1))
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED]))
        await act(async () => late.resolve())
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(gate).toHaveTextContent('linked anew since it was last checked')
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Only Retry checks the current link: incompatible, then passing.
        routes[WIN_AUTH] = () => json(descriptor('env-win', { ui_api_version: 3 }))
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(gate).toHaveTextContent('Version mismatch with Windows Workstation'))
        expect(winLive()).toBeUndefined()
        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('keeps a remote ready once Retry passes after pairing again during an outage, though an older check answers late with the revoked credential, and its live connection recovers', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        routes[WIN_AUTH] = () => proxyRefusal(null)
        routes[WIN_PUBLIC] = () => json({ detail: 'Bad gateway' }, 502)
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Couldn’t connect to Windows Workstation')
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // The next failure's check goes out with the credential that pairing again then replaces; it answers late.
        const before = checks()
        const revoked = deferred<Response>()
        routes[WIN_AUTH] = () => revoked.promise
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Paired again and back: Retry passes.
        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())

        await act(async () => revoked.resolve(json({ detail: 'Unauthorized' }, 401)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'ready', activeCheck: { status: 'ready' } })
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        expect(remoteShell).toBeVisible()

        // Its live connection opens again, passes its own check and applies what it sends.
        const applied = vi.fn()
        window.addEventListener('spark:harness-checked', applied)
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        const recovered = winLive()!
        act(() => {
            recovered.onopen?.(new Event('open'))
            recovered.onmessage?.(new MessageEvent('message', { data: JSON.stringify({ type: 'harness.checked' }) }))
        })
        await waitFor(() => expect(applied).toHaveBeenCalledTimes(1))
        window.removeEventListener('spark:harness-checked', applied)
        expect(getActiveEnvironmentState().status).toBe('ready')
        expect(remoteShell).toBeVisible()
    })

    const gatewayDown = { [WIN_AUTH]: () => proxyRefusal(null), [WIN_PUBLIC]: () => json({ detail: 'Bad gateway' }, 502) }
    it.each([
        ['finds a version this bundle doesn\'t speak', 'a gateway\'s answer for no Spark', () => json(descriptor('env-win', { ui_api_version: 3 })), 'Version mismatch with Windows Workstation', gatewayDown],
        ['finds its credential refused', 'a gateway\'s answer for no Spark', () => json({ detail: 'Unauthorized' }, 401), 'refused this Spark’s credential', gatewayDown],
        ['finds its credential refused', 'no answer', () => json({ detail: 'Unauthorized' }, 401), 'refused this Spark’s credential', { [WIN_AUTH]: () => json({ detail: 'connection refused' }, 503) }],
    ])('keeps a remote\'s live connection closed after a check %s, through a Retry that gets %s and a Run\'s journal, answered late, that moves it, until Retry passes', async (_finding, _inconclusive, failure, message, inconclusive: Record<string, Route>) => {
        const run = { run_id: 'win-run', flow_name: 'review.dot', status: 'running', outcome: null, working_directory: '/win/project', project_path: '/win/project', started_at: '2026-11-24T00:00:00Z', ended_at: null }
        const routes: Record<string, Route> = {
            [`${WIN}/attractor/runs`]: () => json({ runs: [run] }),
            [`${WIN}/attractor/pipelines/win-run`]: () => json({ ...run, pipeline_id: 'win-run' }),
        }
        const { user, remoteShell, checks } = await mountWindows(routes)
        // The Run's journal answers only once its fetch is let go.
        const journal = deferred<void>()
        const journalUrl = `${WIN}/attractor/pipelines/win-run/journal`
        let journalAsked = false
        const backendFetch = globalThis.fetch
        vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
            if (!resolveUrl(input).startsWith(journalUrl)) return backendFetch(input, init)
            journalAsked = true
            await journal.promise
            return json({ pipeline_id: 'win-run', entries: [], oldest_sequence: null, newest_sequence: null, has_older: false })
        })
        act(() => {
            useStore.getState().setViewMode('runs')
            useStore.getState().setRunsSelectedRunId('win-run')
        })
        await waitFor(() => expect(journalAsked).toBe(true))
        await waitFor(() => expect(winLive()).toBeDefined())
        expect(winLive()!.url).not.toContain('run_id=win-run')

        // A final failure: the gate shows and the connection stays closed.
        routes[WIN_AUTH] = failure
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent(message)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        const tried = connects()

        // A Retry that gets no conclusive answer keeps the failure, so the journal answering, which moves the
        // connection to follow the Run, still tries nothing.
        Object.assign(routes, inconclusive)
        const checked = checks()
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(checks()).toBe(checked + 1))
        await act(async () => journal.resolve())
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 600)))
        expect(screen.getByTestId('remote-environment-gate')).toHaveTextContent(message)
        expect(remoteShell).not.toBeVisible()
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Retry passes: one connection opens, following the Run.
        for (const url of Object.keys(inconclusive)) delete routes[url]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()?.url).toContain('run_id=win-run'))
        expect(connects()).toBe(tried + 1)
    })

    it('closes a replacement live connection, its handshake still pending, once the check of a reopened connection that dropped finds the credential refused, and opens one promptly once Retry passes', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // A connection opens again; the check it starts gets the refusal only later.
        const reopened = winLive()!
        const before = checks()
        const refusal = deferred<void>()
        routes[WIN_AUTH] = async () => {
            await refusal.promise
            return json({ detail: 'Unauthorized' }, 401)
        }
        act(() => reopened.onopen?.(new Event('open')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // It drops meanwhile, and its replacement's handshake stays pending.
        act(() => reopened.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        const replacement = winLive()!

        await act(async () => refusal.resolve())
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('refused this Spark’s credential')
        expect(replacement.closed).toBe(true)
        const tried = connects()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        // Retry passes: a new connection opens at once.
        delete routes[WIN_AUTH]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 400 })
        expect(connects()).toBe(tried + 1)
    })

    it('shows a refused credential that an earlier reconnect\'s check finds after a newer one\'s gets no answer, and stops reconnecting until Retry passes', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // A handshake accepted before the credential is revoked: the check it starts gets the refusal only later.
        const accepted = winLive()!
        const before = checks()
        const refusal = deferred<void>()
        routes[WIN_AUTH] = async () => {
            await refusal.promise
            return json({ detail: 'Unauthorized' }, 401)
        }
        act(() => accepted.onopen?.(new Event('open')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // That connection drops and the next handshake is accepted; the newer check it starts gets a 503.
        routes[WIN_AUTH] = () => json({ detail: 'connection refused' }, 503)
        act(() => accepted.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        const reopened = winLive()!
        act(() => reopened.onopen?.(new Event('open')))
        await waitFor(() => expect(reopened.closed).toBe(true))
        expect(checks()).toBe(before + 2)

        // The earlier check's refusal still takes effect: the credential gate shows, and nothing is tried or checked again.
        await act(async () => refusal.resolve())
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('refused this Spark’s credential')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        expect(remoteShell).not.toBeVisible()
        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        delete routes[WIN_AUTH]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it.each([
        ['refused credential', () => json({ detail: 'Unauthorized' }, 401), 'refused this Spark’s credential', 'refused'],
        ['version mismatch', () => json(descriptor('env-win', { ui_api_version: 3 })), 'Version mismatch with Windows Workstation', 'incompatible'],
    ])('shows a mounted remote\'s %s and stops reconnecting at once, while this machine\'s descriptor is still asked for, and keeps them when that answers 503', async (_finding, answer, message, status) => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        const localLate = deferred<void>()
        let localAsked = 0
        routes[WIN_AUTH] = answer
        routes['/environment'] = async () => {
            localAsked += 1
            await localLate.promise
            return json({ detail: 'unavailable' }, 503)
        }
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent(message)
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status, activeCheck: { status } })
        expect(remoteShell).not.toBeVisible()

        // This machine's descriptor is still pending, and nothing is tried or checked again.
        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(localAsked).toBeGreaterThan(0)
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        // Its late 503 leaves the gate as it was.
        await act(async () => localLate.resolve())
        expect(gate).toHaveTextContent(message)
        expect(getActiveEnvironmentState()).toMatchObject({ status, activeCheck: { status } })
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()

        delete routes[WIN_AUTH]
        delete routes['/environment']
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('shows a refused credential an automatic check finds while a Retry gets no conclusive answer, and stops reconnecting until Retry passes', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell, checks } = await mountWindows(routes)
        Object.assign(routes, gatewayDown)
        const connects = () => MockEventSource.instances.filter((source) => source.url.startsWith(`${WIN}/workspace/api/live/events`)).length
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to Windows Workstation')
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // The next failure's check gets this machine's 503 at once, and the peer's refusal of the credential only later.
        const refusal = deferred<void>()
        routes['/environment'] = () => json({ detail: 'unavailable' }, 503)
        routes[WIN_AUTH] = async () => {
            await refusal.promise
            return json({ detail: 'Unauthorized' }, 401)
        }
        const before = checks()
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))

        // Meanwhile a Retry gets only a gateway's answer and this machine's 503.
        routes[WIN_AUTH] = gatewayDown[WIN_AUTH]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(gate).toHaveTextContent('Could not verify this Spark server’s version'))

        // The automatic check lands after it: the credential gate shows, and nothing is tried or checked again.
        await act(async () => refusal.resolve())
        expect(gate).toHaveTextContent('refused this Spark’s credential')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'refused', activeCheck: { status: 'refused' } })
        const tried = connects()
        const checked = checks()
        await act(() => new Promise((resolve) => window.setTimeout(resolve, 1200)))
        expect(connects()).toBe(tried)
        expect(winLive()).toBeUndefined()
        expect(checks()).toBe(checked)

        for (const url of [WIN_AUTH, WIN_PUBLIC, '/environment']) delete routes[url]
        await user.click(within(gate).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('opens no live connection to a remote whose reconnect found it incompatible as you return to this machine', async () => {
        const routes: Record<string, Route> = {}
        const { user } = await mountWindows(routes)
        routes[WIN_AUTH] = () => json(descriptor('env-win', { ui_api_version: 3 }))
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        const opened = MockEventSource.instances.length

        await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
        await waitFor(() => expect(getActiveEnvironmentId()).toBeNull())
        await waitFor(() => expect(MockEventSource.instances.filter((source) => !source.closed)).toHaveLength(1))
        expect(MockEventSource.instances.slice(opened).map((source) => source.url.startsWith(`${WIN}/`))).toEqual([false])
    })

    it('checks a reconnected remote anew while its outage\'s check still runs, and keeps what the new check found', async () => {
        const routes: Record<string, Route> = {}
        const { checks } = await mountWindows(routes)
        const before = checks()
        const outage = deferred<Response>()
        routes[WIN_AUTH] = () => outage.promise
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })

        // Back, upgraded past this bundle, while the outage's check has no answer yet.
        routes[WIN_AUTH] = () => json(descriptor('env-win', { ui_api_version: 3 }))
        act(() => winLive()!.onopen?.(new Event('open')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Version mismatch with Windows Workstation')
        expect(checks()).toBe(before + 2)

        // The outage's check answering late, from before the upgrade, changes nothing.
        await act(async () => outage.resolve(json(WIN_ENV)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'incompatible' })
        expect(screen.getByTestId('remote-environment-gate')).toBeInTheDocument()
    })

    it('checks a remote whose failed live connection Try again restarted', async () => {
        const routes: Record<string, Route> = {}
        await mountWindows(routes)
        const failed = winLive()!
        act(() => failed.onerror?.(new Event('error')))
        await act(async () => {})
        act(() => requestRunsTransportReconnect())
        await waitFor(() => expect(winLive()).toBeDefined())

        routes[WIN_AUTH] = () => json(descriptor('env-win', { ui_api_version: 3 }))
        act(() => winLive()!.onopen?.(new Event('open')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Version mismatch with Windows Workstation')
    })

    it('checks a remote whose outage\'s check found a gateway that answers for no Spark as its live connection opens again, and comes back once it passes', async () => {
        const routes: Record<string, Route> = {}
        const { remoteShell } = await mountWindows(routes)
        routes[WIN_AUTH] = () => proxyRefusal(null)
        routes[WIN_PUBLIC] = () => json({ detail: 'Bad gateway' }, 502)
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toBeInTheDocument()
        expect(remoteShell).not.toBeVisible()

        // The outage outlasts the first connection that opens again: its check fails alike, and it is tried again.
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        const early = winLive()!
        act(() => early.onopen?.(new Event('open')))
        await waitFor(() => expect(early.closed).toBe(true))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        expect(screen.getByTestId('remote-environment-gate')).toBeInTheDocument()

        delete routes[WIN_AUTH]
        delete routes[WIN_PUBLIC]
        const reopened = winLive()!
        act(() => reopened.onopen?.(new Event('open')))
        await waitFor(() => expect(remoteShell).toBeVisible())
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', selectedEnvironmentId: 'env-win', status: 'ready' })
        // Two connects failed, so runs were told they may be out of date; back, the stream restarts to reload them.
        await waitFor(() => expect(winLive()).toBeDefined())
    })

    it('keeps what a new mount of a remote found from a check its previous mount started', async () => {
        const routes: Record<string, Route> = {}
        const { user, checks } = await mountWindows(routes)
        const before = checks()
        const stale = deferred<Response>()
        routes[WIN_AUTH] = () => stale.promise
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))
        delete routes[WIN_AUTH]

        await user.click(tab('local'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBeNull())
        await user.click(tab('env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        const shell = workspaceMounted()

        await act(async () => stale.resolve(json({ detail: 'Unauthorized' }, 401)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'ready', activeCheck: { status: 'ready' } })
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        expect(shell).toBeVisible()
    })

    it('closes, unanswered, a confirmation asked in the workspace while another environment was checked, as the switch commits', async () => {
        const user = userEvent.setup()
        const check = deferred<void>()
        const backend = stubBackend({ [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)) }, [WIN_LINK, GPU_LINK])
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.click(tab('env-win'))
        await user.click(await screen.findByRole('button', { name: 'Remove link GPU' }))
        const confirmation = await screen.findByRole('alertdialog', { name: 'Remove environment link?' })

        await act(async () => check.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(confirmation).not.toBeInTheDocument()
        expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
        expect(backend.requests.filter((request) => request.method === 'DELETE')).toEqual([])
    })

    it('applies nothing a reconnected remote sends until its check passes, and keeps a connection that fails it closed until Retry', async () => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell } = await mountWindows(routes)
        const applied = vi.fn()
        const refreshed = vi.fn()
        window.addEventListener('spark:harness-checked', applied)
        window.addEventListener('spark:settings-live-event', refreshed)
        const frame = new MessageEvent('message', { data: JSON.stringify({ type: 'harness.checked' }) })
        act(() => winLive()!.onerror?.(new Event('error')))
        await waitFor(() => expect(winLive()).toBeDefined(), { timeout: 2000 })
        await act(async () => {})

        // Its first frame arrives before its check answers: neither it nor the reconnect's reloads apply yet.
        const failing = winLive()!
        const check = deferred<Response>()
        routes[WIN_AUTH] = () => check.promise
        refreshed.mockClear()
        act(() => {
            failing.onopen?.(new Event('open'))
            failing.onmessage?.(frame)
        })
        expect(applied).not.toHaveBeenCalled()
        expect(refreshed).not.toHaveBeenCalled()

        // It came back re-paired as another environment: the frame never applies and the connection stays closed.
        await act(async () => check.resolve(json(descriptor('env-other'))))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('answered as environment env-other')
        expect(failing.closed).toBe(true)
        await act(() => new Promise((resolve) => setTimeout(resolve, 600)))
        expect(winLive()).toBeUndefined()
        expect(applied).not.toHaveBeenCalled()
        expect(refreshed).not.toHaveBeenCalled()

        // Retry passes: a new connection opens, and what it sends applies once its own check passes.
        delete routes[WIN_AUTH]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(winLive()).toBeDefined())
        expect(remoteShell).toBeVisible()
        const recovered = winLive()!
        act(() => {
            recovered.onopen?.(new Event('open'))
            recovered.onmessage?.(frame)
        })
        expect(applied).not.toHaveBeenCalled()
        await waitFor(() => expect(applied).toHaveBeenCalledTimes(1))
        expect(refreshed).toHaveBeenCalled()
        window.removeEventListener('spark:harness-checked', applied)
        window.removeEventListener('spark:settings-live-event', refreshed)
    })

    it('keeps a mounted remote\'s failed re-check for a return from another environment\'s failed check', async () => {
        const user = userEvent.setup()
        const gpu = deferred<void>()
        const routes: Record<string, Route> = { [GPU_AUTH]: () => gpu.promise.then(() => json({ detail: 'connection refused' }, 503)) }
        const backend = stubBackend(routes, [WIN_LINK, GPU_LINK])
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        const remoteShell = workspaceMounted()
        const checks = () => backend.requests.filter((request) => request.url === WIN_AUTH).length
        const live = () => MockEventSource.instances.filter((source) => !source.closed && source.url.startsWith(`${WIN}/workspace/api/live/events`)).at(-1)
        await waitFor(() => expect(live()).toBeDefined())

        // While the GPU's check is pending, Windows refuses its reconnect for a revoked credential.
        await user.click(tab('env-gpu'))
        routes[WIN_AUTH] = () => json({ detail: 'Unauthorized' }, 401)
        const before = checks()
        act(() => live()!.onerror?.(new Event('error')))
        await waitFor(() => expect(checks()).toBe(before + 1))
        await act(async () => {})
        expect(getActiveEnvironmentState()).toMatchObject({ selectedEnvironmentId: 'env-gpu', status: 'checking' })

        await act(async () => gpu.resolve())
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to GPU')
        await user.click(within(gate).getByRole('button', { name: 'Return to Windows Workstation' }))

        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('refused this Spark’s credential')
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', selectedEnvironmentId: 'env-win', status: 'refused' })
        expect(workspaceMounted()).toBe(remoteShell)
        expect(remoteShell).not.toBeVisible()
    })

    it.each([
        ['passes, which mounts it', true],
        ['fails, which leaves Retry to show the remote again once it passes', false],
    ])('keeps a remote that failed its re-check hidden while another environment\'s check, shown over it, runs and %s', async (_outcome, passes) => {
        const gpu = deferred<void>()
        const routes: Record<string, Route> = {
            [GPU_AUTH]: () => gpu.promise.then(() => passes ? json(descriptor('env-gpu')) : json({ detail: 'connection refused' }, 503)),
        }
        const { user, remoteShell } = await mountWindows(routes, [WIN_LINK, GPU_LINK])
        routes[WIN_AUTH] = () => json({ detail: 'Unauthorized' }, 401)
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('refused this Spark’s credential')

        await user.click(tab('env-gpu'))
        const checking = screen.getByRole('dialog', { name: 'Connecting to GPU…' })
        expect(within(checking).getByTestId('environment-checking')).toHaveTextContent('Connecting to GPU…')
        expect(workspaceMounted()).toBe(remoteShell)
        expect(remoteShell).not.toBeVisible()

        await act(async () => gpu.resolve())
        if (passes) {
            await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-gpu'))
            expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
            expect(workspaceMounted()).not.toBe(remoteShell)
            expect(workspaceMounted()).toBeVisible()
            return
        }
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Couldn’t connect to GPU')
        expect(remoteShell).not.toBeVisible()
        await user.click(screen.getByRole('button', { name: 'Return to Windows Workstation' }))
        expect(screen.getByTestId('remote-environment-gate')).toHaveTextContent('refused this Spark’s credential')
        expect(remoteShell).not.toBeVisible()

        delete routes[WIN_AUTH]
        await user.click(screen.getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        expect(workspaceMounted()).toBe(remoteShell)
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', selectedEnvironmentId: 'env-win', status: 'ready' })
    })

    it.each([
        ['the Run Flow dialog', async () => {
            act(() => { window.dispatchEvent(new CustomEvent('spark:run-flow', { detail: {} })) })
            await screen.findByTestId('run-flow-dialog')
        }, () => screen.getByTestId('run-flow-dialog')],
        ['a confirmation', async (user: ReturnType<typeof userEvent.setup>) => {
            await user.click(await screen.findByRole('button', { name: 'Remove link GPU' }))
            await screen.findByRole('alertdialog', { name: 'Remove environment link?' })
        }, () => screen.getByRole('alertdialog', { name: 'Remove environment link?' })],
    ])('keeps the gate usable over %s left open in the workspace as a re-check failed, and its confirmation over the gate', async (_dialog, openDialog, leftOpen) => {
        const routes: Record<string, Route> = { [`${WIN}${LINKS}`]: () => json({ links: [GPU_LINK] }) }
        const { user, remoteShell } = await mountWindows(routes)
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')
        await openDialog(user)

        routes[WIN_AUTH] = () => json({ detail: 'Unauthorized' }, 401)
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(screen.getByRole('dialog', { name: 'Couldn’t connect to Windows Workstation' })).toContainElement(gate)
        expect(gate).toBeVisible()

        // Leaving asks about the unsaved settings over the gate; keeping them keeps the gate.
        await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
        const confirmation = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
        await user.click(within(confirmation).getByRole('button', { name: 'Keep editing' }))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', selectedEnvironmentId: 'env-win', status: 'refused' })

        // Retry shows the same workspace with its draft, and the dialog it left open still there.
        delete routes[WIN_AUTH]
        await user.click(within(screen.getByTestId('remote-environment-gate')).getByRole('button', { name: 'Retry' }))
        await waitFor(() => expect(remoteShell).toBeVisible())
        expect(workspaceMounted()).toBe(remoteShell)
        expect(screen.getByLabelText('Command-line server URL')).toHaveValue('http://draft.example')
        expect(leftOpen()).toBeVisible()
    })

    it('holds a launch\'s confirmation asked as the mounted remote failed its re-check behind the gate, which leaving still asks over, and asks it once the remote passes', async () => {
        const metadata = deferred<Response>()
        const metadataUrl = `${WIN}/workspace/api/projects/metadata?directory=${encodeURIComponent('/win/project')}`
        const routes: Record<string, Route> = {
            [metadataUrl]: () => metadata.promise,
            [`POST ${WIN}/attractor/pipelines`]: () => json({ status: 'started', pipeline_id: 'win-run' }, 202),
        }
        const { user, backend } = await mountWindows(routes)
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')
        const { result } = renderHook(() => useStartPipeline(), { wrapper: DialogProvider })
        const launch = result.current.startFlow({ projectPath: '/win/project', workingDirectory: '/win/project', flowSource: 'flow', model: null })
        await waitFor(() => expect(backend.sentTo('env-win')).toContain(metadataUrl))

        // Back upgraded past this bundle while the launch waits on its Git state, which then can't be read.
        routes[WIN_AUTH] = () => json(descriptor('env-win', { ui_api_version: 3 }))
        act(() => winLive()!.onerror?.(new Event('error')))
        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        await act(async () => metadata.resolve(json({ detail: 'Bad Gateway' }, 502)))
        expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()

        // Leaving still asks over the gate.
        await user.click(within(gate).getByRole('button', { name: 'Return to this machine' }))
        const leave = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
        await user.click(within(leave).getByRole('button', { name: 'Keep editing' }))
        expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
        const pipelineStarts = () => backend.requests.filter((request) => request.method === 'POST' && request.url.endsWith('/attractor/pipelines'))
        expect(pipelineStarts()).toEqual([])

        delete routes[WIN_AUTH]
        await user.click(within(screen.getByTestId('remote-environment-gate')).getByRole('button', { name: 'Retry' }))
        const confirmation = await screen.findByRole('alertdialog', { name: 'Unable to verify Git state' })
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        await user.click(within(confirmation).getByRole('button', { name: 'Continue' }))
        await expect(launch).resolves.toMatchObject({ status: 'launched', runId: 'win-run' })
        expect(pipelineStarts().map((request) => request.url)).toEqual([`${WIN}/attractor/pipelines`])
    })

    it('names a peer Settings switched to, and its direct webhook URL, though the switcher could not list it', async () => {
        const user = userEvent.setup()
        const webhook = {
            id: 'trigger-webhook', project_path: '/remote/project', name: 'Webhook trigger', enabled: true, protected: false,
            revision: 'revision-1', source_type: 'webhook', created_at: '2026-11-24T00:00:00Z', updated_at: '2026-11-24T00:00:00Z',
            action: { flow_name: 'demo', static_context: {} }, source: { webhook_key: 'webhook-key-1' },
            state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: null, recent_history: [] },
            webhook_secret: null,
        }
        let listings = 0
        stubBackend({
            // The switcher's own listing, at startup, fails; Settings lists the link once opened.
            [LINKS]: () => (++listings === 1 ? json({ detail: 'unavailable' }, 503) : json({ links: [WIN_LINK] })),
            [`${WIN}/workspace/api/triggers`]: () => json([webhook]),
        })
        render(<App />)
        await openSystemSettings()
        await user.click(await screen.findByRole('button', { name: 'Switch to Windows Workstation' }))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(listings).toBeGreaterThan(1)
        expect(tab('env-win')).toHaveTextContent('Windows Workstation')

        act(() => useStore.getState().setViewMode('triggers'))
        await user.click(await screen.findByTestId('trigger-row-trigger-webhook'))
        for (const guidance of screen.getAllByTestId('trigger-webhook-destination')) {
            expect(guidance).toHaveTextContent('This trigger runs on Windows Workstation. Another service starts it by sending a POST request with a JSON body directly to http://env-win:8000/workspace/api/webhooks, not through this Spark')
        }
    })

    describe.each([
        // The tab bar opens the environment on its own last view; Settings' Switch opens it on its Overview.
        ['the tab bar', (user: ReturnType<typeof userEvent.setup>) => user.click(tab('env-win')), 'settings'],
        ['Settings', (user: ReturnType<typeof userEvent.setup>) => user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' })), 'overview'],
    ])('switching from %s with unsaved settings', (_source, requestSwitch, landsOn) => {
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

        it('drops a switch awaiting approval once Settings in another tab removed its destination, keeping this workspace and its draft', async () => {
            let links = [WIN_LINK]
            const { user, shell } = await renderWithDraft({ [LINKS]: () => json({ links }) })
            await requestSwitch(user)
            await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })

            // The link is removed elsewhere; this tab's switcher sees it once focused again.
            links = []
            act(() => { window.dispatchEvent(new Event('focus')) })
            await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
            expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' })
            expect(screen.queryByTestId('environment-tab-env-win')).not.toBeInTheDocument()
            expect(workspaceMounted()).toBe(shell)
            expect(shell).toBeVisible()
            expect(draftField()).toHaveValue('http://draft.example')
        })

        it('drops a switch awaiting approval once another tab linked its destination anew, never mounting the new link under the old one\'s check', async () => {
            let links = [WIN_LINK]
            const { user, backend, shell } = await renderWithDraft({ [LINKS]: () => json({ links }) })
            await requestSwitch(user)
            await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })

            links = [WIN_RELINKED]
            act(() => { window.dispatchEvent(new Event('focus')) })
            await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
            expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready', links: [WIN_RELINKED] })
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
            // Windows was last left in Settings.
            saveSessionViewMode('settings', 'env-win')
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
            await waitFor(() => expect(useStore.getState().viewMode).toBe(landsOn))
            expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
        })
    })

    it('commits a switch approved after the mounted remote\'s live connection came back compatible during its dialog', async () => {
        const user = userEvent.setup()
        const backend = stubBackend()
        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        const live = () => MockEventSource.instances.filter((source) => !source.closed && source.url.startsWith(`${WIN}/workspace/api/live/events`)).at(-1)
        await waitFor(() => expect(live()).toBeDefined())
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')

        await user.click(tab('local'))
        const dialog = await screen.findByRole('alertdialog')
        const checks = () => backend.requests.filter((request) => request.url === WIN_AUTH).length
        const before = checks()
        const dropped = live()!
        act(() => dropped.onerror?.(new Event('error')))
        await waitFor(() => expect(live()).toBeDefined(), { timeout: 2000 })
        act(() => live()!.onopen?.(new Event('open')))
        await waitFor(() => expect(checks()).toBe(before + 2))
        await act(async () => {})
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', status: 'ready' })

        await user.click(within(dialog).getByRole('button', { name: 'Discard and leave' }))
        await waitFor(() => expect(getActiveEnvironmentId()).toBeNull())
        expect(tab('local')).toHaveAttribute('aria-selected', 'true')
    })

    it.each([
        ['approved, which leaves once', 'Discard and leave'],
        ['cancelled, which keeps the draft behind the gate until Return asks again', 'Keep editing'],
    ])('keeps a switch awaiting approval answerable over the gate when the mounted remote fails its re-check, and it is %s', async (_outcome, answer) => {
        const routes: Record<string, Route> = {}
        const { user, remoteShell } = await mountWindows(routes)
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')
        vi.spyOn(useRunJournalStore, 'setState')
        const commits = () => vi.mocked(useRunJournalStore.setState).mock.calls.filter(([update]) => JSON.stringify(update) === '{"byRunId":{}}').length
        await user.click(tab('local'))
        await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })

        // Windows revokes this Spark's credential and restarts while the switch awaits approval.
        routes[WIN_AUTH] = () => json({ detail: 'Unauthorized' }, 401)
        act(() => winLive()!.onerror?.(new Event('error')))
        expect(await screen.findByTestId('remote-environment-gate')).toHaveTextContent('Switching to this machine…')
        expect(remoteShell).not.toBeVisible()
        const confirmation = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
        await user.click(within(confirmation).getByRole('button', { name: answer }))

        if (answer === 'Keep editing') {
            expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', selectedEnvironmentId: 'env-win', status: 'refused' })
            expect(screen.getByTestId('remote-environment-gate')).toHaveTextContent('refused this Spark’s credential')
            expect(remoteShell).not.toBeVisible()
            expect(screen.getByLabelText('Command-line server URL')).toHaveValue('http://draft.example')
            expect(commits()).toBe(0)
            await user.click(screen.getByRole('button', { name: 'Return to this machine' }))
            const again = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
            await user.click(within(again).getByRole('button', { name: 'Discard and leave' }))
        }
        await waitFor(() => expect(getActiveEnvironmentId()).toBeNull())
        expect(commits()).toBe(1)
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        expect(workspaceMounted()).not.toBe(remoteShell)
        expect(workspaceMounted()).toBeVisible()
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

    it.each([
        ['passes', () => json(WIN_ENV)],
        // As the proxy answers once the link is gone.
        ['fails', () => json({ detail: 'Not Found' }, 404)],
    ])('drops a switch from the tab bar whose destination Settings removed during its check, which then %s', async (_outcome, answer) => {
        const user = userEvent.setup()
        const check = deferred<void>()
        let links = [WIN_LINK, GPU_LINK]
        stubBackend({
            [LINKS]: () => json({ links }),
            [WIN_AUTH]: () => check.promise.then(answer),
            [`DELETE ${LINKS}/env-win`]: () => {
                links = links.filter((row) => row.environment_id !== 'env-win')
                return json({ environment_id: 'env-win', removed: true })
            },
        }, links)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        const shell = workspaceMounted()

        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')
        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(tabs()).toEqual(['mac-studio(local)', 'GPU']))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' })
        expect(screen.queryByTestId('environment-checking')).not.toBeInTheDocument()

        await act(async () => check.resolve())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' })
        expect(screen.queryByTestId('remote-environment-gate')).not.toBeInTheDocument()
        expect(workspaceMounted()).toBe(shell)
        expect(shell).toBeVisible()
    })

    it('drops a switch from the tab bar whose destination another tab linked anew during its check, though the check then passes', async () => {
        const user = userEvent.setup()
        const check = deferred<void>()
        let links = [WIN_LINK]
        stubBackend({ [LINKS]: () => json({ links }), [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)) })
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        const shell = workspaceMounted()
        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')

        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(screen.queryByTestId('environment-checking')).not.toBeInTheDocument())
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready', links: [WIN_RELINKED] })

        await act(async () => check.resolve())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' })
        expect(workspaceMounted()).toBe(shell)
        expect(shell).toBeVisible()
    })

    it('checks once, and asks once, for a switch whose destination listings show unchanged during its check and its confirmation', async () => {
        const user = userEvent.setup()
        const check = deferred<void>()
        const backend = stubBackend({ [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)) })
        const listings = () => backend.requests.filter((request) => request.url === LINKS).length
        const listAgain = async () => {
            const before = listings()
            act(() => { window.dispatchEvent(new Event('focus')) })
            await waitFor(() => expect(listings()).toBe(before + 1))
            await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        }
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Command-line server URL'), 'http://draft.example')
        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')

        await listAgain()
        await act(async () => check.resolve())
        const dialog = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
        await listAgain()
        expect(dialog).toBeInTheDocument()
        expect(backend.requests.filter((request) => request.url === WIN_AUTH)).toHaveLength(1)

        await user.click(within(dialog).getByRole('button', { name: 'Discard and leave' }))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
        expect(backend.requests.filter((request) => request.url === WIN_AUTH)).toHaveLength(1)
    })

    it('keeps a link listed anew, and a switch to it, when Settings\' removal of the link it replaced answers late', async () => {
        const user = userEvent.setup()
        const removal = deferred<void>()
        const check = deferred<void>()
        let links = [WIN_LINK]
        const backend = stubBackend({
            [LINKS]: () => json({ links }),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
            [`DELETE ${LINKS}/env-win`]: () => removal.promise.then(() => json({ environment_id: 'env-win', removed: true })),
        })
        const changed = vi.fn()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(backend.requests.some((request) => request.method === 'DELETE')).toBe(true))

        // Linked anew elsewhere once that removal went: the switcher lists the new link, and a switch to it starts.
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)'))
        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')

        // The removal answers late: the new link and the switch to it stay.
        await act(async () => removal.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking', links: [WIN_RELINKED] })
        await act(async () => check.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED])
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it('lets a switch from Settings to a link listed anew finish when Settings\' removal of the link it replaced answers late, though the switcher still lists the old link', async () => {
        const user = userEvent.setup()
        const removal = deferred<void>()
        const check = deferred<void>()
        let links = [WIN_LINK]
        const backend = stubBackend({
            [LINKS]: () => json({ links }),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
            [`DELETE ${LINKS}/env-win`]: () => removal.promise.then(() => json({ environment_id: 'env-win', removed: true })),
        })
        const changed = vi.fn()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(backend.requests.some((request) => request.method === 'DELETE')).toBe(true))

        // Linked anew elsewhere: Settings, opened again, lists the new link before the switcher does, and switches to it.
        links = [WIN_RELINKED]
        act(() => useStore.getState().setViewMode('runs'))
        await openSystemSettings()
        expect(await screen.findByText('http://env-win-new:8000 · env-win')).toBeVisible()
        expect(getActiveEnvironmentState().links).toEqual([WIN_LINK])
        await user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))
        await screen.findByTestId('environment-checking')

        // The removal answers late: the new link and the switch to it stay.
        await act(async () => removal.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking', links: [WIN_RELINKED] })
        await act(async () => check.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED])
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it('drops a switch from Settings to the row it still shows once the switcher lists the link made anew, keeping that listing and never mounting the new link under the old origin', async () => {
        const user = userEvent.setup()
        const check = deferred<void>()
        let links = [WIN_LINK]
        stubBackend({ [LINKS]: () => json({ links }), [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)) })
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        expect(await screen.findByText('http://env-win:8000 · env-win')).toBeVisible()
        const shell = workspaceMounted()

        // Linked anew in another tab: the switcher lists it once focused, while Settings still shows the old row.
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)'))
        expect(screen.getByText('http://env-win:8000 · env-win')).toBeVisible()

        await user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))
        expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED])
        await waitFor(() => expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready' }))
        await act(async () => check.resolve())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready', links: [WIN_RELINKED] })
        expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)')
        expect(workspaceMounted()).toBe(shell)
        expect(shell).toBeVisible()
    })

    it('drops a switch from Settings to the row it still shows once the latest listing, answering after an outdated one and the check, shows the link made anew', async () => {
        const user = userEvent.setup()
        let links = [WIN_LINK]
        let hold = false
        const held: Array<() => void> = []
        stubBackend({ [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) }) : json({ links })) })
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        expect(await screen.findByText('http://env-win:8000 · env-win')).toBeVisible()
        const shell = workspaceMounted()
        links = [WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)'))

        // Another reload outdates the one the switch started before either answers; the check passes meanwhile, and
        // the outdated reload answers first: the switch waits for the latest.
        hold = true
        await user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(held).toHaveLength(2))
        await act(async () => held[0]())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking' })
        hold = false
        await act(async () => held[1]())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready', links: [WIN_RELINKED] })
        expect(workspaceMounted()).toBe(shell)
        expect(shell).toBeVisible()
    })

    it('keeps a switch to a link made anew, and the switcher\'s row for it, when Settings\' adding of the row it replaced answers late', async () => {
        const user = userEvent.setup()
        const added = deferred<void>()
        const check = deferred<void>()
        let links = [GPU_LINK]
        const backend = stubBackend({
            [LINKS]: () => json({ links }),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
            [`POST ${LINKS}`]: () => added.promise.then(() => json(WIN_LINK)),
        }, links)
        const changed = vi.fn()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-gpu')
        await openSystemSettings()
        await user.type(await screen.findByLabelText('Remote origin'), 'http://env-win:8000')
        await user.type(screen.getByLabelText('Remote environment ID'), 'env-win')
        await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
        await user.type(screen.getByLabelText('Link label (optional)'), 'Windows Workstation')
        await user.click(screen.getByRole('button', { name: 'Add link' }))
        await waitFor(() => expect(backend.requests.some((request) => request.method === 'POST')).toBe(true))

        // Paired again from another tab before that answer arrives: the switcher lists the later link, and a switch to it starts.
        links = [GPU_LINK, WIN_RELINKED]
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)'))
        await user.click(tab('env-win'))
        await screen.findByTestId('environment-checking')

        // The earlier pairing answers late: the switch, and the row it awaits, stay.
        await act(async () => added.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking' })
        expect(tab('env-win')).toHaveAttribute('title', 'Windows Workstation (http://env-win-new:8000)')
        await act(async () => check.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(getActiveEnvironmentState().links).toEqual([GPU_LINK, WIN_RELINKED])
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it('lets a switch from Settings to a link made anew in the same second with the same origin, under another label, finish when the removal of the row it replaced answers late, though the switcher still lists that row', async () => {
        const user = userEvent.setup()
        const removal = deferred<void>()
        const check = deferred<void>()
        const relabeled = { ...WIN_LINK, label: 'Windows Laptop' }
        let links = [WIN_LINK]
        let hold = false
        const held: Array<() => void> = []
        const backend = stubBackend({
            [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) }) : json({ links })),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
            [`DELETE ${LINKS}/env-win`]: () => removal.promise.then(() => json({ environment_id: 'env-win', removed: true })),
        })
        const changed = vi.fn()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(backend.requests.some((request) => request.method === 'DELETE')).toBe(true))

        // Linked anew at once: Settings, opened again, lists the new row and switches to it, while the switcher, its
        // reloads unanswered, still lists the row removed.
        links = [relabeled]
        act(() => useStore.getState().setViewMode('runs'))
        await openSystemSettings()
        const switchTo = await screen.findByRole('button', { name: 'Switch to Windows Laptop' })
        expect(getActiveEnvironmentState().links).toEqual([WIN_LINK])
        hold = true
        await user.click(switchTo)
        await screen.findByTestId('environment-checking')
        await act(async () => removal.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking' })

        hold = false
        await act(async () => held.splice(0).forEach((resolve) => resolve()))
        await waitFor(() => expect(getActiveEnvironmentState().links).toEqual([relabeled]))
        expect(getActiveEnvironmentState()).toMatchObject({ selectedEnvironmentId: 'env-win', status: 'checking' })
        await act(async () => check.resolve())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(tab('env-win')).toHaveTextContent('Windows Laptop')
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it('lets a switch from Settings to a link listed anew finish once the latest listing shows it, though the removal of the row it replaced answers late and the reload that removal outdated answers first', async () => {
        const user = userEvent.setup()
        const removal = deferred<void>()
        const check = deferred<void>()
        let links = [WIN_LINK]
        let hold = false
        const held: Array<() => void> = []
        const backend = stubBackend({
            [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) }) : json({ links })),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
            [`DELETE ${LINKS}/env-win`]: () => removal.promise.then(() => json({ environment_id: 'env-win', removed: true })),
        })
        const changed = vi.fn()
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        await user.click(await screen.findByRole('button', { name: 'Remove link Windows Workstation' }))
        await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Remove link' }))
        await waitFor(() => expect(backend.requests.some((request) => request.method === 'DELETE')).toBe(true))

        // Linked anew elsewhere: Settings, opened again, lists the new link before the switcher does, and switches to it.
        links = [WIN_RELINKED]
        act(() => useStore.getState().setViewMode('runs'))
        await openSystemSettings()
        expect(await screen.findByText('http://env-win-new:8000 · env-win')).toBeVisible()
        hold = true
        await user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))
        await screen.findByTestId('environment-checking')
        await waitFor(() => expect(held).toHaveLength(1))

        // The removal answers, starting another reload; the one the switch started answers first, then the check passes.
        await act(async () => removal.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        await act(async () => held[0]())
        await act(async () => check.resolve())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: 'env-win', status: 'checking' })

        // The latest listing shows the new link: the switch finishes.
        hold = false
        await act(async () => held.splice(1).forEach((resolve) => resolve()))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED])
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it.each(['approved while the listing reloads', 'left open'])('drops a switch whose confirmation is %s once Settings\' adding of a link in its destination\'s place answers and the listing shows that link, keeping the draft', async (answer) => {
        const user = userEvent.setup()
        const added = deferred<void>()
        let links = [WIN_LINK]
        let hold = false
        const held: Array<() => void> = []
        const changed = vi.fn()
        stubBackend({
            [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) }) : json({ links })),
            [`POST ${LINKS}`]: () => added.promise.then(() => json(WIN_RELINKED)),
        })
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        const shell = workspaceMounted()

        // Windows is paired again in Settings; that answer is still on its way.
        await user.type(await screen.findByLabelText('Remote origin'), 'http://env-win-new:8000')
        await user.type(screen.getByLabelText('Remote environment ID'), 'env-win')
        await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
        await user.type(screen.getByLabelText('Link label (optional)'), 'Windows Workstation')
        await user.click(screen.getByRole('button', { name: 'Add link' }))

        // A switch to Windows, its check passed, asks about unsaved settings.
        await user.type(screen.getByLabelText('Command-line server URL'), 'http://draft.example')
        await user.click(tab('env-win'))
        const dialog = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })

        // The pairing answers, and the listing that shows it is still on its way.
        links = [WIN_RELINKED]
        hold = true
        await act(async () => added.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        await waitFor(() => expect(held.length).toBeGreaterThan(0))
        if (answer !== 'left open') {
            await user.click(within(dialog).getByRole('button', { name: 'Discard and leave' }))
            await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
            expect(getActiveEnvironmentId()).toBeNull()
            expect(workspaceMounted()).toBe(shell)
        }

        hold = false
        await act(async () => held.splice(0).forEach((resolve) => resolve()))
        await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
        await waitFor(() => expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: null, selectedEnvironmentId: null, status: 'ready', links: [WIN_RELINKED] }))
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentId()).toBeNull()
        expect(workspaceMounted()).toBe(shell)
        expect(shell).toBeVisible()
        expect(screen.getByLabelText('Command-line server URL')).toHaveValue('http://draft.example')
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
    })

    it('finishes a switch from Settings to a link listed anew once a newer listing shows it and its check passes, though the listing the switch started never answers', async () => {
        const user = userEvent.setup()
        const check = deferred<void>()
        let links = [WIN_LINK]
        let hold = false
        const held: Array<() => void> = []
        stubBackend({
            [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links }))) }) : json({ links })),
            [WIN_AUTH]: () => check.promise.then(() => json(WIN_ENV)),
        })
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()

        // Linked anew elsewhere: Settings, opened again, lists the new link before the switcher does, and switches to it.
        links = [WIN_RELINKED]
        act(() => useStore.getState().setViewMode('runs'))
        await openSystemSettings()
        expect(await screen.findByText('http://env-win-new:8000 · env-win')).toBeVisible()
        hold = true
        await user.click(screen.getByRole('button', { name: 'Switch to Windows Workstation' }))
        await screen.findByTestId('environment-checking')
        await waitFor(() => expect(held).toHaveLength(1))

        // The check passes while that listing is still out; a newer one, as on focus, shows the new link.
        await act(async () => check.resolve())
        act(() => { window.dispatchEvent(new Event('focus')) })
        await waitFor(() => expect(held).toHaveLength(2))
        await act(async () => held[1]())
        await waitFor(() => expect(getActiveEnvironmentId()).toBe('env-win'))
        expect(getActiveEnvironmentState().links).toEqual([WIN_RELINKED])

        hold = false
        await act(async () => held[0]())
        await act(() => new Promise((resolve) => setTimeout(resolve, 50)))
        expect(getActiveEnvironmentState()).toMatchObject({ activeEnvironmentId: 'env-win', links: [WIN_RELINKED] })
    })

    it.each([
        ['Keep editing', null],
        ['Discard and leave', 'env-win'],
    ])('asks again before mounting a switch approved while Settings\' late adding of an older row left its link in doubt, once the listing confirms the link: %s', async (answer, mounted) => {
        const user = userEvent.setup()
        const added = deferred<void>()
        let hold = false
        const held: Array<() => void> = []
        const changed = vi.fn()
        stubBackend({
            [LINKS]: () => (hold ? new Promise<Response>((resolve) => { held.push(() => resolve(json({ links: [WIN_RELINKED] }))) }) : json({ links: [WIN_RELINKED] })),
            [`POST ${LINKS}`]: () => added.promise.then(() => json(WIN_LINK)),
        }, [WIN_RELINKED])
        window.addEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
        render(<App />)
        await screen.findByTestId('environment-tab-env-win')
        await openSystemSettings()
        const shell = workspaceMounted()

        // An earlier pairing of Windows, since replaced, is still on its way.
        await user.type(await screen.findByLabelText('Remote origin'), 'http://env-win:8000')
        await user.type(screen.getByLabelText('Remote environment ID'), 'env-win')
        await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
        await user.type(screen.getByLabelText('Link label (optional)'), 'Windows Workstation')
        await user.click(screen.getByRole('button', { name: 'Add link' }))

        await user.type(screen.getByLabelText('Command-line server URL'), 'http://draft.example')
        await user.click(tab('env-win'))
        const dialog = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })

        // It answers: the listing that tells which link is current is still on its way as the leave is approved, and
        // the workspace, still shown, is edited again.
        hold = true
        await act(async () => added.resolve())
        await waitFor(() => expect(changed).toHaveBeenCalled())
        await waitFor(() => expect(held.length).toBeGreaterThan(0))
        await user.click(within(dialog).getByRole('button', { name: 'Discard and leave' }))
        await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
        await user.type(screen.getByLabelText('Command-line server URL'), '/more')
        expect(getActiveEnvironmentId()).toBeNull()

        // The listing confirms the link: those edits are asked about before anything mounts, once it is checked again.
        hold = false
        await act(async () => {
            held.splice(0).forEach((resolve) => resolve())
            await new Promise((resolve) => setTimeout(resolve, 50))
        })
        const again = await screen.findByRole('alertdialog', { name: 'Discard unsaved settings?' })
        expect(getActiveEnvironmentId()).toBeNull()
        expect(workspaceMounted()).toBe(shell)
        expect(screen.getByLabelText('Command-line server URL')).toHaveValue('http://draft.example/more')

        await user.click(within(again).getByRole('button', { name: answer }))
        await waitFor(() => expect(getActiveEnvironmentId()).toBe(mounted))
        if (mounted === null) {
            expect(getActiveEnvironmentState()).toMatchObject({ selectedEnvironmentId: null, status: 'ready' })
            expect(workspaceMounted()).toBe(shell)
            expect(screen.getByLabelText('Command-line server URL')).toHaveValue('http://draft.example/more')
        }
        window.removeEventListener(ENVIRONMENT_LINKS_CHANGED_EVENT, changed)
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
