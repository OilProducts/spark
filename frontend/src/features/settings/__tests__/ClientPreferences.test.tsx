import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { applyTheme, cachedAppearance, type Appearance } from '@/lib/theme'
import { deferred, switchEnvironment } from '@/test/environments'
import { requestNavigation } from '@/state/workspaceSlice'
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

it('saves a theme with another preference to this machine, applying it only once the save succeeds, and Discard restores both', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'system') }
    servePreferences(servers)
    applyTheme('light')
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const theme = await screen.findByLabelText('Theme')
    await waitFor(() => expect(theme).toHaveValue('light'))
    await user.selectOptions(theme, 'dark')
    await user.click(screen.getByLabelText('Show advanced controls'))

    // A refused save changes neither the theme in use nor the draft.
    vi.mocked(saveClientPreferences).mockRejectedValueOnce(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    expect(screen.queryByText('Saved.')).not.toBeInTheDocument()
    expect(document.documentElement).not.toHaveClass('dark')
    expect(cachedAppearance()).toBe('light')
    expect(theme).toHaveValue('dark')
    expect(screen.getByLabelText('Show advanced controls')).toBeChecked()
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(theme).toHaveValue('light'))
    expect(screen.getByLabelText('Show advanced controls')).not.toBeChecked()

    await user.selectOptions(theme, 'dark')
    await user.click(screen.getByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(prefs('local-1', 'light'), expect.objectContaining({ appearance: 'dark', show_advanced_controls: true }), null)
    expect(servers.local.stored).toMatchObject({ appearance: 'dark', show_advanced_controls: true })
    expect(servers.remote.revision).toBe('remote-1')
    expect(document.documentElement).toHaveClass('dark')
    expect(theme).toHaveValue('dark')
})

it('applies a theme saved in another environment\'s Settings, saving it only there, and keeps it across switches', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'system') }
    servePreferences(servers)
    applyTheme('light')
    await switchEnvironment('env-r')
    const settings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const theme = await screen.findByLabelText('Theme')
    // Settings shows the theme this browser uses, not the one the remote environment stored.
    await waitFor(() => expect(theme).toHaveValue('light'))

    await user.selectOptions(theme, 'dark')
    await user.click(screen.getByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(document.documentElement).toHaveClass('dark')
    expect(servers.remote.stored).toMatchObject({ appearance: 'dark', show_advanced_controls: true })
    expect(fetchClientPreferences).not.toHaveBeenCalledWith(null)
    expect(servers.local.revision).toBe('local-1')

    // Back on this machine, the theme chosen remotely stays, and its Settings shows it.
    settings.unmount()
    await switchEnvironment(null)
    render(<DialogProvider><ClientPreferencesController /><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(fetchClientPreferences).toHaveBeenCalledWith(null))
    await act(async () => {})
    expect(cachedAppearance()).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('dark'))
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
})

it('saves a theme another environment already stores when this browser uses another', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'dark') }
    servePreferences(servers)
    applyTheme('light')
    await switchEnvironment('env-r')
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    const theme = await screen.findByLabelText('Theme')
    await waitFor(() => expect(theme).toHaveValue('light'))
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()

    await user.selectOptions(theme, 'dark')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeEnabled()
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(prefs('remote-1', 'dark'), expect.objectContaining({ appearance: 'dark' }), 'env-r')
    expect(cachedAppearance()).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
    expect(theme).toHaveValue('dark')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
})

it('saves another preference from another environment\'s Settings, showing this browser\'s theme and leaving both themes as they were', async () => {
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
    expect(screen.getByLabelText('Theme')).toHaveValue('light')
})

it('keeps the theme chosen last when the save of a theme chosen before it answers late', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'light') }
    const held = deferred<void>()
    let reached = false
    servePreferences(servers, (server, request) => {
        if (server !== 'remote' || request !== 'save') return
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

    // Back here, System is chosen after.
    await switchEnvironment(null)
    const localSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await user.selectOptions(screen.getByLabelText('Theme'), 'system')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(cachedAppearance()).toBe('system')
    localSettings.unmount()
    render(<ClientPreferencesController />)
    await act(async () => {})

    await act(async () => held.resolve())
    await act(async () => {})
    expect(cachedAppearance()).toBe('system')
    expect(document.documentElement).not.toHaveClass('dark')
    // Each environment keeps the theme saved in its own Settings.
    expect(servers.local.stored.appearance).toBe('system')
    expect(servers.remote.stored.appearance).toBe('dark')
})

it('follows the browser theme in an open Settings when a theme saved elsewhere applies, and saves another preference without changing it', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'light') }
    const held = deferred<void>()
    servePreferences(servers, (server, request) => server === 'remote' && request === 'save' ? held.promise : undefined)
    applyTheme('light')
    await switchEnvironment('env-r')
    const remoteSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await user.selectOptions(screen.getByLabelText('Theme'), 'dark')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledOnce())
    remoteSettings.unmount()

    await switchEnvironment(null)
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const theme = await screen.findByLabelText('Theme')
    await waitFor(() => expect(theme).toHaveValue('light'))
    await act(async () => held.resolve())
    // The remote save applied Dark; the open local Settings shows it, with nothing to save.
    expect(cachedAppearance()).toBe('dark')
    expect(theme).toHaveValue('dark')
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()

    await user.selectOptions(screen.getByLabelText('Editor mode'), 'files')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(prefs('local-1', 'light'), expect.objectContaining({ editor_mode: 'files', appearance: 'light' }), null)
    expect(cachedAppearance()).toBe('dark')
    expect(document.documentElement).toHaveClass('dark')
    expect(theme).toHaveValue('dark')
})

it('keeps a remote save refused after a switch for that environment\'s Settings, with its error, to retry against the same revision', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'light') }
    servePreferences(servers)
    const refused = deferred<ClientPreferencesView>()
    vi.mocked(saveClientPreferences).mockImplementationOnce(() => refused.promise)
    applyTheme('light')
    await switchEnvironment('env-r')
    const remoteSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await user.selectOptions(screen.getByLabelText('Theme'), 'dark')
    await user.click(screen.getByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await waitFor(() => expect(saveClientPreferences).toHaveBeenCalledOnce())
    remoteSettings.unmount()

    // Refused while this machine's Settings shows: nothing of it here.
    await switchEnvironment(null)
    const localSettings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Theme')).toHaveValue('light'))
    await act(async () => refused.reject(new Error('Conflict: reload before saving.')))
    expect(screen.queryByText('Conflict: reload before saving.')).not.toBeInTheDocument()
    expect(screen.getByLabelText('Show advanced controls')).not.toBeChecked()
    expect(cachedAppearance()).toBe('light')
    localSettings.unmount()

    // Back there, Settings opens with the refused change and why, and Save retries it against the revision it was based on.
    await switchEnvironment('env-r')
    const reopened = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    expect(await screen.findByText('Conflict: reload before saving.')).toBeInTheDocument()
    expect(screen.getByLabelText('Show advanced controls')).toBeChecked()
    expect(screen.getByLabelText('Theme')).toHaveValue('dark')
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Saved.')
    expect(saveClientPreferences).toHaveBeenLastCalledWith(prefs('remote-1', 'light'), expect.objectContaining({ appearance: 'dark', show_advanced_controls: true }), 'env-r')
    expect(document.documentElement).toHaveClass('dark')
    expect(servers.local.revision).toBe('local-1')

    // Saved, it is not offered again.
    reopened.unmount()
    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Show advanced controls')).toBeChecked())
    expect(screen.queryByText('Conflict: reload before saving.')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^Save/ })).toBeDisabled()
})

it('drops a refused save once Discard reloads the settings, or leaving them discards it', async () => {
    const servers = { local: prefs('local-1', 'light'), remote: prefs('remote-1', 'light') }
    servePreferences(servers)
    vi.mocked(saveClientPreferences).mockRejectedValueOnce(new Error('Conflict: reload before saving.'))
    applyTheme('light')
    const settings = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    const user = userEvent.setup()
    await user.click(await screen.findByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    settings.unmount()

    const reopened = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await screen.findByText('Conflict: reload before saving.')
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    await waitFor(() => expect(screen.getByLabelText('Show advanced controls')).not.toBeChecked())
    reopened.unmount()

    const again = render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Show advanced controls')).toBeEnabled())
    expect(screen.queryByText('Conflict: reload before saving.')).not.toBeInTheDocument()
    expect(screen.getByLabelText('Show advanced controls')).not.toBeChecked()

    vi.mocked(saveClientPreferences).mockRejectedValueOnce(new Error('Conflict: reload before saving.'))
    await user.click(screen.getByLabelText('Show advanced controls'))
    await user.click(screen.getByRole('button', { name: /^Save/ }))
    await screen.findByText('Conflict: reload before saving.')
    const leave = vi.fn()
    act(() => requestNavigation(leave))
    await user.click(await screen.findByRole('button', { name: 'Discard and leave' }))
    expect(leave).toHaveBeenCalledOnce()
    again.unmount()

    render(<DialogProvider><ClientPreferencesEditor /></DialogProvider>)
    await waitFor(() => expect(screen.getByLabelText('Show advanced controls')).toBeEnabled())
    expect(screen.queryByText('Conflict: reload before saving.')).not.toBeInTheDocument()
    expect(screen.getByLabelText('Show advanced controls')).not.toBeChecked()
})
