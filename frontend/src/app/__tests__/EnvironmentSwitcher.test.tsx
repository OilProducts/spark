import { act, render, renderHook, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '@/App'
import { DialogProvider } from '@/components/app/dialog-controller'
import { useStartPipeline } from '@/features/launch'
import { resetActiveEnvironmentForTests } from '@/lib/activeEnvironment'
import { workspaceLiveEventsUrl, workspaceUrl } from '@/lib/api/apiClient'
import { attractorUrl } from '@/lib/api/attractorApi'
import { DEFAULT_ROUTE_STATE, ROUTE_STATE_STORAGE_KEY, saveRouteState } from '@/state/store-helpers'
import { useStore } from '@/store'
import { deferred } from '@/test/environments'

class MockEventSource {
    static readonly instances: MockEventSource[] = []
    readonly url: string
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

const LOCAL_ENV = {
    environment_id: 'env-local',
    display_name: 'mac-studio',
    protocol_version: 1,
    ui_api_version: 1,
    spark_version: '0.2.0',
}

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

const REMOTE_LINKS = {
    links: [
        {
            environment_id: 'env-win',
            origin: 'http://win-box:8000',
            label: 'Windows Workstation',
            created_at: '2026-11-24T00:00:00Z',
        },
    ],
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
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = resolveUrl(input)
            if (url === '/environment') return json(LOCAL_ENV)
            if (url === '/workspace/api/environments/links') return json({ links: [] })
            if (url.includes('/workspace/api/definitions?')) return json({ items: [] })
            if (url.includes('/attractor/runs')) return json({ runs: [] })
            if (url.includes('/workspace/api/projects')) return json([LOCAL_PROJECT])
            return json({})
        }))

        render(<App />)
        await waitFor(() => {
            expect(screen.getByTestId('app-shell')).toBeVisible()
        })
        expect(screen.queryByTestId('environment-switcher')).not.toBeInTheDocument()
    })

    it('switches to a compatible remote environment, prefixes API/SSE URLs, preserves local route state, hides Desktop Server, and switches back', async () => {
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

        const requestedUrls: string[] = []
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = resolveUrl(input)
            requestedUrls.push(url)
            if (url === '/environment') return json(LOCAL_ENV)
            if (url === '/workspace/api/environments/links') return json(REMOTE_LINKS)
            if (url === '/workspace/api/environments/links/env-win/proxy/environment') {
                return json({
                    environment_id: 'env-win',
                    display_name: 'win-box',
                    protocol_version: 1,
                    ui_api_version: 1,
                    spark_version: '0.2.0',
                })
            }
            if (url.includes('/workspace/api/definitions?')) return json({ items: [] })
            if (url.endsWith('/attractor/runs')) return json({ runs: [] })
            if (url === '/workspace/api/projects') return json([LOCAL_PROJECT])
            if (url.endsWith('/workspace/api/projects')) return json([])
            if (url.endsWith('/workspace/api/triggers')) return json([])
            if (url.endsWith('/workspace/api/trigger-issues')) return json([])
            if (url.endsWith('/workspace/api/attention')) return json({ items: [] })
            return json({})
        }))

        render(<App />)

        const remoteTab = await screen.findByTestId('environment-tab-env-win')
        expect(screen.getByTestId('environment-tab-local')).toHaveAttribute('aria-selected', 'true')
        expect(workspaceUrl('/projects')).toBe('/workspace/api/projects')
        expect(attractorUrl('/runs')).toBe('/attractor/runs')

        await user.click(remoteTab)

        await waitFor(() => {
            expect(remoteTab).toHaveAttribute('aria-selected', 'true')
            expect(screen.getByTestId('remote-environment-indicator')).toHaveTextContent('Remote · http://win-box:8000')
        })

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
            expect(
                MockEventSource.instances.some((es) =>
                    es.url.startsWith('/workspace/api/environments/links/env-win/proxy/workspace/api/live/events'),
                ),
            ).toBe(true)
            expect(
                requestedUrls.some((url) =>
                    url.startsWith('/workspace/api/environments/links/env-win/proxy/attractor/runs'),
                ),
            ).toBe(true)
        })

        // Local-only Tauri Desktop Server card is hidden while viewing a remote environment.
        act(() => {
            useStore.getState().setSettingsCategory('system')
            useStore.getState().setViewMode('settings')
        })
        expect(await screen.findByTestId('settings-panel')).toBeVisible()
        expect(screen.queryByRole('heading', { name: 'Desktop server' })).not.toBeInTheDocument()
        expect(invokedTauriCommands).not.toContain('desktop_server_settings')

        // Switching back to local restores unprefixed APIs and local route state.
        await user.click(screen.getByTestId('environment-tab-local'))
        await waitFor(() => {
            expect(screen.getByTestId('environment-tab-local')).toHaveAttribute('aria-selected', 'true')
        })
        expect(workspaceUrl('/projects')).toBe('/workspace/api/projects')
        expect(attractorUrl('/runs')).toBe('/attractor/runs')
        expect(useStore.getState().activeProjectPath).toBe('/local/project')
    })

    it('blocks switching when remote ui_api_version mismatches local and never sends workspace requests to it', async () => {
        const user = userEvent.setup()
        const requestedUrls: string[] = []
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = resolveUrl(input)
            requestedUrls.push(url)
            if (url === '/environment') return json(LOCAL_ENV)
            if (url === '/workspace/api/environments/links') return json(REMOTE_LINKS)
            if (url === '/workspace/api/environments/links/env-win/proxy/environment') {
                return json({
                    environment_id: 'env-win',
                    display_name: 'win-box',
                    protocol_version: 1,
                    ui_api_version: 2,
                    spark_version: '0.3.0',
                })
            }
            if (url.includes('/workspace/api/definitions?')) return json({ items: [] })
            if (url.includes('/attractor/runs')) return json({ runs: [] })
            if (url.includes('/workspace/api/projects')) return json([LOCAL_PROJECT])
            return json({})
        }))

        render(<App />)
        const remoteTab = await screen.findByTestId('environment-tab-env-win')
        await user.click(remoteTab)

        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Version mismatch with Windows Workstation')
        expect(gate).toHaveTextContent('UI API v2')
        expect(gate).toHaveTextContent('UI API v1')

        // Workspace was never mounted against the incompatible remote.
        expect(screen.queryByTestId('app-shell')).not.toBeInTheDocument()
        expect(
            requestedUrls.filter((url) => url.startsWith('/workspace/api/environments/links/env-win/proxy/')),
        ).toEqual(['/workspace/api/environments/links/env-win/proxy/environment'])

        // Returning to local restores the workspace immediately.
        await user.click(screen.getByRole('button', { name: 'Return to this machine' }))
        expect(await screen.findByTestId('app-shell')).toBeVisible()
        expect(workspaceUrl('/projects')).toBe('/workspace/api/projects')
    })

    it.each(['launch', 'continuation'])('leaves the view, Runs selection and URL to a %s answered once the workspace closed for a connection check, then or after it fails', async (kind) => {
        const user = userEvent.setup()
        const checked = deferred<void>()
        const acks: Array<ReturnType<typeof deferred<Response>>> = []
        const requested: string[] = []
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
            const url = resolveUrl(input)
            if (url === '/environment') return json(LOCAL_ENV)
            if (url === '/workspace/api/environments/links') return json(REMOTE_LINKS)
            if (url === '/workspace/api/environments/links/env-win/proxy/environment') {
                await checked.promise
                return json({ detail: 'connection refused' }, 503)
            }
            if (init?.method === 'POST' && /\/attractor\/pipelines(\/[^/]+\/continue)?$/.test(url)) {
                requested.push(url)
                acks.push(deferred<Response>())
                return acks.at(-1)!.promise
            }
            if (url.includes('/workspace/api/projects/metadata')) return json({ branch: 'main' })
            if (url.includes('/workspace/api/definitions?')) return json({ items: [] })
            if (url.includes('/attractor/runs')) return json({ runs: [] })
            if (url === '/workspace/api/projects') return json([LOCAL_PROJECT])
            return json({})
        }))
        render(<App />)
        act(() => {
            useStore.getState().setViewMode('home')
            useStore.getState().setRunsSelectedRunId('newer')
        })
        await waitFor(() => expect(window.location.hash).toBe('#/chats'))
        const { result } = renderHook(() => useStartPipeline(), { wrapper: DialogProvider })
        const form = { projectPath: '/local/project', workingDirectory: '/local/project', flowSource: 'flow', model: null }
        const start = () => kind === 'launch'
            ? result.current.startFlow(form)
            : result.current.continueFromRun('source', form, { startNodeId: 'review', flowSourceMode: 'snapshot' })
        const savedRoute = () => window.localStorage.getItem(ROUTE_STATE_STORAGE_KEY)
        const saved = savedRoute()
        const unchanged = () => {
            expect(useStore.getState().viewMode).toBe('home')
            expect(useStore.getState().runsListSession.selectedRunId).toBe('newer')
            expect(window.location.hash).toBe('#/chats')
            expect(savedRoute()).toBe(saved)
        }
        const launches = [start(), start()]
        await waitFor(() => expect(acks).toHaveLength(2))

        // The remote tab closes this workspace while its connection is checked; one start answers meanwhile.
        await user.click(screen.getByTestId('environment-tab-env-win'))
        await screen.findByTestId('remote-environment-gate')
        await act(async () => {
            acks[0].resolve(json({ status: 'started', pipeline_id: 'old-ack' }, 202))
            await expect(launches[0]).resolves.toEqual({ status: 'cancelled' })
        })
        unchanged()

        // The connection fails; back on this machine its workspace opens on the same store, and the other answers.
        await act(async () => checked.resolve())
        await user.click(await screen.findByRole('button', { name: 'Return to this machine' }))
        await screen.findByTestId('app-shell')
        await act(async () => {
            acks[1].resolve(json({ status: 'started', pipeline_id: 'old-ack' }, 202))
            await expect(launches[1]).resolves.toEqual({ status: 'cancelled' })
        })
        unchanged()
        // Both went to this machine, where they began.
        expect(requested.every((url) => url.startsWith('/attractor/pipelines'))).toBe(true)
    })

    it('shows an error screen when remote is unreachable and recovers on Retry', async () => {
        const user = userEvent.setup()
        let remoteOnline = false
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = resolveUrl(input)
            if (url === '/environment') return json(LOCAL_ENV)
            if (url === '/workspace/api/environments/links') return json(REMOTE_LINKS)
            if (url === '/workspace/api/environments/links/env-win/proxy/environment') {
                if (!remoteOnline) {
                    return json({ detail: 'connection refused' }, 503)
                }
                return json({
                    environment_id: 'env-win',
                    display_name: 'win-box',
                    protocol_version: 1,
                    ui_api_version: 1,
                    spark_version: '0.2.0',
                })
            }
            if (url.includes('/workspace/api/definitions?')) return json({ items: [] })
            if (url.includes('/attractor/runs')) return json({ runs: [] })
            if (url === '/workspace/api/projects') return json([LOCAL_PROJECT])
            if (url.includes('/workspace/api/projects')) return json([])
            return json({})
        }))

        render(<App />)
        await user.click(await screen.findByTestId('environment-tab-env-win'))

        const gate = await screen.findByTestId('remote-environment-gate')
        expect(gate).toHaveTextContent('Couldn’t connect to Windows Workstation')
        expect(gate).toHaveTextContent(/connection refused/i)

        remoteOnline = true
        await user.click(screen.getByRole('button', { name: 'Retry' }))

        expect(await screen.findByTestId('app-shell')).toBeVisible()
        expect(screen.getByTestId('remote-environment-indicator')).toBeVisible()
    })
})
