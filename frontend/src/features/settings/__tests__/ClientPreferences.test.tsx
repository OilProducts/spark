import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { applyTheme, cachedAppearance, type Appearance } from '@/lib/theme'
import { deferred, switchEnvironment } from '@/test/environments'
import { ClientPreferencesController } from '../ClientPreferencesController'
import { ClientPreferencesEditor } from '../ClientPreferencesEditor'
import { fetchClientPreferences, parseClientPreferences, saveClientPreferences, type ClientPreferencesView } from '../services/clientPreferences'

vi.mock('../services/clientPreferences', async (original) => ({ ...await original<typeof import('../services/clientPreferences')>(), fetchClientPreferences: vi.fn(), saveClientPreferences: vi.fn() }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

const prefs = (revision: string, appearance: Appearance): ClientPreferencesView => ({ client_id: 'browser-one', revision,
    stored: { editor_mode: null, editor_sidebar_width: null, appearance },
    effective: { editor_mode: 'structured', editor_sidebar_width: 288, appearance } })

type Server = 'local' | 'remote'
// Each environment's preferences server, picked as the requests do; `wait` can hold an answer.
function servePreferences(servers: Record<Server, ClientPreferencesView>, wait: (server: Server, request: 'fetch' | 'save') => Promise<void> | void = () => {}) {
    const serverOf = (environmentId: string | null | undefined): Server =>
        ((environmentId === undefined ? getActiveEnvironmentId() : environmentId) === null ? 'local' : 'remote')
    vi.mocked(fetchClientPreferences).mockImplementation(async (environmentId) => {
        const server = serverOf(environmentId)
        await wait(server, 'fetch')
        return servers[server]
    })
    vi.mocked(saveClientPreferences).mockImplementation(async (base, preferences, environmentId) => {
        const server = serverOf(environmentId)
        const value: ClientPreferencesView = servers[server] = { ...base, revision: `${base.revision}+`, stored: preferences,
            effective: { ...base.effective, ...preferences } as ClientPreferencesView['effective'] }
        await wait(server, 'save')
        return value
    })
}

it('reads an unknown stored editor mode, such as the retired raw mode, as structured', () => {
    const parse = (mode: unknown) => parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: mode, editor_sidebar_width: null }, effective: { editor_mode: mode, editor_sidebar_width: 288 } } }, '/workspace/api/settings')
    expect(parse('raw')).toMatchObject({ stored: { editor_mode: null }, effective: { editor_mode: 'structured' } })
    expect(parse('files')).toMatchObject({ stored: { editor_mode: 'files' }, effective: { editor_mode: 'files' } })
})

it('retains the draft and original revision after live conflicts, and reloads on Discard', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: 'structured', editor_sidebar_width: 288 },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const mode = await screen.findByLabelText('Editor mode')
    await user.selectOptions(mode, 'files')
    const other = { ...view, revision: 'other', stored: { ...view.stored, editor_mode: 'structured' as const, show_advanced_controls: true } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(other)
    act(() => window.dispatchEvent(new Event('spark:settings-live-event')))
    await screen.findByText('Settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
    vi.mocked(saveClientPreferences).mockRejectedValue(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    expect(mode).toHaveValue('files')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_mode: 'files' }, null)
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(mode).toHaveValue('structured')
    expect(screen.getByLabelText('Show advanced controls')).toBeChecked()
})

it('Reset layout clears dragged sizes in the draft, and Save applies it', async () => {
    const view: ClientPreferencesView = { client_id: 'browser-one', revision: 'first',
        stored: { editor_mode: null, editor_sidebar_width: 400, home_sidebar_primary_split_ratio: 0.4, run_presentation: { sort: 'oldest', graph_height: 700 } },
        effective: { editor_mode: 'structured', editor_sidebar_width: 400, home_sidebar_primary_split_ratio: 0.4, run_presentation: { sort: 'oldest', graph_height: 700 } } }
    vi.mocked(fetchClientPreferences).mockResolvedValue(view)
    vi.mocked(saveClientPreferences).mockImplementation(async (_, preferences) => ({ ...view, revision: 'second', stored: preferences }))
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const reset = await screen.findByRole('button', { name: 'Reset layout' })
    expect(reset).toBeEnabled()
    await user.click(reset)
    expect(reset).toBeDisabled()
    expect(saveClientPreferences).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenCalledWith(view, { ...view.stored, editor_sidebar_width: null, home_sidebar_primary_split_ratio: null, run_presentation: { sort: 'oldest', graph_height: null } }, null)
})

it('defaults Show thinking on and toggles it as a switch in the settings editor', async () => {
    const view = parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null, show_thinking: false },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288, show_thinking: false } } }, '/workspace/api/settings')
    expect(view.stored.show_thinking).toBe(false)
    expect(() => parseClientPreferences({ preferences: { scope: 'client', client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null, show_thinking: 'yes' },
        effective: { editor_mode: 'structured', editor_sidebar_width: 288 } } }, '/workspace/api/settings')).toThrow('Invalid show_thinking')
    vi.mocked(fetchClientPreferences).mockResolvedValue({ client_id: 'c', revision: 'r',
        stored: { editor_mode: null, editor_sidebar_width: null }, effective: { editor_mode: 'structured', editor_sidebar_width: 288, show_thinking: true } })
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const toggle = await screen.findByRole('switch', { name: 'Show thinking' })
    expect(toggle).toBeChecked()
    await userEvent.setup().click(toggle)
    expect(toggle).not.toBeChecked()
})

it('changes this browser\'s theme from another environment\'s Settings, and keeps it across switches', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'dark') }
    servePreferences(servers)
    applyTheme('light')
    await switchEnvironment('env-r')
    const settings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const theme = await screen.findByLabelText('Theme')
    // The theme in use shows, not the one the remote environment keeps.
    await waitFor(() => expect(theme).toHaveValue('light'))

    await user.selectOptions(theme, 'dark')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(document.documentElement).toHaveClass('dark')
    expect(servers.local.stored.appearance).toBe('dark')

    // Back on this machine, its preferences keep the theme chosen remotely.
    settings.unmount()
    await switchEnvironment(null)
    render(<ClientPreferencesController />)
    await waitFor(() => expect(fetchClientPreferences).toHaveBeenLastCalledWith(null))
    await act(async () => {})
    expect(cachedAppearance()).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
})

it('saves another preference from another environment\'s Settings, leaving both themes as they were', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'dark') }
    servePreferences(servers)
    applyTheme('light')
    await switchEnvironment('env-r')
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await user.click(screen.getByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')

    expect(servers.remote.stored).toMatchObject({ appearance: 'dark', show_advanced_controls: true })
    expect(servers.local.revision).toBe('local-1')
    expect(cachedAppearance()).toBe('light')
})

it.each([
    ['its environment\'s answer', 'remote', 'save'],
    ['this machine\'s answer', 'local', 'fetch'],
] as const)('keeps the theme chosen last when %s to a theme chosen before it comes late', async (_, heldServer, heldRequest) => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'light') }
    const held = deferred<void>()
    let holding = true
    let reached = false
    servePreferences(servers, (server, request) => {
        if (!holding || server !== heldServer || request !== heldRequest) return
        reached = true
        return held.promise
    })
    applyTheme('light')
    await switchEnvironment('env-r')
    const remoteSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await user.selectOptions(screen.getByLabelText('Theme'), 'dark')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(reached).toBe(true))
    remoteSettings.unmount()
    holding = false

    // Back here, System is chosen after.
    await switchEnvironment(null)
    const localSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue(servers.local.stored.appearance))
    await user.selectOptions(screen.getByLabelText('Theme'), 'system')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    localSettings.unmount()
    render(<ClientPreferencesController />)
    await waitFor(() => expect(cachedAppearance()).toBe('system'))

    await act(async () => held.resolve())
    await act(async () => {})
    expect(servers.local.stored.appearance).toBe('system')
    expect(cachedAppearance()).toBe('system')
    expect(document.documentElement).not.toHaveClass('dark')
})
